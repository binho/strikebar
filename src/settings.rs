//! User settings, kept between runs in eframe's storage (see `StrikebarApp::save`).

use serde::{Deserialize, Serialize};

use crate::redact::{self, CopyNaming, FillColor, SaveSize, Shape, Style};
use crate::theme::Appearance;

/// Key of the settings in eframe's storage.
pub const KEY: &str = "settings";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    /// Start with the style, color and shape used last.
    pub remember_tools: bool,
    /// What new boxes looked like when the app last quit.
    pub last_tools: Tools,
    /// Pixelate and blur block size for new boxes, at launch.
    pub strength: Strength,
    pub naming: CopyNaming,
    pub jpeg_quality: u8,
    /// Save size at launch.
    pub save_size: SaveSize,
    pub after_save: AfterSave,
    pub check_updates: bool,
    /// Unix time of the last automatic update check.
    pub last_update_check: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            remember_tools: true,
            last_tools: Tools::default(),
            strength: Strength::default(),
            naming: CopyNaming::default(),
            jpeg_quality: redact::DEFAULT_JPEG_QUALITY,
            save_size: SaveSize::Original,
            after_save: AfterSave::default(),
            // Off unless asked for: a redaction app shouldn't phone home on its own.
            check_updates: false,
            last_update_check: 0,
        }
    }
}

impl Settings {
    /// The tools new boxes start with.
    pub fn start_tools(&self) -> Tools {
        if self.remember_tools {
            self.last_tools
        } else {
            Tools::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tools {
    pub style: Style,
    pub fill_color: FillColor,
    pub shape: Shape,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            style: Style::Fill(FillColor::Black),
            fill_color: FillColor::Black,
            shape: Shape::Rect,
        }
    }
}

/// Default pixelate and blur block size, relative to the image's long side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strength {
    Light,
    #[default]
    Medium,
    Strong,
}

impl Strength {
    pub const ALL: [Self; 3] = [Self::Light, Self::Medium, Self::Strong];

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Strong => "Strong",
        }
    }

    /// Block size as a fraction of the image's long side.
    pub fn fraction(self) -> f32 {
        match self {
            Self::Light => 1.0 / 120.0,
            Self::Medium => redact::DEFAULT_BLOCK_FRACTION,
            Self::Strong => 1.0 / 50.0,
        }
    }
}

/// What happens after a copy is saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterSave {
    #[default]
    Nothing,
    /// Show the saved file in Finder / Explorer.
    Reveal,
    /// Select the next image in the list.
    NextImage,
}

impl AfterSave {
    pub const ALL: [Self; 3] = [Self::Nothing, Self::Reveal, Self::NextImage];

    pub fn label(self) -> &'static str {
        match self {
            Self::Nothing => "Stay on the image",
            Self::Reveal if cfg!(target_os = "macos") => "Show it in Finder",
            Self::Reveal => "Show it in its folder",
            Self::NextImage => "Go to the next image",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_or_partial_settings_keep_the_defaults() {
        let s: Settings = ron::from_str("(jpeg_quality: 70)").unwrap();
        assert_eq!(s.jpeg_quality, 70);
        assert_eq!(s.naming, CopyNaming::default());
        assert!(s.remember_tools);
        assert!(!s.check_updates);
    }

    #[test]
    fn settings_roundtrip() {
        let s = Settings {
            appearance: Appearance::Light,
            strength: Strength::Strong,
            save_size: SaveSize::LongSide(1920),
            last_tools: Tools {
                style: Style::Blur,
                fill_color: FillColor::Red,
                shape: Shape::Pill,
            },
            ..Settings::default()
        };
        let text = ron::to_string(&s).unwrap();
        assert_eq!(ron::from_str::<Settings>(&text).unwrap(), s);
    }
}
