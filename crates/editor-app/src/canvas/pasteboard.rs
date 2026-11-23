//! Visible vector tiles for the pasteboard and high-zoom artboard.
use super::display_filter::{self, Image, ARTWORK_TEXTURE};
use super::{egui, Color32, FrameTexture, Rect, TextureHandle, View};
use document_core::Document;
use renderer::pasteboard::{Damage, Patch, Scene, Tile, TEXTURE, TILE};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Pasteboard {
    on_page: bool,
    scene: Option<Scene>,
    source: std::sync::Weak<Document>,
    tiles: BTreeMap<Tile, Option<TextureHandle>>,
    previous: Option<(Grid, BTreeMap<Tile, Option<TextureHandle>>)>,
    job: Option<Job>,
    error: Option<String>,
}

struct Job {
    key: Tile,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Patch, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Job {
    fn start(
        ctx: &egui::Context,
        key: Tile,
        paint: impl FnOnce(&AtomicBool) -> Result<Patch, String> + Send + 'static,
    ) -> Option<Self> {
        let ctx = ctx.clone();
        let viewport = ctx.viewport_id();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::Builder::new()
            .name("phyro-pasteboard".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| paint(&stop)))
                        .unwrap_or_else(|_| {
                            Err("The pasteboard renderer stopped unexpectedly.".into())
                        });
                if tx.send(result).is_ok() {
                    ctx.request_repaint_of(viewport);
                }
            })
            .ok()?;
        Some(Self { key, cancel, rx })
    }
}

#[derive(Clone, Copy)]
struct Grid {
    scale: [f32; 2],
}
impl Grid {
    fn range(self, view: View, rect: Rect, limit: i64) -> Option<([i64; 2], [i64; 2])> {
        let cell = |p| {
            let q = view.world(p);
            [
                (f64::from(q.x) * f64::from(self.scale[0]) / f64::from(TILE)).floor() as i64,
                (f64::from(q.y) * f64::from(self.scale[1]) / f64::from(TILE)).floor() as i64,
            ]
        };
        let a = cell(rect.min);
        let b = cell(rect.max);
        // The texture budget also bounds work in extremely thin native windows.
        let cols = b[0].checked_sub(a[0])?.checked_add(1)?;
        let rows = b[1].checked_sub(a[1])?.checked_add(1)?;
        (cols.checked_mul(rows)? <= limit && cols > 0 && rows > 0).then_some((a, b))
    }
    fn rect(self, view: View, [x, y]: Tile) -> Rect {
        let point = |x: f64, y: f64| {
            view.screen(egui::pos2(
                (x * f64::from(TILE) / f64::from(self.scale[0])) as f32,
                (y * f64::from(TILE) / f64::from(self.scale[1])) as f32,
            ))
        };
        Rect::from_min_max(
            point(x as f64, y as f64),
            point(x as f64 + 1., y as f64 + 1.),
        )
    }
    fn image(self, view: View, key: Tile, tile: &TextureHandle, clip: Rect) -> Option<Image> {
        let dst = self.rect(view, key);
        let part = dst.intersect(clip);
        if !part.is_positive() || !dst.is_positive() {
            return None;
        }
        let uv = |p: egui::Pos2| {
            let t = (p - dst.min) / dst.size();
            egui::pos2(
                (1. + t.x * TILE as f32) / TEXTURE as f32,
                (1. + t.y * TILE as f32) / TEXTURE as f32,
            )
        };
        Some(Image::rectangular(
            tile.id(),
            part,
            Rect::from_min_max(uv(part.min), uv(part.max)),
            Color32::WHITE,
        ))
    }
}

// High-resolution monitors still use a bounded number of outside tiles.
fn tile_grid(
    view: View,
    viewport: Rect,
    dim: [u32; 2],
    density: f32,
    on_page: bool,
) -> Option<([u32; 2], (Tile, Tile))> {
    let mut pixels = dim.map(|v| (f64::from(v) * f64::from(density)).round().max(1.) as u32);
    loop {
        let grid = Grid {
            scale: std::array::from_fn(|i| pixels[i] as f32 / dim[i] as f32),
        };
        if let Some(span) = grid.range(view, viewport, if on_page { 256 } else { 128 }) {
            return Some((pixels, span));
        }
        let smaller = pixels.map(|v| (v / 2).max(1));
        if pixels == smaller {
            return None;
        }
        pixels = smaller;
    }
}

impl Pasteboard {
    pub(super) fn preview_density(
        &self,
        view: View,
        viewport: Rect,
        dim: [u32; 2],
        density: f32,
    ) -> f32 {
        let Some((pixels, _)) = tile_grid(view, viewport, dim, density, false) else {
            return 0.;
        };
        let minimum = (pixels[0] as f32 / dim[0] as f32).min(pixels[1] as f32 / dim[1] as f32);
        self.scene
            .as_ref()
            .map_or(minimum, |s| s.scale().into_iter().fold(minimum, f32::min))
    }

    pub(super) fn density(&self) -> Option<f32> {
        self.scene.as_ref().map(|s| s.scale()[0])
    }
    pub(super) fn sample(
        &self,
        doc: &Arc<Document>,
        frame: u32,
        point: [f32; 2],
    ) -> Option<Result<[u8; 4], String>> {
        if !self.source.ptr_eq(&Arc::downgrade(doc)) {
            return None;
        }
        let scene = self.scene.as_ref().filter(|s| s.frame() == frame)?;
        if self.on_page {
            let key = [0, 1].map(|i| (point[i] * scene.scale()[i] / TILE as f32).floor() as i64);
            if self.error.is_some() {
                return None;
            }
            if self.tiles.get(&key)?.is_none() {
                return Some(Ok([0; 4]));
            }
        }
        Some(scene.sample(doc, point))
    }

    pub(super) fn warning(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(super) fn clear(&mut self, ctx: &egui::Context) {
        self.tiles.clear();
        self.previous = None;
        self.scene = None;
        self.source = Default::default();
        self.error = None;
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        // Retain that receiver until its worker actually exits. Rapid pans
        // across the page edge may not create a succession of unbounded jobs.
        self.poll(ctx);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint(
        &mut self,
        p: &egui::Painter,
        slot: egui::layers::ShapeIdx,
        view: View,
        frame: u32,
        art: &FrameTexture,
        old: Option<&Arc<Document>>,
        density: f32,
        preview: bool,
        on_page: bool,
        settling: bool,
    ) -> bool {
        self.on_page = on_page;
        let viewport = if on_page {
            p.clip_rect().intersect(view.board)
        } else {
            p.clip_rect()
        };
        if !viewport.is_positive() || !on_page && view.board.contains_rect(viewport) {
            self.clear(p.ctx());
            return true;
        }
        let doc = art.authored();
        let same = old.is_some_and(|old| Arc::ptr_eq(old, doc));
        let dim = [doc.width, doc.height];
        // Follow the zoom's settled density. The page may temporarily use a
        // coarser texture while dragging an unrelated on-page object.
        let Some((pixels, (a, b))) = tile_grid(view, viewport, dim, density, on_page) else {
            self.clear(p.ctx());
            return true;
        };
        let reset = self
            .scene
            .as_ref()
            .is_none_or(|s| s.frame() != frame || s.page_pixels() != pixels)
            || old.is_none_or(|d| [d.width, d.height] != dim);
        let damage = if reset {
            self.previous = if same && self.scene.as_ref().is_some_and(|s| s.frame() == frame) {
                self.scene
                    .as_ref()
                    .map(|s| (Grid { scale: s.scale() }, std::mem::take(&mut self.tiles)))
            } else {
                None
            };
            self.tiles.clear();
            if let Some(j) = &self.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
            self.scene = if on_page {
                Scene::for_artboard(doc, frame, pixels)
            } else {
                Scene::new(doc, frame, pixels)
            };
            if self.scene.is_none() {
                self.clear(p.ctx());
                return true;
            }
            self.error = None;
            Damage::default()
        } else if !same {
            self.previous = None;
            if on_page && self.error.is_some() {
                self.tiles.clear();
            }
            self.error = None;
            self.scene
                .as_mut()
                .unwrap()
                .update(old.unwrap(), doc, [a, b])
        } else {
            Damage::default()
        };
        if reset || !same {
            self.source = Arc::downgrade(doc);
        }
        let scene = self.scene.as_ref().unwrap();
        if scene.is_empty() {
            self.tiles.clear();
            self.previous = None;
            if let Some(j) = &self.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
            self.poll(p.ctx());
            return true;
        }
        let grid = Grid {
            scale: scene.scale(),
        };
        let retained = |[x, y]: Tile| a[0] <= x && x <= b[0] && a[1] <= y && y <= b[1];
        self.tiles
            .retain(|&key, _| retained(key) && !(settling && damage.affects(key)));
        if let Some(j) = &self.job {
            if !retained(j.key) || damage.affects(j.key) {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.poll(p.ctx());
        let mut keys = Vec::new();
        for y in a[1]..=b[1] {
            for x in a[0]..=b[0] {
                let key = [x, y];
                if on_page || !view.board.contains_rect(grid.rect(view, key)) {
                    keys.push(key);
                }
            }
        }
        let pointer = p
            .ctx()
            .input(|i| i.pointer.hover_pos())
            .unwrap_or_else(|| viewport.center());
        keys.sort_by(|&a, &b| {
            grid.rect(view, a)
                .center()
                .distance_sq(pointer)
                .total_cmp(&grid.rect(view, b).center().distance_sq(pointer))
        });
        let start = Instant::now();
        let full = Damage::all();
        let mut missing = false;
        for (index, &key) in keys.iter().enumerate() {
            let cached = self.tiles.contains_key(&key);
            // Changed, previously visible tiles must show their current pose on
            // this paint, including the last pointer position or cancellation.
            if cached && !damage.affects(key) {
                continue;
            }
            // The released selection still presents the entire signed viewport.
            // Repaint its dirty tiles in the background before swapping displays.
            if settling && self.job.is_some() {
                missing = true;
                continue;
            }
            // Even listing candidates for many newly exposed tiles can be
            // expensive in dense drawings. Resume the rest on the next paint.
            if (on_page || settling) && index > 0 && start.elapsed() > Duration::from_millis(2) {
                self.tiles.remove(&key);
                missing = true;
                continue;
            }
            if !cached
                && self
                    .job
                    .as_ref()
                    .is_some_and(|j| !j.cancel.load(Ordering::Relaxed) && j.key == key)
            {
                missing = true;
                continue;
            }
            let requested = if cached { &damage } else { &full };
            let mut plan = self.scene.as_ref().unwrap().plan(key, requested).unwrap();
            let defer = !plan.is_empty()
                && (settling || !plan.is_small(doc) || start.elapsed() > Duration::from_millis(2));
            if defer
                && (settling
                    || on_page
                    || !cached
                        && !(preview
                            && (damage.affects(key) || grid.rect(view, key).contains(pointer))))
            {
                // A changed tile may be expensive. Show the current page preview
                // in its place while the worker repaints it at display density.
                if cached {
                    self.tiles.remove(&key);
                    plan = self.scene.as_ref().unwrap().plan(key, &full).unwrap();
                }
                if self.job.is_some() {
                    missing = true;
                    continue;
                }
                self.job = if settling || on_page && !preview {
                    let snapshot = Arc::clone(doc);
                    Job::start(p.ctx(), key, move |stop| plan.render_until(&snapshot, stop))
                } else {
                    let paint = plan.detach(doc);
                    Job::start(p.ctx(), key, move |stop| paint.render_until(stop))
                };
                if self.job.is_some() {
                    missing = true;
                    continue;
                }
                let plan = self.scene.as_ref().unwrap().plan(key, &full).unwrap();
                self.store(p.ctx(), key, plan.render(doc));
            } else {
                self.store(p.ctx(), key, plan.render(doc));
            }
        }
        if !settling || !missing {
            p.set(
                slot,
                display_filter::batch(p, self.images(view, viewport, art, grid, keys)),
            );
        }
        if !missing {
            self.previous = None;
        } else if self.job.is_none() {
            p.ctx().request_repaint();
        }
        !missing
    }

    /// During a wheel gesture the final grid is still unknown. Reproject
    /// already painted cells over the overview without looking up new geometry.
    pub(super) fn during_zoom(
        &mut self,
        p: &egui::Painter,
        slot: egui::layers::ShapeIdx,
        view: View,
        frame: u32,
        art: &FrameTexture,
    ) {
        if !self.source.ptr_eq(&Arc::downgrade(art.authored()))
            || self.scene.as_ref().is_none_or(|s| s.frame() != frame)
        {
            self.clear(p.ctx());
            return;
        }
        self.poll(p.ctx());
        let viewport = p.clip_rect().intersect(view.board);
        let grid = Grid {
            scale: self.scene.as_ref().unwrap().scale(),
        };
        if viewport.is_positive() {
            if let Some((a, b)) = grid.range(view, viewport, 256) {
                let keys = (a[1]..=b[1]).flat_map(|y| (a[0]..=b[0]).map(move |x| [x, y]));
                p.set(
                    slot,
                    display_filter::batch(p, self.images(view, viewport, art, grid, keys)),
                );
            }
        }
    }

    fn images(
        &self,
        view: View,
        viewport: Rect,
        art: &FrameTexture,
        grid: Grid,
        keys: impl IntoIterator<Item = Tile>,
    ) -> Vec<Image> {
        let on_page = self.on_page;
        let mut shapes = Vec::new();
        let overview = |part: Rect| {
            let uv = |p: egui::Pos2| ((p - view.board.min) / view.board.size()).to_pos2();
            Image::rectangular(
                art.handle.id(),
                part,
                Rect::from_min_max(uv(part.min), uv(part.max)),
                Color32::WHITE,
            )
        };
        for key in keys {
            let clip = grid.rect(view, key).intersect(viewport);
            if !clip.is_positive() {
                continue;
            }
            if on_page && self.error.is_some() {
                shapes.push(overview(clip));
                continue;
            }
            if let Some(tile) = self.tiles.get(&key) {
                if let Some(shape) = tile
                    .as_ref()
                    .and_then(|t| grid.image(view, key, t, viewport))
                {
                    shapes.push(shape);
                }
            } else if let Some((prior, tiles)) = &self.previous {
                // Only missing new cells display the last resolution. A transparent
                // replacement cannot reveal older pixels through a painted cell.
                if let Some((min, max)) = prior.range(view, clip, 256) {
                    for y in min[1]..=max[1] {
                        for x in min[0]..=max[0] {
                            let old = [x, y];
                            let part = prior.rect(view, old).intersect(clip);
                            if !part.is_positive() {
                                continue;
                            }
                            match tiles.get(&old) {
                                Some(Some(t)) => shapes.extend(prior.image(view, old, t, part)),
                                None if on_page => shapes.push(overview(part)),
                                _ => (),
                            }
                        }
                    }
                } else if on_page {
                    shapes.push(overview(clip));
                }
            } else if on_page {
                shapes.push(overview(clip));
            }
        }
        shapes
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(j) = self.job.take() else { return };
        match j.rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => self.job = Some(j),
            result if !j.cancel.load(Ordering::Relaxed) => self.store(
                ctx,
                j.key,
                result.unwrap_or_else(|_| {
                    Err("The pasteboard renderer stopped unexpectedly.".into())
                }),
            ),
            _ => {}
        }
    }
    fn store(&mut self, ctx: &egui::Context, key: Tile, result: Result<Patch, String>) {
        let handle = self.tiles.entry(key).or_default();
        let patch = match result {
            Ok(p) => p,
            Err(e) => {
                let name = if self.on_page {
                    "Artboard detail"
                } else {
                    "Pasteboard preview"
                };
                self.error = Some(format!("{name} unavailable. {e}"));
                return;
            }
        };
        let full = [TEXTURE as usize; 2];
        let name = if self.on_page {
            "artboard detail"
        } else {
            "pasteboard artwork"
        };
        let color = |p: renderer::Pixmap| {
            egui::ColorImage::from_rgba_premultiplied(
                [p.width() as usize, p.height() as usize],
                p.data(),
            )
        };
        if patch.size == full {
            if let Some(pixels) = patch.pixels {
                let image = color(pixels);
                if let Some(h) = handle {
                    h.set(image, ARTWORK_TEXTURE);
                } else {
                    *handle = Some(ctx.load_texture(name, image, ARTWORK_TEXTURE));
                }
            } else {
                *handle = None;
            }
        } else if let Some(h) = handle {
            let image = patch
                .pixels
                .map(color)
                .unwrap_or_else(|| egui::ColorImage::new(patch.size, Color32::TRANSPARENT));
            h.set_partial(patch.origin, image, ARTWORK_TEXTURE);
        } else if let Some(pixels) = patch.pixels {
            let patch_image = color(pixels);
            let mut image = egui::ColorImage::new(full, Color32::TRANSPARENT);
            for (y, row) in patch_image.pixels.chunks_exact(patch.size[0]).enumerate() {
                let start = (y + patch.origin[1]) * full[0] + patch.origin[0];
                image.pixels[start..start + row.len()].copy_from_slice(row);
            }
            *handle = Some(ctx.load_texture(name, image, ARTWORK_TEXTURE));
        }
    }
}

#[cfg(test)]
mod tests;
