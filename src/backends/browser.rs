//! Legacy ChatGPT-web-scraping backend.
//!
//! This is the original chatbang backend: launch a real Chromium browser,
//! navigate to https://chatgpt.com, type the prompt into the textarea, click
//! submit, then scrape the response via the clipboard.
//!
//! It's preserved for backward compatibility and for users who don't have
//! any API key. New users should prefer the Ollama / Groq / Gemini / Together
//! backends — they're faster, don't require a GUI, and don't depend on the
//! ChatGPT web UI remaining stable.
//!
//! Gated behind the `browser` feature (default on, but off for headless CI).

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chromiumoxide::{Browser, Page};
use futures::StreamExt;
use std::path::Path;
use tokio::time::{sleep, Duration};

use super::{Backend, Message, Role};

const JS_READ_CLIPBOARD: &str = r#"
    new Promise((resolve) => {
        window.navigator.clipboard.readText()
            .then(t => resolve(t))
            .catch(() => resolve(""));
    })
"#;

fn js_click_last_copy_button() -> &'static str {
    r#"(() => {
        const buttons = document.querySelectorAll('button[data-testid="copy-turn-action-button"]');
        if (buttons.length > 0) { buttons[buttons.length - 1].click(); return true; }
        return false;
    })()"#
}

/// chatbang-side configuration for the browser backend. NOT to be confused
/// with `chromiumoxide::BrowserConfig`, which is the lower-level CDP config.
pub struct ChatbangBrowserConfig {
    pub browser_exe: String,
    pub profile_dir: std::path::PathBuf,
    pub headless: bool,
}

pub struct BrowserBackend {
    cfg: ChatbangBrowserConfig,
    // The browser is launched lazily on the first chat() call — that way
    // `BrowserBackend::new()` is cheap and side-effect-free, and tests that
    // just construct one don't try to spawn a real browser.
    page: tokio::sync::OnceCell<Page>,
    _browser: tokio::sync::OnceCell<Browser>,
    _handler: tokio::sync::OnceCell<tokio::task::JoinHandle<()>>,
}

impl BrowserBackend {
    pub fn new(cfg: ChatbangBrowserConfig) -> Self {
        Self {
            cfg,
            page: tokio::sync::OnceCell::new(),
            _browser: tokio::sync::OnceCell::new(),
            _handler: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_page(&self) -> Result<&Page> {
        self.page
            .get_or_try_init(|| async {
                let bc = chromiumoxide::BrowserConfig::builder()
                    .chrome_executable(&self.cfg.browser_exe)
                    .user_data_dir(&self.cfg.profile_dir)
                    .arg("--disable-blink-features=AutomationControlled")
                    .arg("--exclude-switches=enable-automation")
                    .arg("--disable-extensions=false")
                    .arg("--profile-directory=Default")
                    .arg("--no-sandbox")
                    .arg(
                        "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                         AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                    )
                    .with_head()
                    .build()
                    .map_err(|e| anyhow!("BrowserConfig error: {}", e))?;

                let (browser, mut handler) = Browser::launch(bc).await?;
                let h = tokio::spawn(async move {
                    while let Some(_e) = handler.next().await {}
                });
                let page = browser.new_page("https://chatgpt.com").await?;
                sleep(Duration::from_secs(3)).await;

                // Stash browser + handler so they live as long as self.
                let _ = self._browser.set(browser);
                let _ = self._handler.set(h);
                Ok(page)
            })
            .await
    }

    /// Type `text` into the prompt textarea and submit, then wait for and
    /// return the full clipboard text of ChatGPT's response.
    async fn submit_and_wait(&self, page: &Page, text: &str) -> Result<String> {
        let textarea = page.find_element("#prompt-textarea").await?;
        textarea.click().await?;
        textarea.type_str(text).await?;
        page.find_element("#composer-submit-button")
            .await?
            .click()
            .await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(anyhow!("Timed out waiting for ChatGPT response."));
            }
            sleep(Duration::from_millis(1_500)).await;

            let clicked: bool = page
                .evaluate(js_click_last_copy_button())
                .await?
                .into_value()
                .unwrap_or(false);

            if !clicked {
                continue;
            }

            sleep(Duration::from_millis(400)).await;

            let clipboard: String = page
                .evaluate(JS_READ_CLIPBOARD)
                .await?
                .into_value()
                .unwrap_or_default();

            if !clipboard.is_empty() && clipboard != text {
                return Ok(clipboard);
            }
        }
    }

    /// Open a browser session so the user can log in to ChatGPT and grant
    /// clipboard permission. Run this once before the first chat.
    pub async fn login_session(&self) -> Result<()> {
        println!("\nOpening ChatGPT in your browser.");
        println!("1. Log in with your OpenAI account.");
        println!("2. When ChatGPT asks about clipboard access, click Allow.");
        println!("3. Come back here and press Enter.\n");

        // We launch a one-off browser here — we do NOT reuse self.page,
        // because that would call ensure_page() which navigates to chatgpt.com
        // and waits 3 seconds. The login flow needs interactive timing.
        let cfg = chromiumoxide::BrowserConfig::builder()
            .chrome_executable(&self.cfg.browser_exe)
            .user_data_dir(&self.cfg.profile_dir)
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--no-sandbox")
            .arg("--profile-directory=Default")
            .with_head()
            .build()
            .map_err(|e| anyhow!("BrowserConfig error: {}", e))?;

        let (browser, mut handler) = Browser::launch(cfg).await?;
        let _h = tokio::spawn(async move { while handler.next().await.is_some() {} });

        browser.new_page("https://chatgpt.com").await?;

        use std::io::{self, BufRead, Write};
        print!("Press Enter when you have finished logging in... ");
        io::stdout().flush()?;
        io::stdin().lock().lines().next();

        println!("\nSession saved. Run `chatbang` to start chatting.");
        Ok(())
    }
}

#[async_trait]
impl Backend for BrowserBackend {
    fn name(&self) -> &str {
        "browser"
    }

    async fn chat(&self, messages: &[Message]) -> Result<String> {
        let page = self.ensure_page().await?;

        // Flatten the conversation into a single text prompt. ChatGPT web UI
        // doesn't expose role-based turn-taking to us, so we concatenate.
        // The system prompt is prepended to the first user turn.
        let mut text = String::new();
        for m in messages {
            match m.role {
                Role::System => {
                    text.push_str("[SYSTEM]\n");
                    text.push_str(&m.content);
                    text.push_str("\n[/SYSTEM]\n\n");
                }
                Role::User => {
                    text.push_str(&m.content);
                    text.push_str("\n\n");
                }
                Role::Assistant => {
                    text.push_str("[assistant] ");
                    text.push_str(&m.content);
                    text.push_str("\n\n");
                }
                Role::Tool => {
                    text.push_str(&format!("<tool_result>\n{}\n</tool_result>\n\n", m.content));
                }
            }
        }
        let text = text.trim().to_string();

        self.submit_and_wait(page, &text)
            .await
            .context("browser backend chat failed")
    }
}

/// Build a `BrowserBackend` from a browser-exe path and a profile dir.
pub fn from_paths(browser_exe: impl Into<String>, profile_dir: impl AsRef<Path>) -> BrowserBackend {
    BrowserBackend::new(ChatbangBrowserConfig {
        browser_exe: browser_exe.into(),
        profile_dir: profile_dir.as_ref().to_path_buf(),
        headless: false,
    })
}
