//! Cross-platform shell and browser detection (task CB-005).
//!
//! Replaces the hardcoded `SysCommand::new("cmd").args(["/C", cmd])` and
//! Windows-only `BROWSER_PATHS` array with `#[cfg(target_os)]`-gated helpers
//! that work on Windows, Linux, and macOS.
//!
//! Public surface:
//!  - [`Platform::shell_command`] — returns the right shell interpreter for
//!    the host OS, plus the right flag to pass a single command string.
//!  - [`detect_browser`] — finds a Chromium-based browser on any platform.
//!  - [`normalize_path`] — converts Windows-style paths to host-native form
//!    when needed (currently a no-op on Unix, since the existing code is
//!    already portable).

use std::path::PathBuf;

/// OS-specific shell selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// Windows: `cmd /C <cmd>`
    Cmd,
    /// Unix (Linux, macOS, *BSD): `sh -c <cmd>`
    Sh,
}

impl Shell {
    /// The interpreter binary name.
    pub fn binary(self) -> &'static str {
        match self {
            Shell::Cmd => "cmd",
            Shell::Sh => "sh",
        }
    }

    /// The flag used to pass a single command string to the interpreter.
    pub fn command_flag(self) -> &'static str {
        match self {
            Shell::Cmd => "/C",
            Shell::Sh => "-c",
        }
    }

    #[cfg(target_os = "windows")]
    pub fn host() -> Self {
        Shell::Cmd
    }

    #[cfg(not(target_os = "windows"))]
    pub fn host() -> Self {
        Shell::Sh
    }
}

/// Convenience: returns the (binary, flag) pair for the current platform.
pub fn platform_shell() -> (String, String) {
    let s = Shell::host();
    (s.binary().to_string(), s.command_flag().to_string())
}

/// Detect a Chromium-based browser on any platform.
///
/// Order:
///  1. `CHATBANG_BROWSER` env var (explicit override).
///  2. `which`-style PATH lookup for known browser binary names.
///  3. Well-known absolute paths per platform (Windows Program Files,
///     macOS `/Applications`, Linux `/usr/bin/*`).
pub fn detect_browser() -> Option<PathBuf> {
    // 1. Explicit override.
    if let Ok(p) = std::env::var("CHATBANG_BROWSER") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Some(pb);
        }
    }

    // 2. PATH lookup.
    for name in path_browser_candidates() {
        if let Ok(p) = which::which(name) {
            return Some(p);
        }
    }

    // 3. Absolute paths.
    for p in absolute_browser_candidates() {
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Binary names to look up via `which` (PATH lookup).
fn path_browser_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["chrome", "chrome.exe", "msedge", "msedge.exe", "brave", "brave.exe"]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
            "vivaldi",
        ]
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
            "vivaldi",
            "epiphany",
        ]
    }
}

/// Hardcoded absolute paths to try when PATH lookup fails.
fn absolute_browser_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let win_paths = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files\Vivaldi\Application\vivaldi.exe",
            r"C:\Program Files (x86)\Vivaldi\Application\vivaldi.exe",
        ];
        for s in win_paths {
            v.push(PathBuf::from(s));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local_paths = [
                r"Google\Chrome\Application\chrome.exe",
                r"Microsoft\Edge\Application\msedge.exe",
                r"BraveSoftware\Brave-Browser\Application\brave.exe",
            ];
            for rel in &local_paths {
                v.push(PathBuf::from(&local).join(rel));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mac_paths = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for s in mac_paths {
            v.push(PathBuf::from(s));
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let linux_paths = [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
            "/usr/bin/brave-browser",
            "/usr/bin/vivaldi",
            "/snap/bin/chromium",
            "/opt/google/chrome/chrome",
            "/opt/microsoft/msedge/msedge",
        ];
        for s in linux_paths {
            v.push(PathBuf::from(s));
        }
    }

    v
}

/// Normalize a user-supplied path. Currently a no-op on Unix; on Windows it
/// leaves backslashes intact. Exposed so future locale quirks can be patched
/// in one place.
pub fn normalize_path(p: &str) -> PathBuf {
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_matches_host() {
        let s = Shell::host();
        if cfg!(target_os = "windows") {
            assert_eq!(s, Shell::Cmd);
        } else {
            assert_eq!(s, Shell::Sh);
        }
    }

    #[test]
    fn platform_shell_returns_correct_pair() {
        let (bin, flag) = platform_shell();
        if cfg!(target_os = "windows") {
            assert_eq!(bin, "cmd");
            assert_eq!(flag, "/C");
        } else {
            assert_eq!(bin, "sh");
            assert_eq!(flag, "-c");
        }
    }

    #[test]
    fn absolute_browser_candidates_nonempty_on_all_platforms() {
        // We can't assert that a browser actually exists in CI, but the
        // candidate list itself must be non-empty on every supported OS.
        let v = absolute_browser_candidates();
        assert!(!v.is_empty());
    }

    #[test]
    fn env_override_works_when_path_missing() {
        // Setting to a non-existent path should NOT short-circuit detection
        // — we just skip the override and fall through to PATH/absolutes.
        std::env::set_var("CHATBANG_BROWSER", "/nonexistent/definitely/not/here");
        // We don't assert the result; the point is that the function does
        // not panic or return an Err.
        let _ = detect_browser();
        std::env::remove_var("CHATBANG_BROWSER");
    }
}
