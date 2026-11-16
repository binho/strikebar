//! Zed-like look: One Dark colors, compact spacing, ghost buttons, Lucide icons and
//! the system UI font (SF Pro on macOS, Segoe UI on Windows) when it can be found.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response,
    RichText, Stroke, TextStyle, Ui, Vec2,
};
pub use lucide_icons::Icon;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

// Zed "One Dark".
pub const TITLE_BAR: Color32 = hex(0x3b414d);
pub const PANEL: Color32 = hex(0x2f343e);
pub const EDITOR: Color32 = hex(0x282c33);
pub const BORDER: Color32 = hex(0x464b57);
pub const BORDER_VARIANT: Color32 = hex(0x363c46);
pub const TEXT: Color32 = hex(0xdce0e5);
pub const TEXT_MUTED: Color32 = hex(0xa9afbc);
pub const TEXT_PLACEHOLDER: Color32 = hex(0x878a98);
pub const HOVER: Color32 = hex(0x363c46);
pub const SELECTED: Color32 = hex(0x454a56);
pub const ACCENT: Color32 = hex(0x74ade8);
pub const WARNING: Color32 = hex(0xdec184);

pub const TITLE_BAR_HEIGHT: f32 = 32.0;
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
pub const TOOLBAR_HEIGHT: f32 = 36.0;
/// Room for the macOS traffic lights, which sit on top of our title bar.
pub const TRAFFIC_LIGHTS_WIDTH: f32 = if cfg!(target_os = "macos") { 78.0 } else { 8.0 };

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Heading, FontId::proportional(15.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(4.0, 4.0);
        s.button_padding = Vec2::new(6.0, 3.0);
        s.interact_size = Vec2::new(24.0, 22.0);
        s.menu_margin = Margin::same(4);
        s.window_margin = Margin::same(8);
        s.slider_width = 90.0;
        s.slider_rail_height = 4.0;
        style.interaction.tooltip_delay = 0.4;
    });
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let radius = CornerRadius::same(4);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = EDITOR;
    v.faint_bg_color = HOVER;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARNING;
    v.selection.bg_fill = SELECTED;
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    // Labels, separators, panel borders.
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER_VARIANT);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = radius;
    // Ghost buttons: no background until hovered.
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = SELECTED; // slider rail, checkbox box
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.weak_bg_fill = SELECTED;
    w.active.bg_fill = SELECTED;
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open = w.active;
    v
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "lucide".into(),
        Arc::new(
            FontData::from_static(lucide_icons::LUCIDE_FONT_BYTES).tweak(egui::FontTweak {
                y_offset_factor: 0.07,
                ..Default::default()
            }),
        ),
    );

    let ui_font = [
        "/System/Library/Fonts/SFNS.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
    ]
    .iter()
    .find_map(|p| std::fs::read(p).ok());
    let mono_font = [
        "/System/Library/Fonts/SFNSMono.ttf",
        "C:\\Windows\\Fonts\\consola.ttf",
    ]
    .iter()
    .find_map(|p| std::fs::read(p).ok());

    if let Some(bytes) = ui_font {
        fonts
            .font_data
            .insert("ui".into(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "ui".into());
    }
    if let Some(bytes) = mono_font {
        fonts
            .font_data
            .insert("mono".into(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "mono".into());
    }
    // Icons as a fallback in every family, so they can sit inline in any text.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("lucide".into());
    }
    fonts
}

/// Text with a leading icon.
pub fn icon_text(icon: Icon, text: &str) -> String {
    if text.is_empty() {
        icon.unicode().to_string()
    } else {
        format!("{}  {text}", icon.unicode())
    }
}

/// Zed-style tooltip: action name plus its shortcut, dimmed.
pub fn tooltip(response: Response, label: &str, shortcut: Option<&str>) -> Response {
    response.on_hover_ui(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(TEXT));
            if let Some(s) = shortcut {
                ui.add_space(8.0);
                ui.label(RichText::new(s).color(TEXT_PLACEHOLDER));
            }
        });
    })
}

/// Square ghost button with just an icon.
pub fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    enabled: bool,
    label: &str,
    shortcut: Option<&str>,
) -> Response {
    let text = RichText::new(icon.unicode().to_string()).size(15.0);
    let color = if enabled {
        TEXT_MUTED
    } else {
        TEXT_PLACEHOLDER.gamma_multiply(0.6)
    };
    let button = egui::Button::new(text.color(color)).min_size(Vec2::new(26.0, 24.0));
    let response = ui.add_enabled(enabled, button);
    tooltip(response, label, shortcut)
}

/// Thin vertical divider for toolbars.
pub fn divider(ui: &mut Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
    ui.add_space(4.0);
}

pub fn frame(fill: Color32, margin: Margin) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(margin)
}
