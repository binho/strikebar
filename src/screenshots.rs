//! Screenshots for the README and the App Store, taken by the app itself (at
//! full Retina resolution, with no Screen Recording permission needed).
//!
//! Off unless `STRIKEBAR_SCREENSHOTS=<folder>` is set. Then the window opens at
//! 1280×800 points (2560×1600 pixels on Retina, a Mac App Store size). Each time
//! a file named `take` appears in the folder, the app runs it as a small scene
//! script, sets the window back to that size if it was resized, and saves it as
//! `<folder>/<name>.png`:
//!
//! ```text
//! doc memo.png                      # show the open image with this file name
//! clear                             # remove its boxes
//! box 820 610 300 46                # centre x, y and width, height in image pixels
//! box 820 700 300 46 pixelate block=40
//! box 820 790 300 46 fill=white pill angle=-3
//! select 1                          # 0-based box index, or `none`
//! size 1920                         # save size preset (`original` for none)
//! menu size                         # open the save size menu
//! save                              # save a copy of the shown image (or `save-all`)
//! shot 01-memo                      # file name of the screenshot
//! ```
//!
//! A `take` file holding just a name (`echo 01-editor > take`) only takes a screenshot.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use image::RgbaImage;

use crate::redact::{FillColor, SaveSize, Shape, Style};

pub const ENV: &str = "STRIKEBAR_SCREENSHOTS";

/// Window size in points while taking screenshots.
pub const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];

/// Time to let pass after a scene before capturing, so new boxes are rendered
/// into the texture and opened menus have finished fading in.
const SETTLE: Duration = Duration::from_millis(600);

/// One line of a scene script, applied by the app (see `StrikebarApp::run_scene`).
#[derive(Debug, PartialEq)]
pub enum Command {
    Doc(String),
    Clear,
    Box {
        cx: f32,
        cy: f32,
        w: f32,
        h: f32,
        style: Style,
        shape: Shape,
        angle_deg: f32,
        block: Option<u32>,
    },
    Select(Option<usize>),
    Size(SaveSize),
    OpenSizeMenu,
    Save,
    SaveAll,
}

pub struct Capture {
    dir: PathBuf,
    count: usize,
    /// Requested screenshot and when it may be taken.
    pending: Option<(String, Instant)>,
}

impl Capture {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os(ENV)?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            dir,
            count: 0,
            pending: None,
        })
    }

    /// Call once per frame, before drawing: saves finished screenshots, starts
    /// requested ones, and returns the scene commands to apply this frame.
    pub fn tick(&mut self, ctx: &egui::Context) -> Vec<Command> {
        self.save_finished(ctx);

        let mut commands = Vec::new();
        let trigger = self.dir.join("take");
        if let Ok(contents) = std::fs::read_to_string(&trigger) {
            let _ = std::fs::remove_file(&trigger);
            self.count += 1;
            let (parsed, name) = parse(&contents);
            for e in &parsed.errors {
                eprintln!("scene: {e}");
            }
            commands = parsed.commands;
            let name = name.unwrap_or_else(|| format!("shot-{}", self.count));
            self.pending = Some((name.replace(['/', '\\'], "-"), Instant::now() + SETTLE));
        }

        // Only capture at the exact size (the App Store needs it). The window
        // stays freely resizable otherwise; it's put back just for the shot.
        if let Some((name, ready_at)) = &self.pending {
            let target = egui::vec2(WINDOW_SIZE[0], WINDOW_SIZE[1]);
            let (size, maximized) = ctx.input(|i| {
                let v = i.viewport();
                (v.inner_rect.map(|r| r.size()), v.maximized == Some(true))
            });
            if size.is_some_and(|s| (s - target).length() <= 0.5) {
                if Instant::now() >= *ready_at {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                        name.clone(),
                    )));
                    self.pending = None;
                } else {
                    ctx.request_repaint();
                }
            } else {
                if maximized {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
            }
        }
        // Keep polling for the trigger file even when nothing else repaints.
        ctx.request_repaint_after(Duration::from_millis(250));
        commands
    }

    fn save_finished(&self, ctx: &egui::Context) {
        let done: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot {
                        user_data, image, ..
                    } => Some((user_data.clone(), image.clone())),
                    _ => None,
                })
                .collect()
        });
        for (data, image) in done {
            let name = data
                .data
                .as_ref()
                .and_then(|d| d.downcast_ref::<String>())
                .cloned()
                .unwrap_or_else(|| "shot".into());
            let [w, h] = image.size;
            let bytes = image
                .pixels
                .iter()
                .flat_map(|c| c.to_srgba_unmultiplied())
                .collect();
            let path = self.dir.join(format!("{name}.png"));
            let saved = RgbaImage::from_raw(w as u32, h as u32, bytes)
                .ok_or_else(|| "bad screenshot buffer".to_owned())
                .and_then(|img| img.save(&path).map_err(|e| e.to_string()));
            match saved {
                Ok(()) => eprintln!("screenshot: {} ({w}×{h})", path.display()),
                Err(e) => eprintln!("screenshot {}: {e}", path.display()),
            }
        }
    }
}

#[derive(Debug, Default)]
struct Parsed {
    commands: Vec<Command>,
    errors: Vec<String>,
}

/// Parses a scene script. Returns the commands and the `shot` name, if any.
fn parse(text: &str) -> (Parsed, Option<String>) {
    let mut out = Parsed::default();
    let mut name = None;
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .collect();
    // Just a name: screenshot only (the original `echo name > take` use).
    if let [only] = lines.as_slice()
        && !only.contains(char::is_whitespace)
        && !matches!(*only, "clear" | "save" | "save-all")
    {
        return (out, Some((*only).to_owned()));
    }
    for line in lines {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let args: Vec<&str> = words.collect();
        match parse_line(cmd, &args) {
            Ok(Some(c)) => out.commands.push(c),
            Ok(None) => name = args.first().map(|s| (*s).to_owned()),
            Err(e) => out.errors.push(format!("{line:?}: {e}")),
        }
    }
    (out, name)
}

/// `Ok(None)` for `shot`, which isn't a command for the app.
fn parse_line(cmd: &str, args: &[&str]) -> Result<Option<Command>, String> {
    let num = |s: &str| s.parse::<f32>().map_err(|_| format!("not a number: {s}"));
    Ok(Some(match (cmd, args) {
        ("shot", [_]) => return Ok(None),
        ("doc", [name]) => Command::Doc((*name).to_owned()),
        ("clear", []) => Command::Clear,
        ("save", []) => Command::Save,
        ("save-all", []) => Command::SaveAll,
        ("menu", ["size"]) => Command::OpenSizeMenu,
        ("select", ["none"]) => Command::Select(None),
        ("select", [n]) => Command::Select(Some(n.parse().map_err(|_| "bad index")?)),
        ("size", ["original"]) => Command::Size(SaveSize::Original),
        ("size", [px]) => Command::Size(SaveSize::LongSide(px.parse().map_err(|_| "bad size")?)),
        ("box", [cx, cy, w, h, opts @ ..]) => {
            let mut style = Style::Fill(FillColor::Black);
            let mut shape = Shape::Rect;
            let mut angle_deg = 0.0;
            let mut block = None;
            for opt in opts {
                match opt.split_once('=') {
                    None if *opt == "pixelate" => style = Style::Pixelate,
                    None if *opt == "fill" => style = Style::Fill(FillColor::Black),
                    None if *opt == "pill" => shape = Shape::Pill,
                    None if *opt == "rect" => shape = Shape::Rect,
                    Some(("fill", c)) => {
                        style = Style::Fill(
                            FillColor::ALL
                                .into_iter()
                                .find(|f| f.name().eq_ignore_ascii_case(c))
                                .ok_or_else(|| format!("unknown color: {c}"))?,
                        );
                    }
                    Some(("angle", a)) => angle_deg = num(a)?,
                    Some(("block", b)) => block = Some(b.parse().map_err(|_| "bad block")?),
                    _ => return Err(format!("unknown box option: {opt}")),
                }
            }
            Command::Box {
                cx: num(cx)?,
                cy: num(cy)?,
                w: num(w)?,
                h: num(h)?,
                style,
                shape,
                angle_deg,
                block,
            }
        }
        _ => return Err("unknown command".into()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_scene() {
        let (p, name) = parse(
            "doc memo.png\nclear\nbox 10 20 30 40 # comment\n\
             box 1 2 3 4 pixelate block=12 pill angle=-3\nbox 1 2 3 4 fill=white\n\
             select 1\nsize 1920\nmenu size\nsave\nshot 01-memo\n",
        );
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(name.as_deref(), Some("01-memo"));
        assert_eq!(p.commands.len(), 9);
        assert_eq!(
            p.commands[3],
            Command::Box {
                cx: 1.0,
                cy: 2.0,
                w: 3.0,
                h: 4.0,
                style: Style::Pixelate,
                shape: Shape::Pill,
                angle_deg: -3.0,
                block: Some(12),
            }
        );
        assert!(matches!(
            p.commands[4],
            Command::Box {
                style: Style::Fill(FillColor::White),
                ..
            }
        ));
    }

    #[test]
    fn a_bare_name_only_takes_a_screenshot() {
        let (p, name) = parse("01-editor\n");
        assert!(p.commands.is_empty());
        assert_eq!(name.as_deref(), Some("01-editor"));
    }

    #[test]
    fn reports_bad_lines() {
        let (p, _) = parse("box 1 2 three 4\nfly away\n");
        assert_eq!(p.errors.len(), 2);
    }
}
