# Strikebar

Small native app for redacting parts of images. Rust + egui (eframe). Runs on macOS, Windows and Linux.

## Build requirements

HEIC/HEIF support uses [libheif](https://github.com/strukturag/libheif) (via `libheif-rs`), found with `pkg-config`:

```bash
brew install libheif pkgconf        # macOS
sudo apt install libheif-dev pkg-config   # Debian/Ubuntu
```

## Run

```bash
cargo run --release -- path/to/image.png other.jpg   # paths are optional
```

### macOS app bundle

```bash
brew install cmake pkgconf      # once, to build the minimal libheif
./scripts/bundle-macos.sh       # → target/Strikebar.app
```

The bundle is self-contained (runs without Homebrew):

- `scripts/build-libheif.sh` builds a minimal libheif from the sources vendored in `libheif-sys`, with only the libde265 HEVC decoder — no x265 (GPL) or aom, which Homebrew's libheif pulls in.
- `scripts/bundle-macos.sh` builds the app against it (in `target/bundle`, so dev builds keep using Homebrew's libheif), copies `libheif` and `libde265` into `Contents/Frameworks`, relinks them via `@rpath`, adds their LGPL-3.0 licenses under `Contents/Resources/licenses`, re-signs ad hoc, and fails if anything still points outside the app.
- libde265 is built from source too, so the app runs on macOS 11 and newer. `LSMinimumSystemVersion` is set to the newest macOS any bundled binary requires.
- The icon is `assets/icon.svg`. After editing it, run `./scripts/make-icon.sh` (needs `brew install resvg`) to regenerate `assets/Strikebar.icns` and `assets/icon-256.png`, which are kept in the repo so building doesn't need resvg.
- It's signed ad hoc, not notarized: on another Mac, open it with right-click → Open the first time.

## Use

- **Open**: ⌘O / Open…, drag files onto the window, or pass paths on the command line.
- **Draw**: drag on empty space to add a box. The new box is selected.
- **Edit a box**: click it to select (outline + handles). Drag inside to move, drag a handle to resize, drag the ○ above it to rotate (Shift snaps to 15°) or type an angle in the toolbar. Pick Fill/Pixelate, a color or the block size to restyle it. Esc deselects.
- **Delete**: acts on what you clicked last. On the canvas it removes the selected box (right-click also removes a box); in the sidebar it removes the selected image from the list (the file on disk is untouched). ⌘Z undoes the last box.
- **Style**: `F` fill with a solid color (black, white, gray, red, yellow or blue; all opaque, safe for anything) or `P` pixelate (large blocks plus random noise; blocks follow the box's rotation). Block size scales with the image (default 1/80 of its long side: ~100 px on a 48 MP photo, 20 px on a 1600 px screenshot), never below 1/200 of it.
- **Navigate**: two-finger scroll, middle-drag or Space+drag to pan; pinch, ⌘/Ctrl+scroll or ⌘+ / ⌘− to zoom; `0` or ⌘0 fit, `1` 100%; ↑/↓ switch images.
- **Formats**: PNG, JPEG, WebP, BMP, TIFF, GIF and HEIC/HEIF (read only). HEIC and GIF can't be written back, so their copies are saved as `name-redacted.jpg` and *Overwrite* is disabled for them.
- **Save**: *Save copy* (⌘S) writes `name-redacted.ext` next to the original; *Save all copies* does it for every image with boxes; *Overwrite original…* replaces the file after confirming.
- **Size**: the picker next to the save buttons scales saved images down to a long side of 3840, 2560, 1920, 1280 or 800 px (default: original size). It applies to every save, overwrite included, and never enlarges smaller images; hover it to see the resulting size. Scaling happens after redacting, so redacted areas stay redacted.

## Notes on safety

- HEIC rotation (as stored by iPhones) is applied by libheif when decoding.
- Box edges are antialiased: only pixels the box partly covers are blended; pixels fully inside are always fully replaced. Straight boxes snap to whole pixels, so their edges stay crisp.
- Images decode on background threads (up to 4), and only the image on screen keeps a full-size GPU texture; edits re-render just the changed area.
- Saved files contain only pixels: EXIF, GPS and embedded thumbnails are dropped. EXIF orientation is applied on load so photos stay upright.
- The preview is rendered with the same code as the export, so what you see is what is saved.
- Blur is intentionally not offered: blurred text can often be recovered. Pixelate adds per-block random noise so it can't be reversed by guessing and comparing.

## Layout

- `src/redact.rs` — UI-free core: regions in image coordinates, apply black/pixelate, load, save, copy naming. Unit tested (`cargo test`).
- `src/app.rs` — egui UI: title bar, image list, editor toolbar, canvas with zoom/pan/selection, status bar.
- `src/theme.rs` — Zed-like look: One Dark colors, spacing, Lucide icons (`lucide-icons` crate), system UI font (SF Pro / Segoe UI when available).
