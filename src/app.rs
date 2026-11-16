use std::path::PathBuf;

use eframe::egui::{
    self, Color32, CursorIcon, Key, Margin, Modifiers, PointerButton, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, TextureFilter, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use image::RgbaImage;
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

use crate::loader::{LoadResult, Loader};
use crate::redact::{self, FillColor, Region, SaveSize, Style};
use crate::theme::{self, Icon};

const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 32.0;
const ZOOM_STEP: f32 = 1.25;
/// Half the side of a resize handle, and how close (in screen points) the pointer must be to grab one.
const HANDLE_RADIUS: f32 = 4.0;
const HANDLE_GRAB: f32 = 8.0;
const EDGE_SLACK: f32 = 3.0;

/// Crisp pixels when zoomed in, smooth when zoomed out.
const TEXTURE_OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    ..TextureOptions::LINEAR
};

/// Rectangle in image pixels (`x0..x1`, `y0..y1`), for tracking what to re-render.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PxRect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl PxRect {
    /// Pixels drawing a (possibly rotated) box can change, antialiased edge included.
    fn of(r: &Region) -> Self {
        let (a, b, c, d) = r.affected_pixels();
        Self {
            x0: a.max(0) as u32,
            y0: b.max(0) as u32,
            x1: c.max(0) as u32,
            y1: d.max(0) as u32,
        }
    }
    fn union(self, o: Self) -> Self {
        Self {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    fn intersects(self, o: Self) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
}

/// A view change that needs the canvas rect, so it's applied when the canvas is drawn.
#[derive(Clone, Copy)]
enum ViewRequest {
    Fit,
    Actual,
    ZoomBy(f32),
}

/// Which area the Delete key acts on: the last one clicked.
#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Sidebar,
    Canvas,
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    const ALL: [Handle; 8] = [
        Handle::NW,
        Handle::N,
        Handle::NE,
        Handle::E,
        Handle::SE,
        Handle::S,
        Handle::SW,
        Handle::W,
    ];

    /// Where the handle sits in the box's own frame (see [`Region::to_local`]).
    fn local(self, r: &Region) -> (f32, f32) {
        let (w, h) = (r.w, r.h);
        match self {
            Handle::NW => (0.0, 0.0),
            Handle::N => (w / 2.0, 0.0),
            Handle::NE => (w, 0.0),
            Handle::E => (w, h / 2.0),
            Handle::SE => (w, h),
            Handle::S => (w / 2.0, h),
            Handle::SW => (0.0, h),
            Handle::W => (0.0, h / 2.0),
        }
    }

    /// Resize cursor pointing the way this handle moves on screen, for a box
    /// rotated by `angle`.
    fn cursor(self, angle: f32) -> CursorIcon {
        let base: f32 = match self {
            Handle::E => 0.0,
            Handle::SE => 45.0,
            Handle::S => 90.0,
            Handle::SW => 135.0,
            Handle::W => 180.0,
            Handle::NW => 225.0,
            Handle::N => 270.0,
            Handle::NE => 315.0,
        };
        let dir = (base + angle.to_degrees()).rem_euclid(180.0);
        match ((dir + 22.5) / 45.0) as u32 % 4 {
            0 => CursorIcon::ResizeHorizontal,
            1 => CursorIcon::ResizeNwSe,
            2 => CursorIcon::ResizeVertical,
            _ => CursorIcon::ResizeNeSw,
        }
    }

    fn moves_left(self) -> bool {
        matches!(self, Handle::W | Handle::NW | Handle::SW)
    }
    fn moves_right(self) -> bool {
        matches!(self, Handle::E | Handle::NE | Handle::SE)
    }
    fn moves_top(self) -> bool {
        matches!(self, Handle::N | Handle::NW | Handle::NE)
    }
    fn moves_bottom(self) -> bool {
        matches!(self, Handle::S | Handle::SW | Handle::SE)
    }
}

/// What the current primary-button drag on the canvas is doing. Points are in image coordinates.
enum DragOp {
    New {
        start: Pos2,
    },
    Move {
        idx: usize,
        grab: Pos2,
        orig: Region,
    },
    Resize {
        idx: usize,
        handle: Handle,
        orig: Region,
    },
    /// `start` is where the pointer grabbed the rotate handle.
    Rotate {
        idx: usize,
        start: Pos2,
        orig: Region,
    },
}

/// What's under the pointer on the selected box.
#[derive(Clone, Copy)]
enum Grab {
    Resize(Handle),
    Rotate,
}

/// Screen distance from the top edge to the rotate handle.
const ROTATE_HANDLE_OFFSET: f32 = 26.0;
const ROTATE_SNAP_DEGREES: f32 = 15.0;

/// One open image with its pending redactions and its own view (zoom/pan).
struct Doc {
    /// Order the image was opened in; the list is kept sorted by it.
    seq: u64,
    path: PathBuf,
    name: String,
    original: RgbaImage,
    regions: Vec<Region>,
    selected: Option<usize>,
    /// Region being moved/resized: left out of the preview texture and drawn as an
    /// overlay instead, so dragging doesn't re-render every frame.
    editing: Option<usize>,
    /// Small preview for the sidebar (boxes are drawn on top of it).
    thumb: TextureHandle,
    /// Full-size preview with the regions burned in, so what you see is what gets
    /// saved. Only the image on screen has one: a 48 MP photo is ~200 MB of GPU memory.
    texture: Option<TextureHandle>,
    /// Part of `texture` that is out of date.
    dirty: Option<PxRect>,
    zoom: f32,
    /// Screen offset of the image's top-left corner from the canvas' top-left corner.
    offset: Vec2,
    view_request: Option<ViewRequest>,
    unsaved: bool,
}

impl Doc {
    fn size(&self) -> Vec2 {
        vec2(self.original.width() as f32, self.original.height() as f32)
    }

    fn apply_view_request(&mut self, canvas: Rect) {
        match self.view_request.take() {
            Some(ViewRequest::Fit) => {
                let size = self.size();
                let avail = canvas.size() - vec2(32.0, 32.0);
                self.zoom = (avail.x / size.x)
                    .min(avail.y / size.y)
                    .clamp(MIN_ZOOM, 1.0);
                self.offset = (canvas.size() - size * self.zoom) / 2.0;
            }
            Some(ViewRequest::Actual) => self.zoom_at(canvas, canvas.center(), 1.0),
            Some(ViewRequest::ZoomBy(f)) => self.zoom_at(canvas, canvas.center(), self.zoom * f),
            None => {}
        }
    }

    /// Zooms keeping the image point under `anchor` (screen) fixed.
    fn zoom_at(&mut self, canvas: Rect, anchor: Pos2, new_zoom: f32) {
        let new_zoom = new_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let local = anchor - canvas.min;
        self.offset = local - (local - self.offset) * (new_zoom / self.zoom);
        self.zoom = new_zoom;
    }

    fn to_image(&self, canvas: Rect, screen: Pos2) -> Pos2 {
        ((screen - canvas.min - self.offset) / self.zoom).to_pos2()
    }

    fn to_screen(&self, canvas: Rect, image: Pos2) -> Pos2 {
        canvas.min + self.offset + image.to_vec2() * self.zoom
    }

    /// A point in a box's own frame, on screen.
    fn local_to_screen(&self, canvas: Rect, r: &Region, (u, v): (f32, f32)) -> Pos2 {
        let (x, y) = r.to_image(u, v);
        self.to_screen(canvas, pos2(x, y))
    }

    fn screen_corners(&self, canvas: Rect, r: &Region) -> [Pos2; 4] {
        r.corners().map(|(x, y)| self.to_screen(canvas, pos2(x, y)))
    }

    /// The rotate handle: above the middle of the box's top edge, following its rotation.
    fn rotate_handle(&self, canvas: Rect, r: &Region) -> (Pos2, Pos2) {
        let top = self.local_to_screen(canvas, r, (r.w / 2.0, 0.0));
        let (s, c) = r.angle.sin_cos();
        (top, top + vec2(s, -c) * ROTATE_HANDLE_OFFSET)
    }

    /// Topmost region under a screen point, with a little slack so clicking right on
    /// a box's edge (where the selection border is drawn) still hits it.
    fn region_at(&self, canvas: Rect, p: Pos2) -> Option<usize> {
        let ip = self.to_image(canvas, p);
        let slack = EDGE_SLACK / self.zoom;
        self.regions
            .iter()
            .rposition(|r| r.contains(ip.x, ip.y, slack))
    }

    /// Resize or rotate handle of the selected region under a screen-space point.
    fn handle_at(&self, canvas: Rect, p: Pos2) -> Option<(usize, Grab)> {
        let idx = self.selected?;
        let r = &self.regions[idx];
        if self.rotate_handle(canvas, r).1.distance(p) <= HANDLE_GRAB {
            return Some((idx, Grab::Rotate));
        }
        Handle::ALL
            .into_iter()
            .find(|h| self.local_to_screen(canvas, r, h.local(r)).distance(p) <= HANDLE_GRAB)
            .map(|h| (idx, Grab::Resize(h)))
    }

    fn mark_dirty(&mut self, r: PxRect) {
        self.dirty = Some(self.dirty.map_or(r, |d| d.union(r)));
    }

    fn mark_region_dirty(&mut self, idx: usize) {
        if let Some(r) = self.regions.get(idx) {
            self.mark_dirty(PxRect::of(r));
        }
    }

    fn remove_region(&mut self, idx: usize) {
        self.mark_region_dirty(idx);
        self.regions.remove(idx);
        self.selected = match self.selected {
            Some(s) if s == idx => None,
            Some(s) if s > idx => Some(s - 1),
            s => s,
        };
        self.editing = None;
        self.unsaved = true;
    }

    /// All regions burned in: this is what gets saved.
    fn rendered(&self) -> RgbaImage {
        let mut img = self.original.clone();
        redact::apply(&mut img, &self.regions);
        img
    }

    /// Regions shown in the texture (the one being dragged is drawn as an overlay).
    fn shown_regions(&self) -> impl Iterator<Item = &Region> {
        self.regions
            .iter()
            .enumerate()
            .filter(|&(i, _)| Some(i) != self.editing)
            .map(|(_, r)| r)
    }

    /// Creates the full-size texture if needed, then re-renders whatever is dirty.
    fn update_texture(&mut self, ctx: &egui::Context, max_side: usize) {
        let (w, h) = self.original.dimensions();
        let fits = (w.max(h) as usize) <= max_side;
        if self.texture.is_none() || (self.dirty.is_some() && !fits) {
            let shown: Vec<Region> = self.shown_regions().copied().collect();
            let image = if shown.is_empty() {
                color_image(&self.original, max_side) // no copy needed
            } else {
                let mut img = self.original.clone();
                redact::apply(&mut img, &shown);
                color_image(&img, max_side)
            };
            match &mut self.texture {
                Some(t) => t.set(image, TEXTURE_OPTIONS),
                None => self.texture = Some(ctx.load_texture(&self.name, image, TEXTURE_OPTIONS)),
            }
            self.dirty = None;
            return;
        }
        let (Some(dirty), Some(texture)) = (self.dirty.take(), self.texture.as_mut()) else {
            return;
        };
        let shown: Vec<Region> = self
            .regions
            .iter()
            .enumerate()
            .filter(|&(i, _)| Some(i) != self.editing)
            .map(|(_, r)| *r)
            .collect();
        if let Some((at, patch)) = render_patch(&self.original, &shown, dirty) {
            texture.set_partial(
                [at.x0 as usize, at.y0 as usize],
                egui::ColorImage::from_rgba_unmultiplied(
                    [patch.width() as usize, patch.height() as usize],
                    patch.as_raw(),
                ),
                TEXTURE_OPTIONS,
            );
        }
    }
}

/// Re-renders just the `dirty` part of the preview: the original pixels there with
/// the shown regions burned in. Returns where the patch goes and its pixels.
///
/// Pixelate depends on the whole region (block grid, noise sequence), so the patch
/// grows until every region touching it is fully inside; the result is then exactly
/// what a full re-render would give for that area.
fn render_patch(
    original: &RgbaImage,
    shown: &[Region],
    dirty: PxRect,
) -> Option<(PxRect, RgbaImage)> {
    let mut d = dirty;
    loop {
        let grown = shown
            .iter()
            .map(PxRect::of)
            .filter(|r| r.intersects(d))
            .fold(d, PxRect::union);
        if grown == d {
            break;
        }
        d = grown;
    }
    d.x1 = d.x1.min(original.width());
    d.y1 = d.y1.min(original.height());
    if d.x0 >= d.x1 || d.y0 >= d.y1 {
        return None;
    }
    let mut patch =
        image::imageops::crop_imm(original, d.x0, d.y0, d.x1 - d.x0, d.y1 - d.y0).to_image();
    let local: Vec<Region> = shown
        .iter()
        .filter(|r| PxRect::of(r).intersects(d))
        .map(|r| Region {
            cx: r.cx - d.x0 as f32,
            cy: r.cy - d.y0 as f32,
            ..*r
        })
        .collect();
    redact::apply(&mut patch, &local);
    Some((d, patch))
}

/// Converts to an egui image, downscaling if it is bigger than the GPU allows.
/// Regions stay in original-image coordinates, so this only affects the preview.
fn color_image(img: &RgbaImage, max_side: usize) -> egui::ColorImage {
    let (w, h) = img.dimensions();
    if (w.max(h) as usize) <= max_side {
        return egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
    }
    let scale = max_side as f32 / w.max(h) as f32;
    let small = image::imageops::thumbnail(
        img,
        ((w as f32 * scale) as u32).max(1),
        ((h as f32 * scale) as u32).max(1),
    );
    egui::ColorImage::from_rgba_unmultiplied(
        [small.width() as usize, small.height() as usize],
        small.as_raw(),
    )
}

/// Accent outline over a dark edge, so the selection shows on any background.
fn draw_outline(painter: &egui::Painter, corners: [Pos2; 4]) {
    painter.add(egui::Shape::closed_line(
        corners.to_vec(),
        Stroke::new(3.0, Color32::from_black_alpha(170)),
    ));
    painter.add(egui::Shape::closed_line(
        corners.to_vec(),
        Stroke::new(1.5, theme::ACCENT),
    ));
}

/// Selected box: outline, resize handles, and the rotate handle above it.
fn draw_selection(painter: &egui::Painter, doc: &Doc, canvas: Rect, r: &Region, handles: bool) {
    draw_outline(painter, doc.screen_corners(canvas, r));
    if !handles {
        return;
    }
    let (top, knob) = doc.rotate_handle(canvas, r);
    painter.line_segment([top, knob], Stroke::new(1.5, theme::ACCENT));
    painter.circle(
        knob,
        HANDLE_RADIUS + 1.0,
        theme::TEXT,
        Stroke::new(1.5, theme::ACCENT),
    );
    for h in Handle::ALL {
        let at = doc.local_to_screen(canvas, r, h.local(r));
        painter.rect(
            Rect::from_center_size(at, Vec2::splat(HANDLE_RADIUS * 2.0)),
            1.5,
            theme::TEXT,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    }
}

/// Screen color for a box's style, for previews and thumbnails.
fn style_color(style: Style, alpha: u8) -> Color32 {
    match style {
        Style::Fill(c) => {
            let [r, g, b] = c.rgb();
            Color32::from_rgba_unmultiplied(r, g, b, alpha)
        }
        Style::Pixelate => Color32::from_rgba_unmultiplied(110, 115, 125, alpha),
    }
}

/// Live preview of a region while it's drawn, moved, resized or rotated.
fn draw_live(painter: &egui::Painter, corners: [Pos2; 4], style: Style) {
    painter.add(egui::Shape::convex_polygon(
        corners.to_vec(),
        style_color(style, 170),
        Stroke::NONE,
    ));
}

fn rect_corners(r: Rect) -> [Pos2; 4] {
    [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
}

/// Small accent dot for unsaved changes.
fn unsaved_dot(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 3.0, theme::ACCENT);
    response
}

/// Small clickable color square for the Fill style.
fn color_swatch(ui: &mut egui::Ui, color: FillColor, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    let painter = ui.painter();
    let [r, g, b] = color.rgb();
    let inner = rect.shrink(3.0);
    painter.rect_filled(inner, 3.0, Color32::from_rgb(r, g, b));
    painter.rect_stroke(
        inner,
        3.0,
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Outside,
    );
    if selected {
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, theme::TEXT_MUTED),
            StrokeKind::Inside,
        );
    }
    theme::tooltip(response, color.name(), None)
}

/// A box's block size as a fraction of its image's long side.
fn block_fraction_of(r: &Region, image: &RgbaImage) -> f32 {
    r.block as f32 / image.width().max(image.height()) as f32
}

/// Placeholder row for an image that's still decoding.
fn pending_row(ui: &mut egui::Ui, p: &Pending) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let row = row.shrink2(vec2(6.0, 0.0));
    let slot = Rect::from_min_size(row.min + vec2(6.0, 5.0), vec2(52.0, 36.0));
    ui.painter().rect_filled(slot, 2.0, theme::HOVER);
    ui.new_child(
        egui::UiBuilder::new().max_rect(Rect::from_center_size(slot.center(), vec2(16.0, 16.0))),
    )
    .add(egui::Spinner::new().size(14.0));
    let text_rect = Rect::from_min_max(
        pos2(slot.right() + 10.0, row.top() + 6.0),
        pos2(row.right() - 30.0, row.bottom() - 6.0),
    );
    let mut text_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    text_ui.spacing_mut().item_spacing.y = 1.0;
    text_ui.add(
        egui::Label::new(RichText::new(&p.name).color(theme::TEXT_MUTED))
            .truncate()
            .selectable(false),
    );
    text_ui.add(
        egui::Label::new(
            RichText::new("Loading…")
                .size(11.5)
                .color(theme::TEXT_PLACEHOLDER),
        )
        .selectable(false),
    );
}

/// Shortcut list shown from the status bar.
fn shortcuts_help(ui: &mut egui::Ui) {
    ui.label(RichText::new("Shortcuts").strong().color(theme::TEXT));
    ui.add_space(4.0);
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            for (keys, what) in [
                ("Drag", "Draw a box"),
                ("Click box", "Select it"),
                ("Drag box / handles", "Move / resize"),
                ("F  /  P", "Fill / pixelate"),
                ("Drag ○ above a box", "Rotate (Shift snaps to 15°)"),
                ("⌫", "Delete box, or image in the list"),
                ("Right-click", "Delete box"),
                ("Esc", "Deselect"),
                ("⌘Z", "Undo last box"),
                ("Space + drag, scroll", "Pan"),
                ("⌘+  ⌘−  pinch", "Zoom"),
                ("0  /  1", "Fit / 100%"),
                ("↑  ↓", "Previous / next image"),
                ("⌘O  /  ⌘S", "Open / save copy"),
            ] {
                ui.label(
                    RichText::new(keys)
                        .font(egui::FontId::monospace(12.0))
                        .color(theme::TEXT_MUTED),
                );
                ui.label(RichText::new(what).color(theme::TEXT));
                ui.end_row();
            }
        });
}

/// An image that's still being decoded; shown as a placeholder row.
struct Pending {
    seq: u64,
    path: PathBuf,
    name: String,
}

pub struct StrikebarApp {
    docs: Vec<Doc>,
    current: Option<usize>,
    style: Style,
    /// Pixelate block size for new boxes, as a fraction of the image's long side.
    block_fraction: f32,
    /// Color the Fill style uses for new boxes.
    fill_color: FillColor,
    /// Size saved images are scaled down to (all saves, overwrite included).
    save_size: SaveSize,
    drag: Option<DragOp>,
    focus: Focus,
    status: String,
    loader: Loader,
    pending: Vec<Pending>,
    next_seq: u64,
    /// First image of the latest batch: selected as soon as it's ready.
    select_seq: Option<u64>,
    /// Progress of the current batch, for the status bar.
    batch_total: usize,
    batch_done: usize,
    batch_errors: Vec<String>,
    batch_started: std::time::Instant,
}

impl StrikebarApp {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Vec<PathBuf>) -> Self {
        // ⌘+ / ⌘− / ⌘0 zoom the image, not the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        theme::apply(&cc.egui_ctx);
        let mut app = Self {
            docs: Vec::new(),
            current: None,
            style: Style::Fill(FillColor::Black),
            block_fraction: redact::DEFAULT_BLOCK_FRACTION,
            fill_color: FillColor::Black,
            save_size: SaveSize::Original,
            drag: None,
            focus: Focus::Sidebar,
            status: "Open or drop images to start.".into(),
            loader: Loader::new(),
            pending: Vec::new(),
            next_seq: 0,
            select_seq: None,
            batch_total: 0,
            batch_done: 0,
            batch_errors: Vec::new(),
            batch_started: std::time::Instant::now(),
        };
        app.open_paths(&cc.egui_ctx, paths);
        app
    }

    /// Starts decoding in the background; images show up as they finish.
    fn open_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let mut jobs = Vec::new();
        for path in paths {
            if let Some(i) = self.docs.iter().position(|d| d.path == path) {
                self.select(i);
                continue;
            }
            if self.pending.iter().any(|p| p.path == path) {
                continue;
            }
            let seq = self.next_seq;
            self.next_seq += 1;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".into());
            self.pending.push(Pending {
                seq,
                path: path.clone(),
                name,
            });
            jobs.push((seq, path));
        }
        let Some(&(first, _)) = jobs.first() else {
            return;
        };
        if self.pending.len() == jobs.len() {
            // Nothing else in flight: start a fresh progress count.
            self.batch_total = 0;
            self.batch_done = 0;
            self.batch_errors.clear();
            self.batch_started = std::time::Instant::now();
        }
        self.batch_total += jobs.len();
        self.select_seq = Some(first);
        self.status = self.loading_status();
        self.loader.load(ctx, jobs);
    }

    fn loading_status(&self) -> String {
        format!("Loading {} of {}…", self.batch_done + 1, self.batch_total)
    }

    /// Moves finished images from the loader into the list.
    fn poll_loaded(&mut self, ctx: &egui::Context) {
        let results: Vec<LoadResult> = self.loader.poll().collect();
        for LoadResult { seq, path, result } in results {
            let Some(pi) = self.pending.iter().position(|p| p.seq == seq) else {
                continue; // closed while loading
            };
            let pending = self.pending.remove(pi);
            self.batch_done += 1;
            match result {
                Ok(loaded) => {
                    let thumb_image = egui::ColorImage::from_rgba_unmultiplied(
                        [
                            loaded.thumb.width() as usize,
                            loaded.thumb.height() as usize,
                        ],
                        loaded.thumb.as_raw(),
                    );
                    let thumb = ctx.load_texture(
                        format!("thumb-{}", pending.name),
                        thumb_image,
                        TextureOptions::LINEAR,
                    );
                    let pos = self.docs.partition_point(|d| d.seq < seq);
                    self.docs.insert(
                        pos,
                        Doc {
                            seq,
                            path,
                            name: pending.name,
                            original: loaded.image,
                            regions: Vec::new(),
                            selected: None,
                            editing: None,
                            thumb,
                            texture: None,
                            dirty: None,
                            zoom: 1.0,
                            offset: Vec2::ZERO,
                            view_request: Some(ViewRequest::Fit),
                            unsaved: false,
                        },
                    );
                    if let Some(c) = self.current
                        && c >= pos
                    {
                        self.current = Some(c + 1);
                    }
                    // Select the batch's first image when it arrives (not whichever
                    // finishes first), so the view doesn't jump around while loading.
                    if self.select_seq == Some(seq) {
                        self.select_seq = None;
                        self.select(pos);
                    }
                }
                Err(e) => {
                    self.batch_errors.push(format!("{}: {e}", pending.name));
                    if self.select_seq == Some(seq) {
                        // Select the next image of the batch instead.
                        self.select_seq = self
                            .pending
                            .iter()
                            .map(|p| p.seq)
                            .filter(|&s| s > seq)
                            .min();
                    }
                }
            }
            self.status = if !self.pending.is_empty() {
                self.loading_status()
            } else if let Some(e) = self.batch_errors.first() {
                format!(
                    "Couldn't open {e}{}",
                    match self.batch_errors.len() {
                        1 => String::new(),
                        n => format!(" (and {} more)", n - 1),
                    }
                )
            } else {
                let n = self.batch_done;
                format!(
                    "Opened {n} image{} in {:.1} s.",
                    if n == 1 { "" } else { "s" },
                    self.batch_started.elapsed().as_secs_f32()
                )
            };
        }
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        if let Some(paths) = FileDialog::new()
            .add_filter("Images", redact::OPEN_EXTENSIONS)
            .pick_files()
        {
            self.open_paths(ctx, paths);
        }
    }

    fn doc(&mut self) -> Option<&mut Doc> {
        self.current.and_then(|i| self.docs.get_mut(i))
    }

    fn selected_region(&self) -> Option<&Region> {
        let doc = self.docs.get(self.current?)?;
        doc.regions.get(doc.selected?)
    }

    /// Pixelate block size (px) shown in the toolbar: the selected box's, or the one
    /// new boxes on this image will get. Also returns the allowed range.
    fn block_px(&self) -> Option<(u32, (u32, u32))> {
        let doc = self.docs.get(self.current?)?;
        let (w, h) = doc.original.dimensions();
        let px = match doc.selected.and_then(|i| doc.regions.get(i)) {
            Some(r) => r.block,
            None => redact::block_for(self.block_fraction, w, h),
        };
        Some((px, redact::block_range(w, h)))
    }

    /// Changes the style (and optionally the block size, in px of the current image)
    /// for new boxes and, if a box is selected, for that box too.
    fn set_style(&mut self, style: Style, block: Option<u32>) {
        self.style = style;
        let Some(doc) = self.current.and_then(|i| self.docs.get_mut(i)) else {
            return;
        };
        let (w, h) = doc.original.dimensions();
        let (min, max) = redact::block_range(w, h);
        let block = block.map(|b| b.clamp(min, max));
        if let Some(b) = block {
            self.block_fraction = b as f32 / w.max(h) as f32;
        }
        if let Some(r) = doc.selected.and_then(|i| doc.regions.get_mut(i)) {
            let new_block = block.unwrap_or(r.block);
            if r.style != style || r.block != new_block {
                r.style = style;
                r.block = new_block;
                let changed = PxRect::of(r);
                doc.mark_dirty(changed);
                doc.unsaved = true;
            }
        }
    }

    fn undo(&mut self) {
        if let Some(doc) = self.doc()
            && !doc.regions.is_empty()
        {
            doc.remove_region(doc.regions.len() - 1);
        }
        self.drag = None;
    }

    fn clear(&mut self) {
        if let Some(doc) = self.doc() {
            if let Some(all) = doc.regions.iter().map(PxRect::of).reduce(PxRect::union) {
                doc.mark_dirty(all);
            }
            doc.regions.clear();
            doc.selected = None;
            doc.editing = None;
        }
        self.drag = None;
    }

    /// The image as it will be written: redacted, then scaled to `save_size`.
    fn output(&self, idx: usize) -> RgbaImage {
        self.save_size.apply(self.docs[idx].rendered())
    }

    /// " at 1920 × 1440" when the saved image is smaller than the original.
    fn resized_note(&self, img: &RgbaImage, idx: usize) -> String {
        if img.dimensions() == self.docs[idx].original.dimensions() {
            String::new()
        } else {
            format!(" at {} × {}", img.width(), img.height())
        }
    }

    fn save_copy(&mut self, idx: usize) {
        let out = self.output(idx);
        let note = self.resized_note(&out, idx);
        let doc = &mut self.docs[idx];
        let target = redact::copy_path(&doc.path);
        match redact::save(&out, &target) {
            Ok(()) => {
                doc.unsaved = false;
                self.status = format!("Saved {}{note}", file_name(&target));
            }
            Err(e) => self.status = format!("Couldn't save {}: {e}", target.display()),
        }
    }

    fn save_all_copies(&mut self) {
        let todo: Vec<usize> = (0..self.docs.len())
            .filter(|&i| !self.docs[i].regions.is_empty())
            .collect();
        if todo.is_empty() {
            self.status = "Nothing to save: no image has redactions.".into();
            return;
        }
        for &i in &todo {
            self.save_copy(i);
        }
        if !self.status.starts_with("Couldn't") {
            self.status = format!(
                "Saved {} redacted copies next to the originals.",
                todo.len()
            );
        }
    }

    fn overwrite(&mut self, idx: usize) {
        let (w, h) = self.docs[idx].original.dimensions();
        let (nw, nh) = self.save_size.dimensions(w, h);
        let resize = if (nw, nh) == (w, h) {
            String::new()
        } else {
            format!(", scaled down from {w} × {h} to {nw} × {nh}")
        };
        let doc = &self.docs[idx];
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title("Overwrite original?")
            .set_description(format!(
                "{} will be replaced with the redacted version{resize}. This can't be undone.",
                doc.name
            ))
            .set_buttons(MessageButtons::OkCancelCustom(
                "Overwrite".into(),
                "Cancel".into(),
            ))
            .show();
        let confirmed = match answer {
            MessageDialogResult::Ok | MessageDialogResult::Yes => true,
            MessageDialogResult::Custom(label) => label == "Overwrite",
            _ => false,
        };
        if !confirmed {
            return;
        }
        let rendered = self.output(idx);
        let note = self.resized_note(&rendered, idx);
        let doc = &mut self.docs[idx];
        match redact::save(&rendered, &doc.path) {
            Ok(()) => {
                // The redactions are now part of the pixels.
                doc.original = rendered;
                doc.regions.clear();
                doc.selected = None;
                doc.editing = None;
                doc.unsaved = false;
                // Same pixels as the preview; the texture just needs rebuilding.
                doc.texture = None;
                doc.dirty = None;
                self.status = format!("Overwrote {}{note}", doc.name);
            }
            Err(e) => self.status = format!("Couldn't overwrite {}: {e}", doc.path.display()),
        }
    }

    /// Removes an image from the list (the file on disk is not touched).
    fn close(&mut self, idx: usize) {
        let doc = &self.docs[idx];
        if doc.unsaved && !doc.regions.is_empty() {
            let answer = MessageDialog::new()
                .set_level(MessageLevel::Warning)
                .set_title("Close without saving?")
                .set_description(format!("{} has redactions that weren't saved.", doc.name))
                .set_buttons(MessageButtons::YesNo)
                .show();
            if answer != MessageDialogResult::Yes {
                return;
            }
        }
        let name = doc.name.clone();
        self.docs.remove(idx);
        self.current = match self.current {
            _ if self.docs.is_empty() => None,
            Some(c) if c > idx || c == self.docs.len() => Some(c - 1),
            c => c,
        };
        self.drag = None;
        self.status = format!("Closed {name} (file not changed).");
    }

    fn select(&mut self, idx: usize) {
        if Some(idx) != self.current {
            // Only the image on screen keeps its full-size texture.
            if let Some(old) = self.current.and_then(|c| self.docs.get_mut(c)) {
                old.texture = None;
                old.dirty = None;
                old.editing = None;
            }
            self.current = Some(idx);
            self.drag = None;
        }
        self.sync_style_from_selection();
    }

    /// Shows the selected box's style in the toolbar.
    fn sync_style_from_selection(&mut self) {
        if let Some(r) = self.selected_region().copied()
            && let Some(doc) = self.current.map(|i| &self.docs[i])
        {
            self.style = r.style;
            self.block_fraction = block_fraction_of(&r, &doc.original);
        }
    }

    fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let typing = ctx.egui_wants_keyboard_input();
        let key = |i: &mut egui::InputState, m: Modifiers, k: Key| !typing && i.consume_key(m, k);
        let mut pressed = [false; 13];
        ui.input_mut(|i| {
            pressed = [
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::S),
                key(i, Modifiers::NONE, Key::F),
                key(i, Modifiers::NONE, Key::P),
                key(i, Modifiers::NONE, Key::Num0) | key(i, Modifiers::COMMAND, Key::Num0),
                key(i, Modifiers::NONE, Key::Num1),
                key(i, Modifiers::COMMAND, Key::Plus) | key(i, Modifiers::COMMAND, Key::Equals),
                key(i, Modifiers::COMMAND, Key::Minus),
                key(i, Modifiers::NONE, Key::ArrowUp),
                key(i, Modifiers::NONE, Key::ArrowDown),
                key(i, Modifiers::NONE, Key::Backspace) | key(i, Modifiers::NONE, Key::Delete),
                key(i, Modifiers::NONE, Key::Escape),
            ];
        });
        let [
            open,
            undo,
            save,
            fill,
            pixelate,
            fit,
            actual,
            zoom_in,
            zoom_out,
            prev,
            next,
            delete,
            escape,
        ] = pressed;

        if open {
            self.pick_files(&ctx);
        }
        if undo {
            self.undo();
        }
        if save && let Some(i) = self.current {
            self.save_copy(i);
        }
        if fill {
            self.set_style(Style::Fill(self.fill_color), None);
        }
        if pixelate {
            self.set_style(Style::Pixelate, None);
        }
        let view = if fit {
            Some(ViewRequest::Fit)
        } else if actual {
            Some(ViewRequest::Actual)
        } else if zoom_in {
            Some(ViewRequest::ZoomBy(ZOOM_STEP))
        } else if zoom_out {
            Some(ViewRequest::ZoomBy(1.0 / ZOOM_STEP))
        } else {
            None
        };
        if view.is_some()
            && let Some(doc) = self.doc()
        {
            doc.view_request = view;
        }
        if let Some(c) = self.current {
            if prev && c > 0 {
                self.select(c - 1);
                self.focus = Focus::Sidebar;
            }
            if next && c + 1 < self.docs.len() {
                self.select(c + 1);
                self.focus = Focus::Sidebar;
            }
        }
        if escape && let Some(doc) = self.doc() {
            doc.selected = None;
        }
        if delete && let Some(c) = self.current {
            match self.focus {
                Focus::Canvas => {
                    let doc = &mut self.docs[c];
                    if let Some(s) = doc.selected {
                        doc.remove_region(s);
                        self.drag = None;
                    }
                }
                Focus::Sidebar => self.close(c),
            }
        }

        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.open_paths(&ctx, dropped);
        }
    }

    /// Custom title bar (macOS draws the traffic lights on top of it). Drag to move
    /// the window, double-click to zoom it, like a native one.
    fn title_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let bar = ui.interact(
            ui.max_rect(),
            ui.id().with("title_bar_drag"),
            Sense::click_and_drag(),
        );

        let has_regions = self
            .current
            .is_some_and(|i| !self.docs[i].regions.is_empty());
        let any_regions = self.docs.iter().any(|d| !d.regions.is_empty());
        let current_path = self.current.map(|i| self.docs[i].path.clone());
        let current_dims = self.current.map(|i| self.docs[i].original.dimensions());
        // HEIC/GIF can't be written back, so their copies are JPEGs and there's no overwrite.
        let can_overwrite = current_path.as_deref().is_some_and(redact::can_overwrite);
        let save_hint = current_path
            .as_deref()
            .and_then(|p| {
                redact::copy_path(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .map_or("Save redacted copy".to_owned(), |n| format!("Save as {n}"));
        let (mut save, mut save_all, mut overwrite) = (false, false, false);

        let controls = ui
            .horizontal_centered(|ui| {
                ui.add_space(theme::TRAFFIC_LIGHTS_WIDTH);
                ui.label(RichText::new("Strikebar").color(theme::TEXT));
                if let Some(doc) = self.current.map(|i| &self.docs[i]) {
                    ui.label(RichText::new("/").color(theme::TEXT_PLACEHOLDER));
                    ui.label(RichText::new(&doc.name).color(theme::TEXT_MUTED));
                    if doc.unsaved {
                        unsaved_dot(ui).on_hover_text("Unsaved redactions");
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    let primary = egui::Button::new(
                        RichText::new(theme::icon_text(Icon::Save, "Save copy")).color(theme::TEXT),
                    )
                    .fill(theme::ACCENT.gamma_multiply(0.22))
                    .stroke(Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.45)));
                    save = theme::tooltip(
                        ui.add_enabled(has_regions, primary),
                        &save_hint,
                        Some("⌘S"),
                    )
                    .clicked();
                    save_all = theme::tooltip(
                        ui.add_enabled(
                            any_regions,
                            egui::Button::new(theme::icon_text(Icon::SaveAll, "Save all")),
                        ),
                        "Save a copy of every image with boxes",
                        None,
                    )
                    .clicked();
                    overwrite = theme::tooltip(
                        ui.add_enabled(
                            has_regions && can_overwrite,
                            egui::Button::new("Overwrite…"),
                        ),
                        "Replace the original file",
                        None,
                    )
                    .on_disabled_hover_text(if has_regions && !can_overwrite {
                        "This format can't be written back. Save copy writes a JPEG."
                    } else {
                        "Add a box first"
                    })
                    .clicked();
                    self.save_size_picker(ui, current_dims);
                })
                .response
                .rect
            })
            .inner;

        // Buttons only sense clicks, so egui hands a drag that starts on them to
        // the bar underneath. Without this check a slightly shaky click on a
        // button moved the window, and a click on the size picker could drop the
        // pointer on one of its options.
        let on_controls = ctx
            .input(|i| i.pointer.press_origin())
            .is_some_and(|p| controls.expand(4.0).contains(p));
        if !on_controls {
            if bar.drag_started_by(PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if bar.double_clicked() {
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
        }

        if let Some(i) = self.current {
            if save {
                self.save_copy(i);
            }
            if overwrite {
                self.overwrite(i);
            }
        }
        if save_all {
            self.save_all_copies();
        }
    }

    /// Size presets for saving. Shows what the current image will come out as.
    fn save_size_picker(&mut self, ui: &mut egui::Ui, current: Option<(u32, u32)>) {
        let response = egui::ComboBox::from_id_salt("save_size")
            .selected_text(theme::icon_text(Icon::Scaling, &self.save_size.label()))
            .show_ui(ui, |ui| {
                for size in SaveSize::ALL {
                    // Label plus a dimmer hint ("Full HD", "web", …).
                    let mut text = egui::text::LayoutJob::default();
                    for (part, color) in [
                        (size.label(), theme::TEXT),
                        (format!("  {}", size.hint()), theme::TEXT_PLACEHOLDER),
                    ] {
                        RichText::new(part).color(color).append_to(
                            &mut text,
                            ui.style(),
                            egui::FontSelection::Default,
                            egui::Align::Center,
                        );
                    }
                    ui.selectable_value(&mut self.save_size, size, text);
                }
            })
            .response;
        let hint = match current {
            Some((w, h)) => {
                let (nw, nh) = self.save_size.dimensions(w, h);
                if (nw, nh) == (w, h) {
                    format!("Saves at {w} × {h}")
                } else {
                    format!("Saves at {nw} × {nh} (from {w} × {h})")
                }
            }
            None => "Size of saved images. Smaller images are never enlarged.".into(),
        };
        theme::tooltip(response, &hint, None);
    }

    /// Breadcrumb on the left, tools on the right (like Zed's editor toolbar).
    fn editor_toolbar(&mut self, ui: &mut egui::Ui) {
        let has_doc = self.current.is_some();
        let has_regions = self
            .current
            .is_some_and(|i| !self.docs[i].regions.is_empty());
        let editing_box = self.selected_region().is_some();
        let zoom = self.current.map(|i| self.docs[i].zoom);
        if let Style::Fill(c) = self.style {
            self.fill_color = c;
        }
        let fill_color = self.fill_color;
        let mut style = self.style;
        let block_info = self.block_px();
        let start_angle = self.selected_region().map(|r| r.angle.to_degrees().round());
        let mut angle_deg = start_angle;
        let mut block = block_info.map_or(0, |(px, _)| px);
        let mut view = None;
        let (mut undo, mut clear) = (false, false);

        ui.horizontal_centered(|ui| {
            if let Some(doc) = self.current.map(|i| &self.docs[i]) {
                if let Some(dir) = doc
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|d| d.to_string_lossy().into_owned())
                {
                    ui.label(RichText::new(dir).color(theme::TEXT_PLACEHOLDER));
                    ui.label(
                        RichText::new(Icon::ChevronRight.unicode().to_string())
                            .color(theme::TEXT_PLACEHOLDER),
                    );
                }
                ui.label(RichText::new(&doc.name).color(theme::TEXT));
            }

            // Right to left: the first widget added is the rightmost.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, Icon::Scan, has_doc, "Fit to window", Some("0")).clicked()
                {
                    view = Some(ViewRequest::Fit);
                }
                if theme::icon_button(ui, Icon::ZoomIn, has_doc, "Zoom in", Some("⌘+")).clicked()
                {
                    view = Some(ViewRequest::ZoomBy(ZOOM_STEP));
                }
                let pct = zoom.map_or("–".to_owned(), |z| format!("{:.0}%", z * 100.0));
                let pct_button = egui::Button::new(
                    RichText::new(pct)
                        .font(egui::FontId::monospace(12.0))
                        .color(theme::TEXT_MUTED),
                )
                .min_size(vec2(48.0, 22.0));
                if theme::tooltip(
                    ui.add_enabled(has_doc, pct_button),
                    "Actual size",
                    Some("1"),
                )
                .clicked()
                {
                    view = Some(ViewRequest::Actual);
                }
                if theme::icon_button(ui, Icon::ZoomOut, has_doc, "Zoom out", Some("⌘−")).clicked()
                {
                    view = Some(ViewRequest::ZoomBy(1.0 / ZOOM_STEP));
                }
                theme::divider(ui);
                clear = theme::icon_button(ui, Icon::Eraser, has_regions, "Remove all boxes", None)
                    .clicked();
                undo =
                    theme::icon_button(ui, Icon::Undo2, has_regions, "Undo last box", Some("⌘Z"))
                        .clicked();
                theme::divider(ui);

                if let Style::Fill(current) = style {
                    // Right to left, so add the swatches in reverse.
                    for color in FillColor::ALL.iter().rev() {
                        if color_swatch(ui, *color, *color == current).clicked() {
                            style = Style::Fill(*color);
                        }
                    }
                }
                if style == Style::Pixelate
                    && let Some((_, (min, max))) = block_info
                {
                    ui.label(
                        RichText::new(format!("{block}px"))
                            .font(egui::FontId::monospace(12.0))
                            .color(theme::TEXT_MUTED),
                    );
                    theme::tooltip(
                        ui.add(
                            egui::Slider::new(&mut block, min..=max)
                                .logarithmic(true)
                                .show_value(false)
                                .step_by(1.0),
                        ),
                        "Block size (scales with the image's size)",
                        None,
                    );
                }
                theme::tooltip(
                    ui.selectable_value(
                        &mut style,
                        Style::Pixelate,
                        theme::icon_text(Icon::Grid2x2, "Pixelate"),
                    ),
                    "Big blocks plus random noise",
                    Some("P"),
                );
                let fill_on = matches!(style, Style::Fill(_));
                if theme::tooltip(
                    ui.selectable_label(fill_on, theme::icon_text(Icon::PaintBucket, "Fill")),
                    "Solid color, safe for anything",
                    Some("F"),
                )
                .clicked()
                    && !fill_on
                {
                    style = Style::Fill(fill_color);
                }
                if let Some(deg) = angle_deg.as_mut() {
                    theme::divider(ui);
                    theme::tooltip(
                        ui.add(
                            egui::DragValue::new(deg)
                                .speed(1.0)
                                .range(-180.0..=180.0)
                                .max_decimals(0)
                                .suffix("°"),
                        ),
                        "Rotation. Or drag the ○ above the box (Shift snaps to 15°)",
                        None,
                    );
                    ui.label(
                        RichText::new(Icon::RotateCw.unicode().to_string())
                            .color(theme::TEXT_MUTED),
                    );
                }
                if editing_box {
                    ui.label(RichText::new("Selected box").color(theme::ACCENT));
                }
            });
        });

        let block_changed = block_info.is_some_and(|(px, _)| px != block);
        if style != self.style || block_changed {
            self.set_style(style, block_changed.then_some(block));
        }
        if angle_deg != start_angle
            && let Some(deg) = angle_deg
            && let Some(doc) = self.doc()
            && let Some(i) = doc.selected
        {
            let before = PxRect::of(&doc.regions[i]);
            doc.regions[i] = doc.regions[i].rotated_to(deg.to_radians());
            doc.mark_dirty(before);
            doc.mark_region_dirty(i);
            doc.unsaved = true;
        }
        if undo {
            self.undo();
        }
        if clear {
            self.clear();
        }
        if view.is_some()
            && let Some(doc) = self.doc()
        {
            doc.view_request = view;
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            let failed = self.status.starts_with("Couldn't");
            let (icon, color) = if failed {
                (Icon::TriangleAlert, theme::WARNING)
            } else {
                (Icon::Info, theme::TEXT_PLACEHOLDER)
            };
            ui.label(
                RichText::new(icon.unicode().to_string())
                    .size(12.0)
                    .color(color),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let shortcuts = ui.add(egui::Button::new(
                    RichText::new(Icon::Keyboard.unicode().to_string())
                        .size(14.0)
                        .color(theme::TEXT_MUTED),
                ));
                shortcuts.on_hover_ui(shortcuts_help);
                ui.add_space(8.0);
                if let Some(doc) = self.current.map(|i| &self.docs[i]) {
                    let small = |t: String| {
                        RichText::new(t)
                            .font(egui::FontId::monospace(11.5))
                            .color(theme::TEXT_MUTED)
                    };
                    ui.label(small(format!(
                        "{}×{}",
                        doc.original.width(),
                        doc.original.height()
                    )));
                    ui.add_space(14.0);
                    let n = doc.regions.len();
                    ui.label(small(format!("{n} box{}", if n == 1 { "" } else { "es" })));
                }
                // Whatever width is left goes to the message, truncated.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&self.status).size(12.0).color(if failed {
                            theme::WARNING
                        } else {
                            theme::TEXT_MUTED
                        }))
                        .truncate(),
                    );
                });
            });
        });
    }

    /// Project-panel style list: thumbnail, name, box count.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut open = false;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("IMAGES  {}", self.docs.len() + self.pending.len()))
                    .size(11.0)
                    .strong()
                    .color(theme::TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                open =
                    theme::icon_button(ui, Icon::Plus, true, "Open images", Some("⌘O")).clicked();
            });
        });

        let mut select = None;
        let mut close = None;
        let sidebar_focused = self.focus == Focus::Sidebar;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                // Loaded images and placeholders, in the order they were opened.
                let mut rows: Vec<(u64, Result<usize, usize>)> = self
                    .docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (d.seq, Ok(i)))
                    .chain(
                        self.pending
                            .iter()
                            .enumerate()
                            .map(|(j, p)| (p.seq, Err(j))),
                    )
                    .collect();
                rows.sort_by_key(|r| r.0);
                for (_, kind) in rows {
                    let i = match kind {
                        Ok(i) => i,
                        Err(j) => {
                            pending_row(ui, &self.pending[j]);
                            continue;
                        }
                    };
                    let doc = &self.docs[i];
                    let selected = self.current == Some(i);
                    let (row, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                    let row = row.shrink2(vec2(6.0, 0.0));
                    let painter = ui.painter();
                    let bg = if selected {
                        theme::SELECTED
                    } else if response.hovered() {
                        theme::HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    painter.rect_filled(row, 4.0, bg);
                    if selected && sidebar_focused {
                        painter.rect_stroke(
                            row,
                            4.0,
                            Stroke::new(1.0, theme::ACCENT),
                            StrokeKind::Inside,
                        );
                    }

                    // Thumbnail, fitted into a 52×36 box.
                    let slot = Rect::from_min_size(row.min + vec2(6.0, 5.0), vec2(52.0, 36.0));
                    let size = doc.size();
                    let thumb = Rect::from_center_size(
                        slot.center(),
                        size * (slot.width() / size.x).min(slot.height() / size.y),
                    );
                    painter.image(
                        doc.thumb.id(),
                        thumb,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    // Boxes drawn on top, so the thumbnail never needs re-rendering.
                    let scale = thumb.width() / size.x;
                    for r in &doc.regions {
                        let corners = r.corners().map(|(x, y)| thumb.min + vec2(x, y) * scale);
                        painter.add(egui::Shape::convex_polygon(
                            corners.to_vec(),
                            style_color(r.style, 255),
                            Stroke::NONE,
                        ));
                    }
                    painter.rect_stroke(
                        thumb,
                        2.0,
                        Stroke::new(1.0, theme::BORDER_VARIANT),
                        StrokeKind::Outside,
                    );

                    let text_left = slot.right() + 10.0;
                    let text_rect = Rect::from_min_max(
                        pos2(text_left, row.top() + 6.0),
                        pos2(row.right() - 30.0, row.bottom() - 6.0),
                    );
                    let n = doc.regions.len();
                    let detail = match (n, doc.unsaved) {
                        (0, _) => "No boxes".to_owned(),
                        (n, true) => format!("{n} box{} · unsaved", if n == 1 { "" } else { "es" }),
                        (n, false) => format!("{n} box{}", if n == 1 { "" } else { "es" }),
                    };
                    // Child UIs don't move the list's cursor, so rows stay where allocated.
                    let mut text_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(text_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    text_ui.spacing_mut().item_spacing.y = 1.0;
                    text_ui.add(
                        egui::Label::new(RichText::new(&doc.name).color(theme::TEXT))
                            .truncate()
                            .selectable(false),
                    );
                    text_ui.add(
                        egui::Label::new(RichText::new(detail).size(11.5).color(
                            if doc.unsaved && n > 0 {
                                theme::ACCENT
                            } else {
                                theme::TEXT_PLACEHOLDER
                            },
                        ))
                        .truncate()
                        .selectable(false),
                    );

                    if response.clicked() {
                        select = Some(i);
                    }
                    // Close button shows on hover and on the selected row.
                    if response.hovered() || selected {
                        let x = Rect::from_center_size(
                            pos2(row.right() - 16.0, row.center().y),
                            vec2(22.0, 22.0),
                        );
                        let button = egui::Button::new(
                            RichText::new(Icon::X.unicode().to_string())
                                .size(13.0)
                                .color(theme::TEXT_MUTED),
                        );
                        let r = ui
                            .new_child(egui::UiBuilder::new().max_rect(x))
                            .add(button.min_size(x.size()));
                        if theme::tooltip(r, "Remove from list", Some("⌫")).clicked() {
                            close = Some(i);
                        }
                    }
                }
            });

        if open {
            let ctx = ui.ctx().clone();
            self.pick_files(&ctx);
        }
        if let Some(i) = select {
            self.select(i);
            self.focus = Focus::Sidebar;
        }
        if let Some(i) = close {
            self.close(i);
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let canvas = response.rect;
        painter.rect_filled(canvas, 0.0, theme::EDITOR);

        if self.current.is_none() && !self.pending.is_empty() {
            let c = canvas.center();
            ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(
                c - vec2(0.0, 16.0),
                vec2(28.0, 28.0),
            )))
            .add(egui::Spinner::new().size(24.0));
            painter.text(
                c + vec2(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                format!("Loading {} images…", self.pending.len()),
                egui::FontId::proportional(14.0),
                theme::TEXT_MUTED,
            );
            return;
        }
        let Some(idx) = self.current else {
            let c = canvas.center();
            painter.text(
                c - vec2(0.0, 44.0),
                egui::Align2::CENTER_CENTER,
                Icon::Images.unicode(),
                egui::FontId::proportional(40.0),
                theme::TEXT_PLACEHOLDER.gamma_multiply(0.6),
            );
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                "No images open",
                egui::FontId::proportional(15.0),
                theme::TEXT_MUTED,
            );
            painter.text(
                c + vec2(0.0, 22.0),
                egui::Align2::CENTER_CENTER,
                "Drop images here, or open them with ⌘O",
                egui::FontId::proportional(13.0),
                theme::TEXT_PLACEHOLDER,
            );
            let button = egui::Button::new(theme::icon_text(Icon::FolderOpen, "Open images"))
                .fill(theme::HOVER);
            if ui
                .put(
                    Rect::from_center_size(c + vec2(0.0, 60.0), vec2(130.0, 26.0)),
                    button,
                )
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.pick_files(&ctx);
            }
            return;
        };
        let max_side = ui.input(|i| i.max_texture_side);
        let doc = &mut self.docs[idx];
        doc.apply_view_request(canvas);

        // Zoom: pinch or ⌘/Ctrl + scroll. Pan: two-finger scroll, middle drag, or Space + drag.
        if response.hovered() {
            let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta()));
            if zoom_delta != 1.0
                && let Some(p) = response.hover_pos()
            {
                doc.zoom_at(canvas, p, doc.zoom * zoom_delta);
            }
            doc.offset += scroll;
        }
        let space = ui.input(|i| i.key_down(Key::Space));
        let panning = response.dragged_by(PointerButton::Middle)
            || (space && response.dragged_by(PointerButton::Primary));
        if panning {
            doc.offset += response.drag_delta();
        }

        if response.clicked() || response.drag_started() || response.secondary_clicked() {
            self.focus = Focus::Canvas;
        }

        // Click: select the box under the pointer, or deselect. A fast drag can be
        // reported as a click in the same frame it starts/stops; that's not a click.
        if response.clicked()
            && !space
            && self.drag.is_none()
            && !response.drag_started()
            && !response.drag_stopped()
            && let Some(p) = response.interact_pointer_pos()
        {
            doc.selected = doc.region_at(canvas, p);
            if let Some(r) = doc.selected.map(|i| doc.regions[i]) {
                self.style = r.style;
                self.block_fraction = block_fraction_of(&r, &doc.original);
            }
        }

        // Drag start: a handle of the selected box (resize or rotate), else move the
        // box under the pointer, else draw a new box. Use where the button went down:
        // egui only reports a drag after the pointer has moved a few pixels.
        if response.drag_started_by(PointerButton::Primary)
            && !space
            && let Some(origin) = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos())
        {
            let img_origin = doc.to_image(canvas, origin);
            self.drag = match doc.handle_at(canvas, origin) {
                Some((i, Grab::Resize(handle))) => Some(DragOp::Resize {
                    idx: i,
                    handle,
                    orig: doc.regions[i],
                }),
                Some((i, Grab::Rotate)) => Some(DragOp::Rotate {
                    idx: i,
                    start: img_origin,
                    orig: doc.regions[i],
                }),
                None => {
                    if let Some(i) = doc.region_at(canvas, origin) {
                        doc.selected = Some(i);
                        self.style = doc.regions[i].style;
                        self.block_fraction = block_fraction_of(&doc.regions[i], &doc.original);
                        Some(DragOp::Move {
                            idx: i,
                            grab: img_origin,
                            orig: doc.regions[i],
                        })
                    } else {
                        doc.selected = None;
                        Some(DragOp::New { start: img_origin })
                    }
                }
            };
            if let Some(
                DragOp::Move { idx, .. } | DragOp::Resize { idx, .. } | DragOp::Rotate { idx, .. },
            ) = self.drag
            {
                doc.editing = Some(idx);
                doc.mark_region_dirty(idx);
            }
        }

        // Drag update / finish.
        let (w, h) = doc.original.dimensions();
        let shift = ui.input(|i| i.modifiers.shift);
        let mut live: Option<(Rect, Style)> = None;
        if let Some(op) = &self.drag
            && let Some(p) = response.interact_pointer_pos()
        {
            let pi = doc.to_image(canvas, p);
            match *op {
                DragOp::New { start } => {
                    if response.drag_stopped() {
                        if let Some(region) = Region::from_points(
                            (start.x, start.y),
                            (pi.x, pi.y),
                            w,
                            h,
                            self.style,
                            redact::block_for(self.block_fraction, w, h),
                        ) {
                            doc.regions.push(region);
                            doc.selected = Some(doc.regions.len() - 1);
                            doc.mark_dirty(PxRect::of(&region));
                            doc.unsaved = true;
                        }
                    } else {
                        live = Some((
                            Rect::from_two_pos(doc.to_screen(canvas, start), p),
                            self.style,
                        ));
                    }
                }
                DragOp::Move { idx, grab, orig } => {
                    doc.regions[idx] = orig.translated(pi.x - grab.x, pi.y - grab.y, w, h);
                }
                DragOp::Resize { idx, handle, orig } => {
                    // Work in the box's own frame, so this is the same for any rotation.
                    let (u, v) = orig.to_local(pi.x, pi.y);
                    let left = if handle.moves_left() { u } else { 0.0 };
                    let right = if handle.moves_right() { u } else { orig.w };
                    let top = if handle.moves_top() { v } else { 0.0 };
                    let bottom = if handle.moves_bottom() { v } else { orig.h };
                    if let Some(r) = orig.with_local_edges(left, top, right, bottom) {
                        doc.regions[idx] = r;
                    }
                }
                DragOp::Rotate { idx, start, orig } => {
                    let angle_to = |q: Pos2| (q.y - orig.cy).atan2(q.x - orig.cx);
                    let mut angle = orig.angle + angle_to(pi) - angle_to(start);
                    if shift {
                        let step = ROTATE_SNAP_DEGREES.to_radians();
                        angle = (angle / step).round() * step;
                    }
                    doc.regions[idx] = orig.rotated_to(angle);
                }
            }
        }
        if response.drag_stopped() {
            if let Some(
                DragOp::Move { idx, orig, .. }
                | DragOp::Resize { idx, orig, .. }
                | DragOp::Rotate { idx, orig, .. },
            ) = self.drag
            {
                if doc.regions[idx] != orig {
                    doc.unsaved = true;
                }
                doc.editing = None;
                // Old spot (now without the box) and new spot (with it).
                doc.mark_dirty(PxRect::of(&orig));
                doc.mark_region_dirty(idx);
            }
            self.drag = None;
        }

        // Right-click removes the topmost region under the pointer.
        if response.secondary_clicked()
            && let Some(p) = response.interact_pointer_pos()
            && let Some(i) = doc.region_at(canvas, p)
        {
            doc.remove_region(i);
            self.drag = None;
        }

        doc.update_texture(ui.ctx(), max_side);

        // Draw.
        let image_rect =
            Rect::from_min_size(doc.to_screen(canvas, Pos2::ZERO), doc.size() * doc.zoom);
        let texture_id = doc.texture.as_ref().map_or(doc.thumb.id(), |t| t.id());
        painter.image(
            texture_id,
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        if let Some(i) = doc.editing {
            let r = doc.regions[i];
            draw_live(&painter, doc.screen_corners(canvas, &r), r.style);
        }
        if let Some(i) = doc.selected {
            let r = doc.regions[i];
            draw_selection(&painter, doc, canvas, &r, doc.editing.is_none());
            if let Some(DragOp::Rotate { .. }) = self.drag {
                // Angle readout next to the rotate handle.
                let (_, knob) = doc.rotate_handle(canvas, &r);
                painter.text(
                    knob + vec2(14.0, -14.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}°", r.angle.to_degrees()),
                    egui::FontId::monospace(12.0),
                    theme::TEXT,
                );
            }
        }
        if let Some((rect, style)) = live {
            draw_live(&painter, rect_corners(rect), style);
            draw_outline(&painter, rect_corners(rect));
        }

        // Cursor.
        if response.hovered() || self.drag.is_some() {
            let pointer = response.hover_pos();
            let selected_angle = doc.selected.map_or(0.0, |i| doc.regions[i].angle);
            let icon = if space || response.dragged_by(PointerButton::Middle) {
                if response.dragged() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                }
            } else if let Some(DragOp::Resize { handle, orig, .. }) = &self.drag {
                handle.cursor(orig.angle)
            } else if matches!(self.drag, Some(DragOp::Move { .. } | DragOp::Rotate { .. })) {
                CursorIcon::Grabbing
            } else if let Some((_, grab)) = pointer.and_then(|p| doc.handle_at(canvas, p)) {
                match grab {
                    Grab::Resize(h) => h.cursor(selected_angle),
                    Grab::Rotate => CursorIcon::Grab,
                }
            } else if self.drag.is_none()
                && pointer.is_some_and(|p| doc.region_at(canvas, p).is_some())
            {
                CursorIcon::Move
            } else {
                CursorIcon::Crosshair
            };
            ui.ctx().set_cursor_icon(icon);
        }
    }
}

impl eframe::App for StrikebarApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loaded(&ctx);
        self.handle_shortcuts(ui);

        egui::Panel::top("title_bar")
            .exact_size(theme::TITLE_BAR_HEIGHT)
            .resizable(false)
            .frame(theme::frame(theme::TITLE_BAR, Margin::ZERO))
            .show(ui, |ui| self.title_bar(ui));
        egui::Panel::bottom("status_bar")
            .exact_size(theme::STATUS_BAR_HEIGHT)
            .resizable(false)
            .frame(theme::frame(theme::TITLE_BAR, Margin::symmetric(8, 0)))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("images")
            .resizable(true)
            .default_size(230.0)
            .min_size(170.0)
            .max_size(420.0)
            .frame(theme::frame(theme::PANEL, Margin::ZERO))
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Panel::top("editor_toolbar")
                .exact_size(theme::TOOLBAR_HEIGHT)
                .resizable(false)
                .frame(theme::frame(theme::EDITOR, Margin::symmetric(10, 0)))
                .show(ui, |ui| self.editor_toolbar(ui));
            self.canvas(ui);
        });
    }
}

/// Just the file name, so status messages stay short enough to show the rest.
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                (x * 3 % 256) as u8,
                (y * 7 % 256) as u8,
                ((x ^ y) % 256) as u8,
                255,
            ])
        })
    }

    fn region(a: (f32, f32), b: (f32, f32), style: Style) -> Region {
        Region::from_points(a, b, 200, 150, style, 12).unwrap()
    }

    /// Applies a patch to `preview` the way the GPU texture is updated.
    fn paste(preview: &mut RgbaImage, at: PxRect, patch: &RgbaImage) {
        image::imageops::replace(preview, patch, at.x0 as i64, at.y0 as i64);
    }

    #[test]
    fn partial_update_matches_full_render() {
        let original = photo(200, 150);
        // Overlapping pixelate boxes, so a change in one affects the other.
        let a = region((10.0, 10.0), (90.0, 70.0), Style::Pixelate);
        let b = region((60.0, 40.0), (150.0, 120.0), Style::Pixelate).rotated_to(-0.6);
        let c = region((120.0, 5.0), (190.0, 30.0), Style::Fill(FillColor::Red)).rotated_to(0.4);

        let mut before = vec![a, b, c];
        let mut preview = original.clone();
        redact::apply(&mut preview, &before);

        // Move `b`, as a drag would: dirty = old spot ∪ new spot.
        let moved = b.translated(-30.0, 15.0, 200, 150);
        let dirty = PxRect::of(&b).union(PxRect::of(&moved));
        before[1] = moved;
        let after = before;
        let (at, patch) = render_patch(&original, &after, dirty).unwrap();
        paste(&mut preview, at, &patch);

        let mut full = original.clone();
        redact::apply(&mut full, &after);
        assert_eq!(preview, full);
    }

    #[test]
    fn partial_update_after_removing_a_box() {
        let original = photo(200, 150);
        let a = region((20.0, 20.0), (120.0, 100.0), Style::Pixelate);
        let b = region((80.0, 60.0), (180.0, 140.0), Style::Fill(FillColor::Black));
        let mut preview = original.clone();
        redact::apply(&mut preview, &[a, b]);

        let (at, patch) = render_patch(&original, &[a], PxRect::of(&b)).unwrap();
        paste(&mut preview, at, &patch);

        let mut full = original.clone();
        redact::apply(&mut full, &[a]);
        assert_eq!(preview, full);
    }
}
