//! Handing things to the OS: opening links and showing files in Finder/Explorer.

use std::path::Path;
use std::process::Command;

/// Opens a URL in the default browser.
pub fn open_url(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    spawn(cmd.arg(url));
}

/// Shows a file selected in Finder or Explorer. Linux file managers have no
/// common way to select a file, so its folder is opened there.
pub fn reveal(path: &Path) {
    if cfg!(target_os = "macos") {
        spawn(Command::new("open").arg("-R").arg(path));
    } else if cfg!(windows) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        spawn(Command::new("explorer").arg(arg));
    } else if let Some(dir) = path.parent() {
        spawn(Command::new("xdg-open").arg(dir));
    }
}

fn spawn(cmd: &mut Command) {
    if let Err(e) = cmd.spawn() {
        eprintln!("couldn't run {:?}: {e}", cmd.get_program());
    }
}
