//! Redaction core: regions in image coordinates, burning them into pixels,
//! loading with EXIF orientation applied, and saving without metadata.
//!
//! Nothing in here knows about the UI, so it can be reused by another frontend.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgba, RgbaImage};

/// Solid colors for [`Style::Fill`]. All fully opaque, so any of them hides what's under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillColor {
    Black,
    White,
    Gray,
    Red,
    Yellow,
    Blue,
}

impl FillColor {
    pub const ALL: [FillColor; 6] = [
        FillColor::Black,
        FillColor::White,
        FillColor::Gray,
        FillColor::Red,
        FillColor::Yellow,
        FillColor::Blue,
    ];

    pub fn rgb(self) -> [u8; 3] {
        match self {
            FillColor::Black => [0, 0, 0],
            FillColor::White => [255, 255, 255],
            FillColor::Gray => [128, 128, 128],
            FillColor::Red => [220, 38, 38],
            FillColor::Yellow => [250, 204, 21],
            FillColor::Blue => [37, 99, 235],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FillColor::Black => "Black",
            FillColor::White => "White",
            FillColor::Gray => "Gray",
            FillColor::Red => "Red",
            FillColor::Yellow => "Yellow",
            FillColor::Blue => "Blue",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Solid color. Safe for any content.
    Fill(FillColor),
    /// Large blocks with random noise added, so it can't be undone by
    /// re-pixelating guesses and comparing.
    Pixelate,
}

/// A box in original-image pixel coordinates, possibly rotated about its center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// Center.
    pub cx: f32,
    pub cy: f32,
    /// Size before rotation.
    pub w: f32,
    pub h: f32,
    /// Rotation in radians, clockwise on screen (image y points down).
    pub angle: f32,
    pub style: Style,
    /// Block size for [`Style::Pixelate`], in image pixels. Blocks follow the rotation.
    pub block: u32,
    /// Random seed for the pixelate noise, picked when the region is created so the
    /// preview and the export match but the noise can't be predicted.
    pub seed: u64,
}

/// Smallest box side, in image pixels.
const MIN_SIDE: f32 = 2.0;

impl Region {
    /// Builds an unrotated region from two corner points in image coordinates (any
    /// order, may be outside the image). Returns `None` if nothing is left after
    /// clamping or the region is smaller than 2×2 pixels.
    pub fn from_points(
        a: (f32, f32),
        b: (f32, f32),
        image_w: u32,
        image_h: u32,
        style: Style,
        block: u32,
    ) -> Option<Self> {
        let clamp = |v: f32, max: u32| v.round().clamp(0.0, max as f32);
        let x0 = clamp(a.0.min(b.0), image_w);
        let x1 = clamp(a.0.max(b.0), image_w);
        let y0 = clamp(a.1.min(b.1), image_h);
        let y1 = clamp(a.1.max(b.1), image_h);
        if x1 - x0 < MIN_SIDE || y1 - y0 < MIN_SIDE {
            return None;
        }
        Some(Self {
            cx: (x0 + x1) / 2.0,
            cy: (y0 + y1) / 2.0,
            w: x1 - x0,
            h: y1 - y0,
            angle: 0.0,
            style,
            block: block.max(block_range(image_w, image_h).0),
            seed: random_seed(),
        })
    }

    /// Image point → this box's own frame, with (0, 0) at its (unrotated) top-left.
    pub fn to_local(self, x: f32, y: f32) -> (f32, f32) {
        let (s, c) = self.angle.sin_cos();
        let (dx, dy) = (x - self.cx, y - self.cy);
        (
            dx * c + dy * s + self.w / 2.0,
            -dx * s + dy * c + self.h / 2.0,
        )
    }

    /// This box's frame → image point (inverse of [`Self::to_local`]).
    pub fn to_image(self, u: f32, v: f32) -> (f32, f32) {
        let (s, c) = self.angle.sin_cos();
        let (lx, ly) = (u - self.w / 2.0, v - self.h / 2.0);
        (self.cx + lx * c - ly * s, self.cy + lx * s + ly * c)
    }

    /// Corners in image coordinates: top-left, top-right, bottom-right, bottom-left
    /// (of the unrotated box).
    pub fn corners(&self) -> [(f32, f32); 4] {
        [
            self.to_image(0.0, 0.0),
            self.to_image(self.w, 0.0),
            self.to_image(self.w, self.h),
            self.to_image(0.0, self.h),
        ]
    }

    /// Axis-aligned bounds `(min_x, min_y, max_x, max_y)` of the rotated box.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        self.corners().iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
        )
    }

    /// Whether an image point is inside, allowing `margin` pixels of slack.
    pub fn contains(&self, x: f32, y: f32, margin: f32) -> bool {
        let (u, v) = self.to_local(x, y);
        u >= -margin && u < self.w + margin && v >= -margin && v < self.h + margin
    }

    /// Moves by `(dx, dy)` image pixels, keeping the box inside the image when it fits.
    pub fn translated(&self, dx: f32, dy: f32, image_w: u32, image_h: u32) -> Self {
        let (x0, y0, x1, y1) = self.bounds();
        let (ex, ey) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let keep = |v: f32, extent: f32, max: u32| {
            let max = max as f32;
            if 2.0 * extent >= max {
                max / 2.0
            } else {
                v.clamp(extent, max - extent)
            }
        };
        let (mut cx, mut cy) = (
            keep(self.cx + dx, ex, image_w),
            keep(self.cy + dy, ey, image_h),
        );
        if self.angle == 0.0 {
            // Keep straight boxes on whole pixels, so their edges stay crisp.
            cx = (cx - self.w / 2.0).round() + self.w / 2.0;
            cy = (cy - self.h / 2.0).round() + self.h / 2.0;
        }
        Self { cx, cy, ..*self }
    }

    /// Resizes by moving edges, given in this box's frame (see [`Self::to_local`]).
    /// The box keeps its angle; its center moves so untouched edges stay put.
    /// `None` if it would be smaller than 2×2.
    pub fn with_local_edges(&self, left: f32, top: f32, right: f32, bottom: f32) -> Option<Self> {
        // Straight boxes snap to whole pixels, so their edges stay crisp.
        let snap = |v: f32| if self.angle == 0.0 { v.round() } else { v };
        let (left, top, right, bottom) = (snap(left), snap(top), snap(right), snap(bottom));
        let (l, r) = (left.min(right), left.max(right));
        let (t, b) = (top.min(bottom), top.max(bottom));
        if r - l < MIN_SIDE || b - t < MIN_SIDE {
            return None;
        }
        let (cx, cy) = self.to_image((l + r) / 2.0, (t + b) / 2.0);
        Some(Self {
            cx,
            cy,
            w: r - l,
            h: b - t,
            ..*self
        })
    }

    pub fn rotated_to(&self, angle: f32) -> Self {
        let tau = std::f32::consts::TAU;
        // Keep it in (-180°, 180°].
        let mut a = angle.rem_euclid(tau);
        if a > std::f32::consts::PI {
            a -= tau;
        }
        Self { angle: a, ..*self }
    }

    /// Pixels drawing this box can change, `(x0, y0, x1, y1)` with exclusive end and
    /// not clamped to the image: its bounds plus one pixel for the antialiased edge.
    pub fn affected_pixels(&self) -> (i64, i64, i64, i64) {
        let (a, b, c, d) = self.bounds();
        (
            a.floor() as i64 - 1,
            b.floor() as i64 - 1,
            c.ceil() as i64 + 1,
            d.ceil() as i64 + 1,
        )
    }

    /// [`Self::affected_pixels`], clamped to the image.
    fn pixel_bounds(&self, image_w: u32, image_h: u32) -> Option<(u32, u32, u32, u32)> {
        let (a, b, c, d) = self.affected_pixels();
        let x0 = a.max(0) as u32;
        let y0 = b.max(0) as u32;
        let x1 = c.clamp(0, image_w as i64) as u32;
        let y1 = d.clamp(0, image_h as i64) as u32;
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    /// How much of pixel `(x, y)` the box covers, 0..=1, for antialiased edges.
    ///
    /// Uses the distance from the pixel center to each pair of sides, scaled by the
    /// pixel's width across a rotated side (1 when straight, √2 at 45°). Straight,
    /// whole-pixel boxes come out exactly 0 or 1, so they stay pixel-crisp.
    fn coverage(&self, x: u32, y: u32, footprint: f32) -> f32 {
        let (u, v) = self.to_local(x as f32 + 0.5, y as f32 + 0.5);
        let du = u.min(self.w - u);
        let dv = v.min(self.h - v);
        (0.5 + du / footprint).clamp(0.0, 1.0) * (0.5 + dv / footprint).clamp(0.0, 1.0)
    }

    fn footprint(&self) -> f32 {
        let (s, c) = self.angle.sin_cos();
        s.abs() + c.abs()
    }
}

/// Smaller blocks leak too much detail to count as redaction, whatever the image size.
pub const MIN_BLOCK: u32 = 8;

/// Pixelate block sizes are relative to the image's long side, so a box hides the
/// same amount of detail on a 2 MP screenshot and a 48 MP phone photo (a fixed
/// 24 px block is only ~0.3% of an 8064 px photo and leaves shapes recognizable).
const MIN_BLOCK_FRACTION: f32 = 1.0 / 200.0;
const MAX_BLOCK_FRACTION: f32 = 1.0 / 16.0;
/// Default block: 1/80 of the long side (≈100 px on a 48 MP photo, 20 px on 1600 px).
pub const DEFAULT_BLOCK_FRACTION: f32 = 1.0 / 80.0;

/// Allowed pixelate block sizes, in pixels, for an image of this size.
pub fn block_range(image_w: u32, image_h: u32) -> (u32, u32) {
    let long = image_w.max(image_h) as f32;
    let min = ((long * MIN_BLOCK_FRACTION).round() as u32).max(MIN_BLOCK);
    let max = ((long * MAX_BLOCK_FRACTION).round() as u32).max(min);
    (min, max)
}

/// Block size in pixels for a fraction of the image's long side, within [`block_range`].
pub fn block_for(fraction: f32, image_w: u32, image_h: u32) -> u32 {
    let (min, max) = block_range(image_w, image_h);
    ((image_w.max(image_h) as f32 * fraction).round() as u32).clamp(min, max)
}

const NOISE_AMPLITUDE: i32 = 28;

/// Burns all regions into `img`, in order. Edges are antialiased: pixels the box
/// only partly covers are blended with what's under them. Pixels fully inside are
/// always fully replaced.
pub fn apply(img: &mut RgbaImage, regions: &[Region]) {
    for r in regions {
        let Some(bounds) = r.pixel_bounds(img.width(), img.height()) else {
            continue;
        };
        match r.style {
            Style::Fill(color) => fill(img, r, bounds, color.rgb()),
            Style::Pixelate => pixelate(img, r, bounds),
        }
    }
}

/// `under` blended toward `over` by `amount` (0..=1).
fn blend(under: [u8; 4], over: [u8; 3], amount: f32) -> [u8; 4] {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    [
        mix(under[0], over[0]),
        mix(under[1], over[1]),
        mix(under[2], over[2]),
        under[3].max((amount * 255.0) as u8),
    ]
}

fn fill(img: &mut RgbaImage, r: &Region, (x0, y0, x1, y1): (u32, u32, u32, u32), rgb: [u8; 3]) {
    let footprint = r.footprint();
    for y in y0..y1 {
        for x in x0..x1 {
            let c = r.coverage(x, y, footprint);
            if c >= 1.0 {
                img.put_pixel(x, y, Rgba([rgb[0], rgb[1], rgb[2], 255]));
            } else if c > 0.0 {
                let under = img.get_pixel(x, y).0;
                img.put_pixel(x, y, Rgba(blend(under, rgb, c)));
            }
        }
    }
}

/// Averages each block (in the box's own, possibly rotated, frame) over the image
/// pixels whose center is inside the box, adds per-block noise, and writes it back,
/// blending the partly covered edge pixels.
fn pixelate(img: &mut RgbaImage, r: &Region, (x0, y0, x1, y1): (u32, u32, u32, u32)) {
    let block = r.block.max(MIN_BLOCK) as f32;
    let cols = (r.w / block).ceil().max(1.0) as usize;
    let rows = (r.h / block).ceil().max(1.0) as usize;
    // Nearest block for a pixel, also for edge pixels just outside the box.
    let block_index = |u: f32, v: f32| -> usize {
        let c = ((u / block).floor().max(0.0) as usize).min(cols - 1);
        let row = ((v / block).floor().max(0.0) as usize).min(rows - 1);
        row * cols + c
    };

    let mut sums = vec![[0u64; 4]; cols * rows];
    for y in y0..y1 {
        for x in x0..x1 {
            let (u, v) = r.to_local(x as f32 + 0.5, y as f32 + 0.5);
            if u >= 0.0 && v >= 0.0 && u < r.w && v < r.h {
                let p = img.get_pixel(x, y).0;
                let s = &mut sums[block_index(u, v)];
                for ch in 0..3 {
                    s[ch] += p[ch] as u64;
                }
                s[3] += 1;
            }
        }
    }
    let colors: Vec<Option<[u8; 3]>> = sums
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if s[3] == 0 {
                return None; // block entirely outside the image
            }
            // Noise depends only on the seed and the block, so it's stable while
            // other boxes change and when re-rendering just part of the image.
            let mut rng = SplitMix64(r.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut out = [0u8; 3];
            for ch in 0..3 {
                let noise =
                    (rng.next() % (2 * NOISE_AMPLITUDE as u64 + 1)) as i32 - NOISE_AMPLITUDE;
                out[ch] = ((s[ch] / s[3]) as i32 + noise).clamp(0, 255) as u8;
            }
            Some(out)
        })
        .collect();
    let footprint = r.footprint();
    for y in y0..y1 {
        for x in x0..x1 {
            let c = r.coverage(x, y, footprint);
            if c <= 0.0 {
                continue;
            }
            let (u, v) = r.to_local(x as f32 + 0.5, y as f32 + 0.5);
            let Some(rgb) = colors[block_index(u, v)] else {
                continue;
            };
            let px = if c >= 1.0 {
                [rgb[0], rgb[1], rgb[2], 255]
            } else {
                blend(img.get_pixel(x, y).0, rgb, c)
            };
            img.put_pixel(x, y, Rgba(px));
        }
    }
}

/// File extensions the app can open, for the file dialog.
pub const OPEN_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "heic", "heif", "hif",
];

/// Formats we write. Anything else (GIF, HEIC, …) is saved as a JPEG copy instead.
fn writable(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::WebP
            | ImageFormat::Tiff
            | ImageFormat::Bmp
    )
}

/// Whether "overwrite original" can keep the file's own format.
pub fn can_overwrite(path: &Path) -> bool {
    ImageFormat::from_path(path).is_ok_and(writable)
}

/// HEIC/HEIF is decoded by libheif through the `image` crate's decoder hooks, so
/// the normal load path (format sniffing, 8/10-bit) works for it. Rotation stored
/// in the HEIC container is applied by libheif while decoding.
fn register_heif() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(libheif_rs::integration::image::register_all_decoding_hooks);
}

/// Loads an image and applies its EXIF orientation, so photos look upright
/// even after the metadata is stripped on save.
pub fn load(path: &Path) -> Result<RgbaImage, String> {
    register_heif();
    let mut decoder = ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(img.into_rgba8())
}

/// Size of saved images. Resizing happens after redacting, and downscaling
/// only averages pixels together, so redacted areas stay redacted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SaveSize {
    #[default]
    Original,
    /// Longest side in pixels. Smaller images are left as they are.
    LongSide(u32),
}

impl SaveSize {
    pub const ALL: [SaveSize; 6] = [
        SaveSize::Original,
        SaveSize::LongSide(3840),
        SaveSize::LongSide(2560),
        SaveSize::LongSide(1920),
        SaveSize::LongSide(1280),
        SaveSize::LongSide(800),
    ];

    pub fn label(self) -> String {
        match self {
            SaveSize::Original => "Original size".into(),
            SaveSize::LongSide(px) => format!("{px} px"),
        }
    }

    /// What the preset is good for, shown next to its label.
    pub fn hint(self) -> &'static str {
        match self {
            SaveSize::LongSide(3840) => "4K",
            SaveSize::LongSide(1920) => "Full HD",
            SaveSize::LongSide(1280) => "web",
            SaveSize::LongSide(800) => "chat",
            _ => "",
        }
    }

    /// Output size for a `w`×`h` image. Never upscales.
    pub fn dimensions(self, w: u32, h: u32) -> (u32, u32) {
        let SaveSize::LongSide(max) = self else {
            return (w, h);
        };
        let long = w.max(h);
        if long <= max {
            return (w, h);
        }
        let scale = |side: u32| {
            if side == long {
                max
            } else {
                ((u64::from(side) * u64::from(max) + u64::from(long) / 2) / u64::from(long)).max(1)
                    as u32
            }
        };
        (scale(w), scale(h))
    }

    pub fn apply(self, img: RgbaImage) -> RgbaImage {
        let (w, h) = img.dimensions();
        let (nw, nh) = self.dimensions(w, h);
        if (nw, nh) == (w, h) {
            return img;
        }
        image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3)
    }
}

/// Encodes `img` to `path`, picking the format from the extension.
/// Only pixels are written: no EXIF, GPS or embedded thumbnails.
/// Writes to a temporary file first and renames it, so a failed save never
/// leaves a half-written file behind (matters when overwriting the original).
pub fn save(img: &RgbaImage, path: &Path) -> Result<(), String> {
    let format = ImageFormat::from_path(path)
        .ok()
        .filter(|f| writable(*f))
        .ok_or_else(|| format!("can't write {}", path.display()))?;
    let tmp = path.with_extension(format!(
        "{}.strikebar-tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("img")
    ));
    let result = (|| -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut w = BufWriter::new(file);
        if format == ImageFormat::Jpeg {
            let rgb = DynamicImage::ImageRgba8(img.clone()).into_rgb8();
            JpegEncoder::new_with_quality(&mut w, 92)
                .encode_image(&rgb)
                .map_err(|e| e.to_string())?;
        } else {
            img.write_to(&mut w, format).map_err(|e| e.to_string())?;
        }
        drop(w);
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `photo.jpg` → `photo-redacted.jpg`, then `photo-redacted-2.jpg`, … if taken.
/// Formats we can't write (HEIC, GIF) get a `.jpg` copy.
pub fn copy_path(original: &Path) -> PathBuf {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| ImageFormat::from_extension(e).is_some_and(writable))
        .unwrap_or("jpg");
    let dir = original.parent().unwrap_or(Path::new("."));
    let mut candidate = dir.join(format!("{stem}-redacted.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-redacted-{n}.{ext}"));
        n += 1;
    }
    candidate
}

fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // RandomState is seeded from the OS RNG once per process.
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.finish()
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 7 % 256) as u8, (y * 5 % 256) as u8, 128, 255])
        })
    }

    const BLACK: Style = Style::Fill(FillColor::Black);

    fn rect(r: &Region) -> (f32, f32, f32, f32) {
        (
            r.cx - r.w / 2.0,
            r.cy - r.h / 2.0,
            r.cx + r.w / 2.0,
            r.cy + r.h / 2.0,
        )
    }

    #[test]
    fn from_points_normalizes_and_clamps() {
        let r = Region::from_points((120.0, 50.0), (-10.0, 10.0), 100, 80, BLACK, 16).unwrap();
        assert_eq!(rect(&r), (0.0, 10.0, 100.0, 50.0));
        assert_eq!(r.angle, 0.0);
    }

    #[test]
    fn from_points_rejects_tiny_or_outside() {
        assert!(Region::from_points((5.0, 5.0), (6.0, 30.0), 100, 100, BLACK, 16).is_none());
        assert!(Region::from_points((200.0, 200.0), (300.0, 300.0), 100, 100, BLACK, 16).is_none());
    }

    #[test]
    fn fill_colors_only_inside() {
        let orig = gradient(50, 50);
        for color in FillColor::ALL {
            let mut img = orig.clone();
            let style = Style::Fill(color);
            let r = Region::from_points((10.0, 10.0), (20.0, 30.0), 50, 50, style, 16).unwrap();
            apply(&mut img, &[r]);
            let [cr, cg, cb] = color.rgb();
            for (x, y, p) in img.enumerate_pixels() {
                if (10..20).contains(&x) && (10..30).contains(&y) {
                    assert_eq!(p.0, [cr, cg, cb, 255]);
                } else {
                    assert_eq!(p, orig.get_pixel(x, y));
                }
            }
        }
    }

    #[test]
    fn rotated_fill_covers_a_diamond() {
        // A 40×40 square at (50, 50) turned 45° becomes a diamond.
        let mut img = gradient(100, 100);
        let r = Region::from_points((30.0, 30.0), (70.0, 70.0), 100, 100, BLACK, 16)
            .unwrap()
            .rotated_to(std::f32::consts::FRAC_PI_4);
        apply(&mut img, &[r]);
        let black = |x, y| img.get_pixel(x, y).0 == [0, 0, 0, 255];
        assert!(black(50, 50));
        assert!(black(50, 25)); // top tip (half-diagonal ≈ 28)
        assert!(black(24, 50)); // left tip
        assert!(!black(32, 32)); // the old corner is now outside
        assert!(!black(68, 68));
    }

    #[test]
    fn pixelate_changes_inside_and_is_blocky() {
        let orig = gradient(64, 64);
        let mut img = orig.clone();
        let r = Region::from_points((0.0, 0.0), (32.0, 32.0), 64, 64, Style::Pixelate, 16).unwrap();
        apply(&mut img, &[r]);
        // Every pixel in a 16×16 block is the same.
        let first = *img.get_pixel(0, 0);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(*img.get_pixel(x, y), first);
            }
        }
        assert_ne!(img.get_pixel(0, 0), orig.get_pixel(0, 0));
        assert_eq!(img.get_pixel(40, 40), orig.get_pixel(40, 40));
    }

    #[test]
    fn pixelate_is_deterministic_per_region() {
        let r = Region::from_points((0.0, 0.0), (32.0, 32.0), 64, 64, Style::Pixelate, 8)
            .unwrap()
            .rotated_to(0.3);
        let mut a = gradient(64, 64);
        let mut b = gradient(64, 64);
        apply(&mut a, &[r]);
        apply(&mut b, &[r]);
        assert_eq!(a, b);
    }

    #[test]
    fn local_and_image_coordinates_round_trip() {
        let r = Region::from_points((10.0, 20.0), (70.0, 50.0), 100, 100, BLACK, 16)
            .unwrap()
            .rotated_to(0.7);
        for (x, y) in [(40.0, 35.0), (12.5, 48.0), (90.0, 3.0)] {
            let (u, v) = r.to_local(x, y);
            let (bx, by) = r.to_image(u, v);
            assert!((bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3);
        }
    }

    #[test]
    fn resizing_a_rotated_box_keeps_the_opposite_edge() {
        let r = Region::from_points((20.0, 20.0), (60.0, 40.0), 100, 100, Style::Pixelate, 12)
            .unwrap()
            .rotated_to(0.5);
        let left_mid = r.to_image(0.0, r.h / 2.0);
        // Drag the right edge 10 px further out (in the box's frame).
        let wider = r.with_local_edges(0.0, 0.0, r.w + 10.0, r.h).unwrap();
        assert!((wider.w - 50.0).abs() < 1e-3 && (wider.h - 20.0).abs() < 1e-3);
        let still = wider.to_image(0.0, wider.h / 2.0);
        assert!((still.0 - left_mid.0).abs() < 1e-3 && (still.1 - left_mid.1).abs() < 1e-3);
        assert_eq!(
            (wider.angle, wider.style, wider.block, wider.seed),
            (r.angle, r.style, r.block, r.seed)
        );
        assert!(r.with_local_edges(0.0, 0.0, 1.0, r.h).is_none());
    }

    #[test]
    fn translated_keeps_size_and_stops_at_edges() {
        let r = Region::from_points((10.0, 10.0), (30.0, 40.0), 100, 100, BLACK, 16).unwrap();
        let moved = r.translated(5.0, -3.0, 100, 100);
        assert_eq!(rect(&moved), (15.0, 7.0, 35.0, 37.0));
        let far = r.translated(500.0, -500.0, 100, 100);
        assert_eq!(rect(&far), (80.0, 0.0, 100.0, 30.0));
        assert_eq!(far.seed, r.seed);
    }

    #[test]
    fn rotation_is_normalized() {
        let r = Region::from_points((10.0, 10.0), (30.0, 40.0), 100, 100, BLACK, 16).unwrap();
        let a = r.rotated_to(3.0 * std::f32::consts::PI / 2.0).angle;
        assert!((a + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn save_strips_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("strikebar-test-{}", random_seed()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = gradient(40, 30);
        for ext in ["png", "jpg"] {
            let p = dir.join(format!("out.{ext}"));
            save(&img, &p).unwrap();
            let back = load(&p).unwrap();
            assert_eq!(back.dimensions(), (40, 30));
        }
        let copy = copy_path(&dir.join("out.png"));
        assert_eq!(copy.file_name().unwrap(), "out-redacted.png");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Which channel dominates: 0 = red, 1 = green, 2 = blue.
    fn dominant(img: &RgbaImage, x: u32, y: u32) -> usize {
        let p = img.get_pixel(x, y).0;
        (0..3).max_by_key(|&c| p[c]).unwrap()
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn loads_heic() {
        // 64×48: red | blue on top, green strip at the bottom.
        let img = load(&fixture("colors.heic")).unwrap();
        assert_eq!(img.dimensions(), (64, 48));
        assert_eq!(dominant(&img, 8, 8), 0);
        assert_eq!(dominant(&img, 56, 8), 2);
        assert_eq!(dominant(&img, 32, 44), 1);
    }

    #[test]
    fn heic_rotation_is_applied_once() {
        // Same picture, stored with a 90° clockwise rotation in the container
        // (like iPhone photos): should come out 48×64 and rotated.
        let img = load(&fixture("rotated.heic")).unwrap();
        assert_eq!(img.dimensions(), (48, 64));
        assert_eq!(dominant(&img, 40, 8), 0);
        assert_eq!(dominant(&img, 40, 56), 2);
        assert_eq!(dominant(&img, 4, 32), 1);
    }

    #[test]
    fn unwritable_formats_get_jpeg_copies_and_no_overwrite() {
        let heic = Path::new("/nowhere/IMG_0001.HEIC");
        assert_eq!(
            copy_path(heic).file_name().unwrap(),
            "IMG_0001-redacted.jpg"
        );
        assert!(!can_overwrite(heic));
        assert_eq!(
            copy_path(Path::new("/nowhere/a.gif")).file_name().unwrap(),
            "a-redacted.jpg"
        );
        assert!(!can_overwrite(Path::new("/nowhere/a.gif")));
        assert!(can_overwrite(Path::new("/nowhere/a.png")));
        assert!(can_overwrite(Path::new("/nowhere/a.JPG")));
    }

    #[test]
    fn save_refuses_unwritable_formats() {
        let img = gradient(4, 4);
        let dir = std::env::temp_dir().join(format!("strikebar-test-{}", random_seed()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(save(&img, &dir.join("x.heic")).is_err());
        assert!(save(&img, &dir.join("x.gif")).is_err());
        assert!(
            std::fs::read_dir(&dir).unwrap().next().is_none(),
            "no temp files left behind"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_size_scales_long_side_and_never_upscales() {
        let s = SaveSize::LongSide(1920);
        assert_eq!(s.dimensions(8064, 6048), (1920, 1440));
        assert_eq!(s.dimensions(6048, 8064), (1440, 1920));
        assert_eq!(s.dimensions(1600, 900), (1600, 900));
        assert_eq!(s.dimensions(4000, 4000), (1920, 1920));
        assert_eq!(s.dimensions(40000, 10), (1920, 1));
        assert_eq!(SaveSize::Original.dimensions(8064, 6048), (8064, 6048));
    }

    #[test]
    fn save_size_keeps_redaction_black() {
        let mut img = gradient(400, 300);
        let r = Region::from_points(
            (100.0, 100.0),
            (300.0, 200.0),
            400,
            300,
            Style::Fill(FillColor::Black),
            8,
        )
        .unwrap();
        apply(&mut img, &[r]);
        let small = SaveSize::LongSide(100).apply(img);
        assert_eq!(small.dimensions(), (100, 75));
        // Inside the box (well away from its edges) everything is still black.
        for y in 30..45 {
            for x in 30..70 {
                assert_eq!(small.get_pixel(x, y).0[..3], [0, 0, 0], "at {x},{y}");
            }
        }
    }

    #[test]
    fn resized_save_roundtrips() {
        let dir = std::env::temp_dir().join(format!("strikebar-test-{}", random_seed()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("small.jpg");
        save(&SaveSize::LongSide(320).apply(gradient(1280, 960)), &path).unwrap();
        assert_eq!(load(&path).unwrap().dimensions(), (320, 240));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Decode throughput on real photos with 1..14 workers (see `loader.rs`):
    /// STRIKEBAR_BENCH_DIR=~/some/photos cargo test --release bench_parallel -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_parallel() {
        use std::time::Instant;
        let Ok(dir) = std::env::var("STRIKEBAR_BENCH_DIR") else {
            return;
        };
        let files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        for workers in [1usize, 2, 4, 6, 8, 14] {
            let t = Instant::now();
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| {
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(f) = files.get(i) else { break };
                            std::hint::black_box(load(f).unwrap());
                        }
                    });
                }
            });
            println!("workers {workers:>2}: {:.2}s", t.elapsed().as_secs_f64());
        }
    }

    #[test]
    fn block_size_scales_with_image() {
        // 48 MP phone photo vs. a small screenshot: same share of the image.
        assert_eq!(block_for(DEFAULT_BLOCK_FRACTION, 8064, 6048), 101);
        assert_eq!(block_for(DEFAULT_BLOCK_FRACTION, 1600, 1000), 20);
        // Never below the floor, never above 1/16 of the long side.
        assert_eq!(block_range(8064, 6048), (40, 504));
        assert_eq!(block_range(300, 200), (MIN_BLOCK, 19));
        assert_eq!(block_for(0.0, 8064, 6048), 40);
        assert_eq!(block_for(1.0, 8064, 6048), 504);
    }

    #[test]
    fn regions_get_at_least_the_minimum_block_for_their_image() {
        let r = Region::from_points((0.0, 0.0), (500.0, 500.0), 8064, 6048, Style::Pixelate, 24)
            .unwrap();
        assert_eq!(r.block, 40);
    }

    #[test]
    fn rotated_edges_are_antialiased() {
        let orig = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        let mut img = orig.clone();
        let r = Region::from_points((30.0, 30.0), (70.0, 70.0), 100, 100, BLACK, 16)
            .unwrap()
            .rotated_to(0.35);
        apply(&mut img, &[r]);
        let gray = img.pixels().filter(|p| p.0[0] > 0 && p.0[0] < 255).count();
        assert!(gray > 40, "expected blended edge pixels, got {gray}");
        // Inside stays solid, far outside untouched.
        assert_eq!(img.get_pixel(50, 50).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255, 255]);
    }

    #[test]
    fn straight_boxes_stay_crisp_after_moving_and_resizing() {
        let orig = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        let r = Region::from_points((10.0, 10.0), (31.0, 40.0), 100, 100, BLACK, 16).unwrap();
        let r = r.translated(3.3, 4.7, 100, 100);
        let r = r.with_local_edges(0.0, 0.0, r.w + 6.4, r.h - 2.2).unwrap();
        let mut img = orig.clone();
        apply(&mut img, &[r]);
        assert!(img.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }
}
