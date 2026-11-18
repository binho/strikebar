//! Zed-like look: One Dark / One Light colors, compact spacing, ghost buttons, Lucide icons and
//! the system UI font (SF Pro on macOS, Segoe UI on Windows) when it can be found.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response,
    RichText, Stroke, TextStyle, Ui, Vec2,
};
pub use lucide_icons::Icon;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The UI colors of one theme.
pub struct Palette {
    pub title_bar: Color32,
    pub panel: Color32,
    pub editor: Color32,
    /// Behind the image. Darker than `editor` in light mode, so white pages stand out.
    pub canvas: Color32,
    pub border: Color32,
    pub border_variant: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_placeholder: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub accent: Color32,
    pub warning: Color32,
}

/// Zed "One Dark".
pub const ONE_DARK: Palette = Palette {
    title_bar: hex(0x3b414d),
    panel: hex(0x2f343e),
    editor: hex(0x282c33),
    canvas: hex(0x282c33),
    border: hex(0x464b57),
    border_variant: hex(0x363c46),
    text: hex(0xdce0e5),
    text_muted: hex(0xa9afbc),
    text_placeholder: hex(0x878a98),
    hover: hex(0x363c46),
    selected: hex(0x454a56),
    accent: hex(0x74ade8),
    warning: hex(0xdec184),
};

/// Zed "One Light".
pub const ONE_LIGHT: Palette = Palette {
    title_bar: hex(0xdcdcdd),
    panel: hex(0xebebec),
    editor: hex(0xfafafa),
    canvas: hex(0xe2e2e4),
    border: hex(0xc9c9ca),
    border_variant: hex(0xdfdfe0),
    text: hex(0x242529),
    text_muted: hex(0x58585a),
    text_placeholder: hex(0x7e8086),
    hover: hex(0xdfdfe0),
    selected: hex(0xcacaca),
    accent: hex(0x5c78e2),
    warning: hex(0xa48819),
};

/// Whether this frame is drawn light. Set by [`sync`] at the start of each frame
/// from egui's theme (which follows the OS when the preference is Auto).
static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn palette() -> &'static Palette {
    if LIGHT.load(Ordering::Relaxed) {
        &ONE_LIGHT
    } else {
        &ONE_DARK
    }
}

macro_rules! colors {
    ($($name:ident),* $(,)?) => {
        $(
            #[doc = concat!("`", stringify!($name), "` of the current theme.")]
            pub fn $name() -> Color32 {
                palette().$name
            }
        )*
    };
}
colors!(
    title_bar,
    panel,
    editor,
    canvas,
    border,
    border_variant,
    text,
    text_muted,
    text_placeholder,
    hover,
    selected,
    accent,
    warning,
);

/// Call at the start of every frame, before drawing.
pub fn sync(ctx: &egui::Context) {
    LIGHT.store(ctx.theme() == egui::Theme::Light, Ordering::Relaxed);
}

/// Appearance setting: follow the OS, or always light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Appearance {
    #[default]
    Auto,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Auto => Icon::SunMoon,
            Self::Light => Icon::Sun,
            Self::Dark => Icon::Moon,
        }
    }

    /// Stored name, for the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key() == key)
    }

    pub fn set(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::Auto => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        });
        sync(ctx);
    }
}

pub const TITLE_BAR_HEIGHT: f32 = 32.0;
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
pub const TOOLBAR_HEIGHT: f32 = 36.0;
/// Room for the macOS traffic lights, which sit on top of our title bar.
pub const TRAFFIC_LIGHTS_WIDTH: f32 = if cfg!(target_os = "macos") { 78.0 } else { 8.0 };

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&ONE_DARK, egui::Visuals::dark()));
    ctx.set_visuals_of(
        egui::Theme::Light,
        visuals(&ONE_LIGHT, egui::Visuals::light()),
    );
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

fn visuals(p: &Palette, mut v: egui::Visuals) -> egui::Visuals {
    let radius = CornerRadius::same(4);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = p.editor;
    v.faint_bg_color = p.hover;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.selection.bg_fill = p.selected;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    // Labels, separators, panel borders.
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border_variant);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;
    // Ghost buttons: no background until hovered.
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = p.selected; // slider rail, checkbox box
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.weak_bg_fill = p.selected;
    w.active.bg_fill = p.selected;
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, p.text);
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
            ui.label(RichText::new(label).color(text()));
            if let Some(s) = shortcut {
                ui.add_space(8.0);
                ui.label(RichText::new(s).color(text_placeholder()));
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
        text_muted()
    } else {
        text_placeholder().gamma_multiply(0.6)
    };
    let button = egui::Button::new(text.color(color)).min_size(Vec2::new(26.0, 24.0));
    let response = ui.add_enabled(enabled, button);
    tooltip(response, label, shortcut)
}

/// The main action of a bar or dialog (Save copy, Done): accent-tinted.
pub fn primary_button<'a>(text: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::new(text)
        .fill(accent().gamma_multiply(0.22))
        .stroke(Stroke::new(1.0, accent().gamma_multiply(0.45)))
}

/// Thin vertical divider for toolbars.
pub fn divider(ui: &mut Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, border());
    ui.add_space(4.0);
}

pub fn frame(fill: Color32, margin: Margin) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(margin)
}
