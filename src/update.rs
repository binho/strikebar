//! Updates from GitHub releases: check for a newer version, then download,
//! verify, install and relaunch.
//!
//! - The check reads the latest release's JSON with `curl` (built into macOS,
//!   Windows 10+ and most Linux installs), so the app has no HTTP or TLS code.
//!   The request carries nothing about the user beyond what any HTTP request
//!   does (IP address, curl's agent).
//! - A release has `Strikebar-<version>-macos-<arch>.zip` plus a minisign
//!   signature of it (`.zip.minisig`), made by `scripts/release.sh` with the key
//!   whose public half is `assets/update-key.pub`. The zip is installed only if
//!   the signature checks out against that key and its trusted comment names
//!   that exact file, so an older signed build can't be passed off as newer.
//! - Installing swaps the app bundle in place (macOS only; elsewhere the release
//!   page opens instead), then a small shell waits for this process to quit and
//!   opens the new bundle.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use eframe::egui;
use serde::Deserialize;

/// The latest release, as GitHub's JSON. Set `STRIKEBAR_UPDATE_FEED` when
/// building to point a test build elsewhere (any URL curl reads, `file://` too).
const FEED: &str = match option_env!("STRIKEBAR_UPDATE_FEED") {
    Some(url) => url,
    None => "https://api.github.com/repos/binho/strikebar/releases/latest",
};

/// Public half of the release signing key (minisign format).
const PUBLIC_KEY: &str = include_str!("../assets/update-key.pub");

/// Automatic checks happen at most this often.
pub const INTERVAL_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing(Release),
    /// The new version is in place at this bundle path; relaunch into it.
    Installed(PathBuf),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// Release page, with the notes.
    pub page: String,
    /// The build for this platform, if the release has one.
    pub download: Option<Download>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub name: String,
    pub zip_url: String,
    pub signature_url: String,
}

pub struct Checker {
    pub status: Status,
    /// The bundle updates install into, or why they can't be installed.
    pub target: Result<PathBuf, String>,
    rx: Option<mpsc::Receiver<Status>>,
}

impl Checker {
    pub fn new() -> Self {
        Self {
            status: Status::Idle,
            target: current_app(),
            rx: None,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Checking | Status::Installing(_))
    }

    pub fn start(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        self.run(ctx, Status::Checking, || check(env!("CARGO_PKG_VERSION")));
    }

    pub fn install(&mut self, ctx: &egui::Context, release: Release) {
        if self.busy() {
            return;
        }
        let target = self.target.clone();
        let job = release.clone();
        self.run(ctx, Status::Installing(release), move || {
            match target.and_then(|app| install(&job, &app).map(|()| app)) {
                Ok(app) => Status::Installed(app),
                Err(e) => Status::Failed(e),
            }
        });
    }

    fn run(
        &mut self,
        ctx: &egui::Context,
        now: Status,
        job: impl FnOnce() -> Status + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.status = now;
        self.rx = Some(rx);
    }

    /// The new status, once a check or an install finishes.
    pub fn poll(&mut self) -> Option<&Status> {
        let status = self.rx.as_ref()?.try_recv().ok()?;
        self.rx = None;
        self.status = status;
        Some(&self.status)
    }
}

/// Name of this platform's build in a release.
fn asset_name(version: &str) -> String {
    format!("Strikebar-{version}-macos-{}.zip", std::env::consts::ARCH)
}

fn check(current: &str) -> Status {
    let json = match curl_text(FEED) {
        Ok(json) => json,
        Err(e) => return Status::Failed(e),
    };
    match parse_release(&json, current) {
        Ok(Some(release)) => Status::Available(release),
        Ok(None) => Status::UpToDate,
        Err(e) => Status::Failed(e),
    }
}

/// The release in GitHub's JSON, if it's newer than `current`.
fn parse_release(json: &str, current: &str) -> Result<Option<Release>, String> {
    #[derive(Deserialize)]
    struct GhRelease {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        assets: Vec<GhAsset>,
    }
    #[derive(Deserialize)]
    struct GhAsset {
        name: String,
        browser_download_url: String,
    }
    let gh: GhRelease =
        serde_json::from_str(json).map_err(|_| "unexpected answer from GitHub".to_owned())?;
    if !newer(&gh.tag_name, current) {
        return Ok(None);
    }
    let version = gh.tag_name.trim_start_matches('v').to_owned();
    let name = asset_name(&version);
    let url = |n: &str| {
        gh.assets
            .iter()
            .find(|a| a.name == n)
            .map(|a| a.browser_download_url.clone())
    };
    let download = match (url(&name), url(&format!("{name}.minisig"))) {
        (Some(zip_url), Some(signature_url)) => Some(Download {
            name,
            zip_url,
            signature_url,
        }),
        _ => None,
    };
    Ok(Some(Release {
        version,
        page: gh.html_url,
        download,
    }))
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

/// The app bundle this process runs from, if updates can be installed into it.
fn current_app() -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("installing updates is only supported on macOS".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);
    // …/Strikebar.app/Contents/MacOS/strikebar
    let app = exe
        .ancestors()
        .nth(3)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("Strikebar isn't running from its app bundle")?;
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Err(
            "macOS runs this copy from a temporary location. Move Strikebar to \
                    Applications, open it from there and update again."
                .into(),
        );
    }
    Ok(app.to_owned())
}

/// Downloads, verifies and unpacks `release`, then puts it in place of `app`.
fn install(release: &Release, app: &Path) -> Result<(), String> {
    let download = release
        .download
        .as_ref()
        .ok_or("this release has no build for this Mac")?;
    let work = std::env::temp_dir().join(format!("strikebar-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let zip = work.join(&download.name);
        let sig = work.join(format!("{}.minisig", download.name));
        curl_file(&download.zip_url, &zip)?;
        curl_file(&download.signature_url, &sig)?;
        let bytes = std::fs::read(&zip).map_err(|e| e.to_string())?;
        let sig_text = std::fs::read_to_string(&sig).map_err(|e| e.to_string())?;
        verify(PUBLIC_KEY, &bytes, &sig_text, &download.name)?;

        let unpacked = work.join("unpacked");
        run(Command::new("ditto")
            .arg("-x")
            .arg("-k")
            .arg(&zip)
            .arg(&unpacked))?;
        let new_app = std::fs::read_dir(&unpacked)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("the download has no app in it")?;
        if bundle_id(&new_app)? != bundle_id(app)? {
            return Err("the download is a different app".into());
        }
        run(Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&new_app))
        .map_err(|e| format!("the downloaded app is damaged: {e}"))?;
        replace(app, &new_app)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Checks a minisign signature of `bytes`, and that it was made for `name`.
fn verify(public_key: &str, bytes: &[u8], signature: &str, name: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::decode(public_key)
        .map_err(|e| format!("bad update key: {e}"))?;
    let sig = minisign_verify::Signature::decode(signature)
        .map_err(|e| format!("bad signature file: {e}"))?;
    key.verify(bytes, &sig, false)
        .map_err(|_| "the download isn't signed by Strikebar's release key".to_owned())?;
    if sig.trusted_comment() != name {
        return Err(format!(
            "the signature is for {}, not {name}",
            sig.trusted_comment()
        ));
    }
    Ok(())
}

fn bundle_id(app: &Path) -> Result<String, String> {
    let out = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("{} has no bundle identifier", app.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Puts `new_app` where `app` is. The new copy is staged next to it first, so
/// the swap is two renames on one volume, and undone if the second fails.
fn replace(app: &Path, new_app: &Path) -> Result<(), String> {
    let parent = app.parent().ok_or("the app has no folder")?;
    let staged = parent.join(".Strikebar-update.app");
    let old = parent.join(".Strikebar-old.app");
    for leftover in [&staged, &old] {
        let _ = std::fs::remove_dir_all(leftover);
    }
    run(Command::new("ditto").arg(new_app).arg(&staged))
        .map_err(|e| format!("can't write to {}: {e}", parent.display()))?;
    if let Err(e) = std::fs::rename(app, &old) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("can't replace {}: {e}", app.display()));
    }
    if let Err(e) = std::fs::rename(&staged, app) {
        let _ = std::fs::rename(&old, app);
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("can't replace {}: {e}", app.display()));
    }
    // The running process keeps its files open; removing them is fine.
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Opens `app` (with `files`) once this process has quit. Call right before quitting.
pub fn relaunch(app: &Path, files: &[PathBuf]) -> Result<(), String> {
    // `open -n`: a new instance even if macOS still sees this one running.
    const SCRIPT: &str = r#"pid=$1; app=$2; shift 2
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
exec /usr/bin/open -n "$app" --args "$@""#;
    Command::new("/bin/sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg("sh")
        .arg(std::process::id().to_string())
        .arg(app)
        .args(files)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't relaunch: {e}"))
}

fn curl() -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["--silent", "--show-error", "--fail", "--location"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
    }
    cmd
}

fn curl_text(url: &str) -> Result<String, String> {
    let out = curl()
        .args([
            "--max-time",
            "15",
            "--header",
            "Accept: application/vnd.github+json",
        ])
        .arg(url)
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if !out.status.success() {
        return Err(curl_error(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn curl_file(url: &str, to: &Path) -> Result<(), String> {
    let out = curl()
        .args(["--max-time", "600", "--output"])
        .arg(to)
        .arg(url)
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {}", curl_error(&out.stderr)));
    }
    Ok(())
}

fn curl_error(stderr: &[u8]) -> String {
    let err = String::from_utf8_lossy(stderr);
    let err = err.trim().trim_start_matches("curl: ").trim();
    if err.is_empty() {
        "no answer".into()
    } else {
        err.to_owned()
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() {
            format!("{:?} failed", cmd.get_program())
        } else {
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway key (not the release key) and a file signed with it:
    /// `minisign -S -m msg.txt -t "Strikebar-9.9.9-macos-test.zip"`.
    const TEST_KEY: &str = "untrusted comment: minisign public key 6BEA6140E3C52F22
RWQiL8XjQGHqa4UIOhZfgm8OAG57oEtauapmfZVCm8tWA1Msmb599e71";
    const TEST_MSG: &[u8] = b"pretend this is a zip\n";
    const TEST_SIG: &str = "untrusted comment: signature from minisign secret key
RUQiL8XjQGHqaw1e/Bt5wYDRM/g/KrCFDRnCodS9XjZ+Nz5UNl4feb2c1nete/VqGBOInoSdm/72aucux27A4ZArPQ++k1G/Sgk=
trusted comment: Strikebar-9.9.9-macos-test.zip
8KVZgO9KD9GB3rsTzM+ZMcibZoPORQkQcjrNuY+yslqcTAef6H2EldtWgzktxiWY9vpP2WJrYiDVOdiXgxriAg==";

    #[test]
    fn release_key_parses() {
        assert!(minisign_verify::PublicKey::decode(PUBLIC_KEY).is_ok());
    }

    #[test]
    fn verifies_signatures_and_their_file_name() {
        let name = "Strikebar-9.9.9-macos-test.zip";
        assert_eq!(verify(TEST_KEY, TEST_MSG, TEST_SIG, name), Ok(()));
        // Changed file, different file name, or another key: refused.
        assert!(verify(TEST_KEY, b"pretend this is a zip?\n", TEST_SIG, name).is_err());
        assert!(
            verify(
                TEST_KEY,
                TEST_MSG,
                TEST_SIG,
                "Strikebar-1.0.0-macos-test.zip"
            )
            .is_err()
        );
        assert!(verify(PUBLIC_KEY, TEST_MSG, TEST_SIG, name).is_err());
    }

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
    fn reads_releases() {
        let name = asset_name("0.2.0");
        let json = format!(
            r#"{{"tag_name": "v0.2.0", "html_url": "https://example.com/r/v0.2.0", "assets": [
                {{"name": "{name}", "browser_download_url": "https://example.com/a.zip"}},
                {{"name": "{name}.minisig", "browser_download_url": "https://example.com/a.sig"}},
                {{"name": "Strikebar-0.2.0-windows.zip", "browser_download_url": "x"}}]}}"#
        );
        let release = parse_release(&json, "0.1.0").unwrap().unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.page, "https://example.com/r/v0.2.0");
        let download = release.download.unwrap();
        assert_eq!(download.zip_url, "https://example.com/a.zip");
        assert_eq!(download.signature_url, "https://example.com/a.sig");
        // Same version: nothing to do. No build for this Mac: page only.
        assert_eq!(parse_release(&json, "0.2.0"), Ok(None));
        let bare = r#"{"tag_name": "v0.3.0", "html_url": "u", "assets": []}"#;
        assert_eq!(
            parse_release(bare, "0.2.0").unwrap().unwrap().download,
            None
        );
        assert!(parse_release("{}", "0.1.0").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn replaces_the_bundle() {
        let dir = std::env::temp_dir().join(format!("strikebar-replace-{}", std::process::id()));
        let app = dir.join("Apps/Strikebar.app");
        let new_app = dir.join("new/Strikebar.app");
        for (path, text) in [(&app, "old"), (&new_app, "new")] {
            std::fs::create_dir_all(path.join("Contents")).unwrap();
            std::fs::write(path.join("Contents/version"), text).unwrap();
        }
        replace(&app, &new_app).unwrap();
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/version")).unwrap(),
            "new"
        );
        let left: Vec<_> = std::fs::read_dir(dir.join("Apps")).unwrap().collect();
        assert_eq!(left.len(), 1, "staging copies are cleaned up");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
