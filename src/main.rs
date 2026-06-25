use anyhow::{anyhow, Context, Result};
use chromiumoxide::{Browser, BrowserConfig, Page};
use clap::{Parser, Subcommand};
use futures::StreamExt;
use serde::Deserialize;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;
use termimad::MadSkin;
use tokio::time::{sleep, Duration};

// ─── Windows browser candidates (preference order) ────────────────────────────

const BROWSER_PATHS: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
    r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
    r"C:\Program Files\Vivaldi\Application\vivaldi.exe",
    r"C:\Program Files (x86)\Vivaldi\Application\vivaldi.exe",
];

// ─── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "chatbang",
    about = "Access ChatGPT from your terminal — no API key required.",
    long_about = None
)]
struct Cli {
    /// Disable the agentic tool-use loop (raw chat only).
    #[arg(long)]
    no_agent: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a browser session to log in to ChatGPT and grant clipboard permission.
    /// Run this once before your first chat.
    Config,
}

// ─── Config helpers ───────────────────────────────────────────────────────────

fn chatbang_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chatbang")
}

fn detect_browser() -> Option<PathBuf> {
    for s in BROWSER_PATHS {
        let p = PathBuf::from(s);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let candidates = [
            r"Google\Chrome\Application\chrome.exe",
            r"Microsoft\Edge\Application\msedge.exe",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ];
        for rel in &candidates {
            let p = PathBuf::from(&local).join(rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn load_config() -> Result<(PathBuf, String)> {
    let dir = chatbang_config_dir();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create config dir: {}", dir.display()))?;

    let config_path = dir.join("chatbang.ini");
    let profile_dir = dir.join("profile_data");

    let saved_browser = if config_path.exists() {
        let raw = fs::read_to_string(&config_path)
            .with_context(|| "Cannot read config file")?;
        raw.lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .find_map(|line| {
                let mut parts = line.splitn(2, '=');
                let key = parts.next()?.trim();
                let val = parts.next()?.trim();
                (key == "browser" && !val.is_empty()).then(|| val.to_string())
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let browser = if !saved_browser.is_empty() && Path::new(&saved_browser).exists() {
        saved_browser
    } else {
        let found = detect_browser().ok_or_else(|| {
            anyhow!(
                "No Chromium-based browser found.\n\
                 Install Chrome, Edge, or Brave, then optionally set\n\
                 browser=<path>  in:  {}",
                config_path.display()
            )
        })?;
        let path_str = found.to_string_lossy().to_string();
        fs::write(&config_path, format!("browser={}\n", path_str))
            .with_context(|| "Cannot write config file")?;
        println!("Auto-detected browser: {}", path_str);
        path_str
    };

    Ok((profile_dir, browser))
}

// ─── Browser helpers ──────────────────────────────────────────────────────────

fn make_browser_config(browser_exe: &str, profile_dir: &Path) -> Result<BrowserConfig> {
    BrowserConfig::builder()
        .chrome_executable(browser_exe)
        .user_data_dir(profile_dir)
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
        .map_err(|e| anyhow!("BrowserConfig error: {}", e))
}

// ─── Login / config session ───────────────────────────────────────────────────

async fn login_session(browser_exe: &str, profile_dir: &Path) -> Result<()> {
    println!("\nOpening ChatGPT in your browser.");
    println!("1. Log in with your OpenAI account.");
    println!("2. When ChatGPT asks about clipboard access, click Allow.");
    println!("3. Come back here and press Enter.\n");

    let config = make_browser_config(browser_exe, profile_dir)?;
    let (browser, mut handler) = Browser::launch(config).await?;
    let _h = tokio::spawn(async move { while handler.next().await.is_some() {} });

    browser.new_page("https://chatgpt.com").await?;

    print!("Press Enter when you have finished logging in... ");
    io::stdout().flush()?;
    io::stdin().lock().lines().next();

    println!("\nSession saved. Run `chatbang` to start chatting.");
    Ok(())
}

// ─── CDP helpers ──────────────────────────────────────────────────────────────

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

/// Type `text` into the prompt textarea and submit, then wait for and return
/// the full clipboard text of ChatGPT's response.
async fn submit_and_wait(page: &Page, text: &str) -> Result<String> {
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

        // The clipboard starts out containing whatever was there before (or the
        // prompt itself).  Accept it only when it's non-empty and different from
        // what we just sent.
        if !clipboard.is_empty() && clipboard != text {
            return Ok(clipboard);
        }
    }
}

// ─── Tool calling ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ToolCall {
    name: String,
    args: serde_json::Value,
}

/// Parse all `<tool_call>...</tool_call>` blocks from a model response.
fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
    let open_tag = "<tool_call>";
    let close_tag = "</tool_call>";
    let mut calls = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find(open_tag) {
        let after_open = &remaining[start + open_tag.len()..];
        match after_open.find(close_tag) {
            Some(end) => {
                let json_str = after_open[..end].trim();
                if let Ok(call) = serde_json::from_str::<ToolCall>(json_str) {
                    calls.push(call);
                } else {
                    eprintln!("[agent] Warning: could not parse tool call JSON: {}", json_str);
                }
                remaining = &after_open[end + close_tag.len()..];
            }
            None => break,
        }
    }
    calls
}

fn arg_str<'a>(args: &'a serde_json::Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Execute a single tool call and return its output as a string.
fn execute_tool(call: &ToolCall) -> String {
    match call.name.as_str() {
        // ── run_command ──────────────────────────────────────────────────────
        "run_command" => {
            let cmd = arg_str(&call.args, "command");
            if cmd.is_empty() {
                return "[error] run_command: missing 'command' argument".to_string();
            }
            match SysCommand::new("cmd").args(["/C", cmd]).output() {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let mut result = stdout.trim().to_string();
                    if !stderr.trim().is_empty() {
                        result.push_str("\n[stderr] ");
                        result.push_str(stderr.trim());
                    }
                    if result.is_empty() {
                        "(no output)".to_string()
                    } else {
                        result
                    }
                }
                Err(e) => format!("[error] run_command failed: {}", e),
            }
        }

        // ── read_file ────────────────────────────────────────────────────────
        "read_file" => {
            let path = arg_str(&call.args, "path");
            if path.is_empty() {
                return "[error] read_file: missing 'path' argument".to_string();
            }
            fs::read_to_string(path).unwrap_or_else(|e| format!("[error] read_file: {}", e))
        }

        // ── write_file ───────────────────────────────────────────────────────
        "write_file" => {
            let path = arg_str(&call.args, "path");
            let content = arg_str(&call.args, "content");
            if path.is_empty() {
                return "[error] write_file: missing 'path' argument".to_string();
            }
            fs::write(path, content)
                .map(|_| format!("Written {} bytes to {}", content.len(), path))
                .unwrap_or_else(|e| format!("[error] write_file: {}", e))
        }

        // ── list_dir ─────────────────────────────────────────────────────────
        "list_dir" => {
            let path = arg_str(&call.args, "path");
            let dir = if path.is_empty() { "." } else { path };
            fs::read_dir(dir)
                .map(|entries| {
                    let mut lines: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            let kind = if e.path().is_dir() { "[dir] " } else { "      " };
                            format!("{}{}", kind, name)
                        })
                        .collect();
                    lines.sort();
                    if lines.is_empty() {
                        "(empty directory)".to_string()
                    } else {
                        lines.join("\n")
                    }
                })
                .unwrap_or_else(|e| format!("[error] list_dir: {}", e))
        }

        other => format!("[error] unknown tool: '{}'", other),
    }
}

/// The system-prompt injected as a prefix on the very first user message.
fn agent_system_prompt() -> &'static str {
    r#"[SYSTEM — read carefully before responding]
You are a helpful assistant with access to tools on the user's Windows machine.

When you need to use a tool, emit one or more tool calls using this exact format
(one per line, nothing else between the tags):

<tool_call>{"name": "TOOL_NAME", "args": {"KEY": "VALUE"}}</tool_call>

Available tools:
  run_command  — run a shell command via cmd.exe.  args: {"command": "string"}
  read_file    — read a file from disk.            args: {"path": "string"}
  write_file   — write content to a file.          args: {"path": "string", "content": "string"}
  list_dir     — list files in a directory.        args: {"path": "string"}

After emitting tool calls, stop immediately. The results will come back to you
wrapped in <tool_result name="TOOL_NAME">...</tool_result>. Use them to continue
reasoning. When you have all the information you need, give your final answer
with no tool calls at all.

[END SYSTEM]

"#
}

// ─── Chat / agent loop ────────────────────────────────────────────────────────

/// Run the interactive REPL, optionally with the agentic tool-use loop.
async fn run_chat(browser_exe: &str, profile_dir: &Path, agent: bool) -> Result<()> {
    let skin = MadSkin::default();

    let config = make_browser_config(browser_exe, profile_dir)?;
    let (browser, mut handler) = Browser::launch(config).await?;
    let _h = tokio::spawn(async move { while handler.next().await.is_some() {} });
    let page = browser.new_page("https://chatgpt.com").await?;

    sleep(Duration::from_secs(3)).await;

    if agent {
        println!("Agent mode ON  —  tools: run_command, read_file, write_file, list_dir");
        println!("Use --no-agent to disable.\n");
    }

    print!("> ");
    io::stdout().flush()?;

    let mut first_message = true;

    let stdin = io::stdin();
    for line_result in stdin.lock().lines() {
        let line = line_result.context("Failed to read stdin")?;
        let raw = line.trim().to_string();

        if raw.is_empty() {
            print!("> ");
            io::stdout().flush()?;
            continue;
        }

        // Build the initial message, prepending the system prompt on the very
        // first turn of an agent session so ChatGPT knows about the tools.
        let mut message = if agent && first_message {
            format!("{}{}", agent_system_prompt(), raw)
        } else {
            raw.clone()
        };
        // The "5 lines" hint from the original chatbang — skip in agent mode
        // because tool-using responses need more room.
        if !agent {
            message = format!("{} (Make an answer in less than 5 lines.)", message);
        }
        first_message = false;

        println!("[Thinking...]\n");

        // ── Agentic turn loop ──────────────────────────────────────────────
        loop {
            let response = match submit_and_wait(&page, &message).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: {:#}", e);
                    break;
                }
            };

            let calls = if agent { parse_tool_calls(&response) } else { vec![] };

            if calls.is_empty() {
                // No tool calls — this is the final answer.
                skin.print_text(&response);
                println!();
                break;
            }

            // Execute all tool calls and collect results.
            let mut tool_results = String::new();
            for call in &calls {
                eprintln!("[tool] {} {:?}", call.name, call.args);
                let output = execute_tool(call);
                tool_results.push_str(&format!(
                    "<tool_result name=\"{}\">\n{}\n</tool_result>\n",
                    call.name, output
                ));
            }

            // Print a brief progress note so the terminal isn't silent.
            eprintln!("[agent] executed {} tool(s), sending results back…\n", calls.len());

            // The next iteration will send the tool results as the new message.
            message = tool_results;
        }

        print!("> ");
        io::stdout().flush()?;
    }

    Ok(())
}

// ─── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (profile_dir, browser_exe) = load_config()?;

    match cli.command {
        Some(Commands::Config) => {
            login_session(&browser_exe, &profile_dir).await?;
        }
        None => {
            run_chat(&browser_exe, &profile_dir, !cli.no_agent).await?;
        }
    }

    Ok(())
}
