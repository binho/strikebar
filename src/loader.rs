//! Decodes images on background threads so the window stays responsive.
//!
//! HEIC photos from recent iPhones are 48 MP and take ~0.4 s each to decode.
//! libheif already decodes the tiles of one image on several threads, so a few
//! workers decoding different files gets most of the speed-up (measured: 14
//! photos go from 5.6 s with 1 worker to 2.1 s with 4, and barely better past that).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use image::RgbaImage;

use crate::redact;

/// Long side of the sidebar thumbnails, in pixels (2× for Retina).
pub const THUMB_SIDE: u32 = 160;

pub struct Loaded {
    pub image: RgbaImage,
    pub thumb: RgbaImage,
}

pub struct LoadResult {
    pub seq: u64,
    pub path: PathBuf,
    pub result: Result<Loaded, String>,
}

struct Queue {
    jobs: VecDeque<(u64, PathBuf)>,
    workers: usize,
}

pub struct Loader {
    queue: Arc<Mutex<Queue>>,
    tx: Sender<LoadResult>,
    rx: Receiver<LoadResult>,
    max_workers: usize,
}

impl Loader {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        // Each 48 MP decode peaks at a few hundred MB, so don't go wide.
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            queue: Arc::new(Mutex::new(Queue {
                jobs: VecDeque::new(),
                workers: 0,
            })),
            tx,
            rx,
            max_workers: (cores / 2).clamp(1, 4),
        }
    }

    /// Queues files and starts workers as needed. `ctx` is used to wake the UI
    /// when a result is ready.
    pub fn load(
        &self,
        ctx: &eframe::egui::Context,
        jobs: impl IntoIterator<Item = (u64, PathBuf)>,
    ) {
        let mut q = self.queue.lock().unwrap();
        q.jobs.extend(jobs);
        let wanted = q.jobs.len().min(self.max_workers);
        while q.workers < wanted {
            q.workers += 1;
            let queue = Arc::clone(&self.queue);
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("image-loader".into())
                .spawn(move || worker(&queue, &tx, &ctx))
                .expect("spawn loader thread");
        }
    }

    /// Results that finished since the last call.
    pub fn poll(&self) -> impl Iterator<Item = LoadResult> + '_ {
        self.rx.try_iter()
    }
}

fn worker(queue: &Mutex<Queue>, tx: &Sender<LoadResult>, ctx: &eframe::egui::Context) {
    loop {
        let job = {
            let mut q = queue.lock().unwrap();
            match q.jobs.pop_front() {
                Some(job) => job,
                None => {
                    // Decided under the lock, so `load` never misses a worker that's exiting.
                    q.workers -= 1;
                    return;
                }
            }
        };
        let (seq, path) = job;
        let result = redact::load(&path).map(|image| {
            let thumb = thumbnail(&image);
            Loaded { image, thumb }
        });
        if tx.send(LoadResult { seq, path, result }).is_err() {
            return; // app is gone
        }
        ctx.request_repaint();
    }
}

fn thumbnail(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let scale = (THUMB_SIDE as f32 / w.max(h) as f32).min(1.0);
    image::imageops::thumbnail(
        img,
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}
