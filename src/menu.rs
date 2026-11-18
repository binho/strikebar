//! Native macOS menu bar: the standard app menu (as winit makes it) plus
//! Settings… (⌘,). Windows and Linux have no app menu; the title bar has a
//! gear button there instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

pub struct MenuBar {
    /// Kept alive: dropping it would remove the items.
    _menu: Menu,
    settings: Arc<AtomicBool>,
}

impl MenuBar {
    /// Replaces winit's default menu bar. Call once the app has started.
    pub fn install(ctx: &egui::Context, app_name: &str) -> Option<Self> {
        let settings_item = MenuItem::new(
            "Settings…",
            true,
            Some(Accelerator::new(Modifiers::META, Code::Comma)),
        );
        let sep = PredefinedMenuItem::separator;
        let app_menu = Submenu::with_items(
            app_name,
            true,
            &[
                &PredefinedMenuItem::about(Some(&format!("About {app_name}")), None),
                &sep(),
                &settings_item,
                &sep(),
                &PredefinedMenuItem::services(None),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &PredefinedMenuItem::quit(None),
            ],
        )
        .ok()?;
        let menu = Menu::with_items(&[&app_menu]).ok()?;
        menu.init_for_nsapp();

        // Menu clicks aren't window events, so wake egui up to handle them.
        let settings = Arc::new(AtomicBool::new(false));
        let (flag, ctx, id) = (settings.clone(), ctx.clone(), settings_item.id().clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == id {
                flag.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
        }));
        Some(Self {
            _menu: menu,
            settings,
        })
    }

    /// Whether Settings… was chosen since the last call.
    pub fn settings_chosen(&self) -> bool {
        self.settings.swap(false, Ordering::Relaxed)
    }
}
