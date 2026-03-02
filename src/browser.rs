/// Cross-platform browser launcher.
///
/// Ported from bcat's lib/bcat/browser.rb by Ryan Tomayko.
use std::io;
use std::process::{Child, Command, Stdio};

// ── Platform detection ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Macos,
    Linux,
    Windows,
    Unknown,
}

fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Unknown
    }
}

// ── Browser command table ─────────────────────────────────────────────────────

/// Resolve a browser name to the argv vec used to launch it.
/// Returns `None` for unknown names (fall back to system default).
fn browser_argv(name: &str) -> Option<Vec<&'static str>> {
    // Normalise aliases first.
    let name = match name {
        "google-chrome" | "google_chrome" => "chrome",
        "chromium-browser" => "chromium",
        other => other,
    };

    match platform() {
        Platform::Macos => Some(match name {
            "default" => vec!["open"],
            "safari" => vec!["open", "-a", "Safari"],
            "firefox" => vec!["open", "-a", "Firefox"],
            "chrome" => vec!["open", "-a", "Google Chrome"],
            "chromium" => vec!["open", "-a", "Chromium"],
            "opera" => vec!["open", "-a", "Opera"],
            "curl" => vec!["curl", "-s"],
            _ => return None,
        }),
        Platform::Linux | Platform::Unknown => Some(match name {
            "default" => vec!["xdg-open"],
            "firefox" => vec!["firefox"],
            "chrome" => vec!["google-chrome"],
            "chromium" => vec!["chromium"],
            "mozilla" => vec!["mozilla"],
            "epiphany" => vec!["epiphany"],
            "curl" => vec!["curl", "-s"],
            _ => return None,
        }),
        Platform::Windows => Some(match name {
            "default" | "chrome" | "firefox" | "edge" => vec!["cmd", "/c", "start", ""],
            "curl" => vec!["curl", "-s"],
            _ => return None,
        }),
    }
}

// ── Browser ───────────────────────────────────────────────────────────────────

/// Launches a browser and returns the child process handle.
pub struct Browser {
    name: String,
}

impl Browser {
    /// Create a browser launcher.
    ///
    /// `name` is one of the named browsers (`"safari"`, `"firefox"`,
    /// `"chrome"`, `"chromium"`, `"opera"`, `"curl"`) or `"default"` to use
    /// the system default. Checked against `BCAT_BROWSER` env var at
    /// construction time by the caller.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Open `url` in the configured browser. Returns the child process.
    pub fn open(&self, url: &str) -> io::Result<Child> {
        let argv = browser_argv(&self.name).unwrap_or_else(|| {
            // Unknown name — treat as a literal command path.
            vec![] // handled below
        });

        if argv.is_empty() {
            // Literal command (not in our table): just exec it with the URL.
            return Command::new(&self.name)
                .arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }

        // Windows `start` needs the URL as a separate arg after the title arg.
        let mut cmd = Command::new(argv[0]);
        for arg in &argv[1..] {
            cmd.arg(arg);
        }
        cmd.arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    /// The resolved command string (for debug logging).
    pub fn command(&self) -> String {
        let argv = browser_argv(&self.name);
        match argv {
            Some(v) => v.join(" "),
            None => self.name.clone(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_browser_has_command() {
        let b = Browser::new("default");
        assert!(!b.command().is_empty());
    }

    #[test]
    fn alias_google_chrome() {
        // The alias should resolve to the same command as "chrome".
        let argv_alias = browser_argv("google-chrome");
        let argv_canon = browser_argv("chrome");
        assert_eq!(argv_alias, argv_canon);
    }

    #[test]
    fn unknown_browser_returns_none_from_table() {
        assert!(browser_argv("netscape").is_none());
    }

    #[test]
    fn curl_browser_known() {
        let argv = browser_argv("curl");
        assert!(argv.is_some());
        let v = argv.unwrap();
        assert_eq!(v[0], "curl");
    }
}
