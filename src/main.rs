#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window on Windows release builds

mod app;
mod loader;
mod redact;
mod screenshots;
mod theme;

const APP_NAME: &str = "Strikebar";

fn main() -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size(if std::env::var_os(screenshots::ENV).is_some() {
            screenshots::WINDOW_SIZE
        } else {
            [1280.0, 820.0]
        })
        .with_min_inner_size([720.0, 420.0])
        .with_drag_and_drop(true)
        // macOS: draw our own title bar under the traffic lights, like Zed.
        // Ignored on other platforms, which keep their native title bar.
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    // Window/taskbar icon. The macOS bundle uses assets/Strikebar.icns instead;
    // this one shows in the Dock for unbundled runs (`cargo run`).
    // Rendered by scripts/make-icon.sh.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let paths: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::StrikebarApp::new(cc, paths)))),
    )
}
