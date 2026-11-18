//! "Is there a newer version?" against the latest GitHub release. Runs `curl`
//! (built into macOS, Windows 10+ and most Linux installs) on a background
//! thread, so the app needs no HTTP or TLS library. The request carries nothing
//! about the user beyond what any HTTP request does (IP address, curl's agent).

use std::process::Command;
use std::sync::mpsc;

use eframe::egui;

/// Latest release of the app, as JSON (`tag_name`, `html_url`).
const LATEST_RELEASE: &str = "https://api.github.com/repos/binho/strikebar/releases/latest";

/// Automatic checks happen at most this often.
pub const INTERVAL_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed(String),
}

pub struct Checker {
    pub status: Status,
    rx: Option<mpsc::Receiver<Status>>,
}

impl Checker {
    pub fn new() -> Self {
        Self {
            status: Status::Idle,
            rx: None,
        }
    }

    pub fn start(&mut self, ctx: &egui::Context) {
        if self.status == Status::Checking {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(check(env!("CARGO_PKG_VERSION")));
            ctx.request_repaint();
        });
        self.status = Status::Checking;
        self.rx = Some(rx);
    }

    /// The new status, once a check finishes.
    pub fn poll(&mut self) -> Option<&Status> {
        let status = self.rx.as_ref()?.try_recv().ok()?;
        self.rx = None;
        self.status = status;
        Some(&self.status)
    }
}

fn check(current: &str) -> Status {
    let mut cmd = Command::new("curl");
    cmd.args([
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--max-time",
        "15",
        "--header",
        "Accept: application/vnd.github+json",
        LATEST_RELEASE,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
    }
    let out = match cmd.output() {
        Ok(out) => out,
        Err(e) => return Status::Failed(format!("couldn't run curl: {e}")),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().trim_start_matches("curl: ").trim();
        return Status::Failed(if err.is_empty() {
            "no answer from GitHub".into()
        } else {
            err.to_owned()
        });
    }
    let body = String::from_utf8_lossy(&out.stdout);
    match (
        json_string(&body, "tag_name"),
        json_string(&body, "html_url"),
    ) {
        (Some(tag), Some(url)) if newer(&tag, current) => Status::Available {
            version: tag.trim_start_matches('v').to_owned(),
            url,
        },
        (Some(_), _) => Status::UpToDate,
        _ => Status::Failed("unexpected answer from GitHub".into()),
    }
}

/// The string value of the first `"key": "…"` in a JSON text. Enough for the
/// two plain fields read here, without a JSON parser.
fn json_string(json: &str, key: &str) -> Option<String> {
    let at = json.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

/// Whether version tag `tag` (`v1.2.0`, `1.10`) is newer than `current`.
fn newer(tag: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .map_while(|p| p.parse().ok())
            .collect()
    };
    let (a, b) = (parts(tag), parts(current));
    let len = a.len().max(b.len());
    let pad = |v: &[u64]| {
        (0..len)
            .map(|i| v.get(i).copied().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    pad(&a) > pad(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("v0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.1", "0.1.0"));
        assert!(!newer("v0.0.9", "0.1.0"));
    }

    #[test]
    fn reads_the_release_fields() {
        let json = r#"{"url": "x", "html_url" : "https://github.com/o/r/releases/tag/v0.2.0",
            "tag_name":"v0.2.0", "name": "0.2"}"#;
        assert_eq!(json_string(json, "tag_name").as_deref(), Some("v0.2.0"));
        assert_eq!(
            json_string(json, "html_url").as_deref(),
            Some("https://github.com/o/r/releases/tag/v0.2.0")
        );
        assert_eq!(json_string(json, "missing"), None);
    }
}
