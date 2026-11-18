//! The keyboard shortcuts dialog, opened with ⌘/ or the keyboard button in the status bar.

use eframe::egui::{self, Align2, FontId, Margin, Pos2, RichText, Sense, Stroke, Ui, vec2};

use crate::theme::{self, Icon};

/// One part of a shortcut, drawn right-aligned on its row.
#[derive(Clone, Copy)]
enum Cap {
    /// ⌘ on macOS, Ctrl elsewhere.
    Cmd,
    /// ⇧ on macOS, Shift elsewhere.
    Shift,
    Backspace,
    Up,
    Down,
    /// A keycap with this text: a key, or a pointer gesture like `drag`.
    Key(&'static str),
    /// Plain text between keycaps.
    Word(&'static str),
}
use Cap::{Backspace, Cmd, Down, Key, Shift, Up, Word};

type Row = (&'static str, &'static [Cap]);
type Section = (&'static str, &'static [Row]);

const LEFT: &[Section] = &[
    (
        "General",
        &[
            ("Open images", &[Cmd, Key("O")]),
            ("Save copy", &[Cmd, Key("S")]),
            ("Undo last box", &[Cmd, Key("Z")]),
            ("Open settings", &[Cmd, Key(",")]),
            ("Show keyboard shortcuts", &[Cmd, Key("/")]),
        ],
    ),
    (
        "Images",
        &[
            ("Previous image", &[Up]),
            ("Next image", &[Down]),
            ("Remove image (in the list)", &[Backspace]),
        ],
    ),
    (
        "View",
        &[
            ("Fit to window", &[Key("0")]),
            ("Actual size", &[Key("1")]),
            ("Zoom in", &[Cmd, Key("+")]),
            ("Zoom out", &[Cmd, Key("−")]),
            ("Zoom", &[Key("pinch"), Word("or"), Cmd, Key("scroll")]),
            (
                "Pan",
                &[Key("space"), Key("drag"), Word("or"), Key("scroll")],
            ),
        ],
    ),
];

const RIGHT: &[Section] = &[
    (
        "Boxes",
        &[
            ("Draw a box", &[Key("drag")]),
            ("Select a box", &[Key("click")]),
            ("Move box", &[Key("drag")]),
            ("Resize box (its handles)", &[Key("drag")]),
            ("Rotate box (the ○ above it)", &[Key("drag")]),
            ("Snap rotation to 15°", &[Shift, Key("drag")]),
            ("Delete box", &[Backspace, Word("or"), Key("right-click")]),
            ("Deselect", &[Key("esc")]),
        ],
    ),
    (
        "Style",
        &[
            ("Fill", &[Key("F")]),
            ("Pixelate", &[Key("P")]),
            ("Blur", &[Key("B")]),
        ],
    ),
];

const COLUMN_GAP: f32 = 40.0;
const DIALOG_WIDTH: f32 = 640.0; // fits the smallest window (720 points wide)
const ROW_HEIGHT: f32 = 27.0;
const CAP_HEIGHT: f32 = 21.0;
const CAP_GAP: f32 = 4.0;

/// Shows the dialog. Returns `true` once it should close.
pub fn dialog(ctx: &egui::Context) -> bool {
    let frame = egui::Frame::popup(&ctx.global_style())
        .fill(theme::panel())
        .corner_radius(12)
        .inner_margin(Margin::same(20));
    let modal = egui::Modal::new(egui::Id::new("shortcuts"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.set_width(DIALOG_WIDTH);
            let close = ui
                .horizontal(|ui| {
                    ui.label(
                        RichText::new("Keyboard shortcuts")
                            .heading()
                            .color(theme::text()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::icon_button(ui, Icon::X, true, "Close", Some("esc")).clicked()
                    })
                    .inner
                })
                .inner;
            ui.add_space(10.0);
            // Scrolls only when the window is too short for it.
            let max_height = (ui.ctx().content_rect().height() - 120.0).max(140.0);
            // A bar that takes its own space, so it never covers the keys.
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            egui::ScrollArea::vertical()
                .max_height(max_height)
                // The dialog's area would otherwise cap this at its last height.
                .min_scrolled_height(max_height)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                    ui.columns_const(|[left, right]| {
                        column(left, LEFT);
                        column(right, RIGHT);
                    });
                });
            close
        });
    modal.inner || modal.should_close()
}

fn column(ui: &mut Ui, sections: &[Section]) {
    ui.spacing_mut().item_spacing.y = 0.0;
    for (i, (title, rows)) in sections.iter().enumerate() {
        if i > 0 {
            ui.add_space(16.0);
        }
        ui.label(
            RichText::new(*title)
                .size(12.5)
                .strong()
                .color(theme::text_muted()),
        );
        ui.add_space(5.0);
        for (label, caps) in *rows {
            row(ui, label, caps);
        }
    }
}

fn row(ui: &mut Ui, label: &str, caps: &[Cap]) {
    let size = vec2(ui.available_width(), ROW_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        theme::text(),
    );
    let mut right = rect.right();
    for cap in caps.iter().rev() {
        right = paint_cap(ui, *cap, Pos2::new(right, rect.center().y)) - CAP_GAP;
    }
}

/// Draws `cap` ending at `end` (its right edge, vertically centered) and returns
/// its left edge.
fn paint_cap(ui: &Ui, cap: Cap, end: Pos2) -> f32 {
    let mono = FontId::monospace(11.5);
    let icon = |icon: Icon| (icon.unicode().to_string(), FontId::proportional(13.0));
    let (text, font) = match cap {
        Cmd if cfg!(target_os = "macos") => icon(Icon::Command),
        Cmd => ("ctrl".to_owned(), mono),
        Shift if cfg!(target_os = "macos") => icon(Icon::ArrowBigUp),
        Shift => ("shift".to_owned(), mono),
        Backspace => icon(Icon::Delete),
        Up => icon(Icon::ArrowUp),
        Down => icon(Icon::ArrowDown),
        Key(key) => (key.to_owned(), mono),
        Word(word) => {
            let rect = ui.painter().text(
                end - vec2(2.0, 0.0),
                Align2::RIGHT_CENTER,
                word,
                FontId::proportional(12.5),
                theme::text_placeholder(),
            );
            return rect.left() - 2.0;
        }
    };
    let galley = ui.painter().layout_no_wrap(text, font, theme::text_muted());
    let width = (galley.size().x + 12.0).max(CAP_HEIGHT + 1.0);
    let rect = egui::Rect::from_min_max(
        Pos2::new(end.x - width, end.y - CAP_HEIGHT / 2.0),
        Pos2::new(end.x, end.y + CAP_HEIGHT / 2.0),
    );
    ui.painter().rect(
        rect,
        5,
        theme::keycap(),
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, theme::text_muted());
    rect.left()
}
