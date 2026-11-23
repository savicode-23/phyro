//! Refine all onion poses into one display-resolution texture of the visible
//! artboard. The small per-pose masks remain available immediately during scrubs.
use super::{
    display_filter, egui, Color32, Ghost, Painter, Pos2, Rect, TextureHandle, View, ARTWORK_TEXTURE,
};
use renderer::pasteboard::{Damage, Scene, TILE};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
struct Region {
    page: [u32; 2],
    min: [u32; 2],
    max: [u32; 2],
}

impl Region {
    fn visible(ctx: &egui::Context, view: View, clip: Rect) -> Option<Self> {
        let visible = clip.intersect(view.board);
        if !visible.is_positive() {
            return None;
        }
        // Limit only the displayed portion on unusually large monitors. Reserve
        // room for a small pan before the next refinement has to be requested.
        let side = ctx.input(|i| i.max_texture_side).saturating_sub(66).max(1) as f32;
        let density = ctx
            .pixels_per_point()
            .min(side / visible.width().max(visible.height()))
            .min((8_000_000. / visible.area()).sqrt());
        let size = view.board.size() * density;
        let page = [size.x.round().max(1.) as u32, size.y.round().max(1.) as u32];
        if page[0].max(page[1]) <= 512 {
            return None;
        }
        let to_pixel = |p: Pos2| {
            let unit = (p - view.board.min) / view.board.size();
            [unit.x * page[0] as f32, unit.y * page[1] as f32]
        };
        let min = to_pixel(visible.min).map(|v| v.floor() as u32);
        let max = to_pixel(visible.max).map(|v| v.ceil() as u32);
        let max = [max[0].min(page[0]), max[1].min(page[1])];
        (min[0] < max[0] && min[1] < max[1]).then_some(Self { page, min, max })
    }

    fn padded(self) -> Self {
        Self {
            min: self.min.map(|v| v.saturating_sub(32)),
            max: std::array::from_fn(|i| (self.max[i] + 32).min(self.page[i])),
            ..self
        }
    }

    fn covers(self, other: Self) -> bool {
        self.page == other.page
            && (0..2).all(|i| self.min[i] <= other.min[i] && self.max[i] >= other.max[i])
    }

    fn screen(self, view: View) -> Rect {
        let position = |[x, y]: [u32; 2]| {
            view.board.min
                + view.board.size()
                    * egui::vec2(
                        x as f32 / self.page[0] as f32,
                        y as f32 / self.page[1] as f32,
                    )
        };
        Rect::from_min_max(position(self.min), position(self.max))
    }
}

struct Request {
    region: Region,
    ghosts: Vec<Ghost>,
}

impl Request {
    fn matches(&self, visible: Region, ghosts: &[Ghost]) -> bool {
        self.region.covers(visible)
            && self.ghosts.len() == ghosts.len()
            && self.ghosts.iter().zip(ghosts).all(|(a, b)| {
                a.frame == b.frame && a.tint == b.tint && Arc::ptr_eq(&a.drawing, &b.drawing)
            })
    }

    fn render(&self, cancel: &AtomicBool) -> Option<egui::ColorImage> {
        let Region { page, min, max } = self.region;
        let size = [(max[0] - min[0]) as usize, (max[1] - min[1]) as usize];
        let mut composite = egui::ColorImage::new(size, Color32::TRANSPARENT);
        for ghost in self.ghosts.iter().rev() {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let scene = Scene::for_artboard(&ghost.drawing, ghost.frame, page)?;
            if scene.is_empty() {
                continue;
            }
            let [r, g, b, a] = ghost.tint;
            let color = Color32::from_rgba_unmultiplied(r, g, b, a);
            for ty in min[1] / TILE..=(max[1] - 1) / TILE {
                for tx in min[0] / TILE..=(max[0] - 1) / TILE {
                    if cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    let plan = scene.plan([i64::from(tx), i64::from(ty)], &Damage::all())?;
                    let Some(tile) = plan.render(&ghost.drawing).ok()?.pixels else {
                        continue;
                    };
                    let left = tx * TILE;
                    let top = ty * TILE;
                    let start = left.max(min[0]);
                    let end = (left + TILE).min(max[0]);
                    for y in top.max(min[1])..(top + TILE).min(max[1]) {
                        // The texture has one neighboring texel on each edge.
                        let src = ((y - top + 1) * tile.width() + start - left + 1) as usize;
                        let dst = (y - min[1]) as usize * size[0] + (start - min[0]) as usize;
                        let count = (end - start) as usize;
                        for (out, pixel) in composite.pixels[dst..dst + count]
                            .iter_mut()
                            .zip(&tile.pixels()[src..src + count])
                        {
                            *out = out.blend(color.gamma_multiply_u8(pixel.alpha()));
                        }
                    }
                }
            }
        }
        Some(composite)
    }
}

struct Job {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Option<egui::ColorImage>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(super) struct Detail {
    request: Option<Arc<Request>>,
    deadline: f64,
    job: Option<Job>,
    // A failed request keeps using the interactive masks until the view changes.
    ready: Option<Option<TextureHandle>>,
}

impl Detail {
    pub(super) fn clear(&mut self) {
        self.request = None;
        self.ready = None;
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
            if !matches!(job.result.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                self.job = None;
            }
        }
    }

    pub(super) fn paint(
        &mut self,
        ctx: &egui::Context,
        painter: &Painter,
        view: View,
        ghosts: &[Ghost],
    ) -> bool {
        let Some(visible) = Region::visible(ctx, view, painter.clip_rect()) else {
            self.clear();
            return false;
        };
        let now = ctx.input(|i| i.time);
        if !self
            .request
            .as_ref()
            .is_some_and(|r| r.matches(visible, ghosts))
        {
            self.clear();
            self.request = Some(Arc::new(Request {
                region: visible.padded(),
                ghosts: ghosts.to_vec(),
            }));
            self.deadline = now + 0.08;
        }
        let request = self.request.as_ref().unwrap();
        if let Some(job) = &self.job {
            match job.result.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return false,
                reply => {
                    if !job.cancel.load(Ordering::Relaxed) {
                        self.ready =
                            Some(reply.ok().flatten().map(|image| {
                                ctx.load_texture("onion detail", image, ARTWORK_TEXTURE)
                            }));
                    }
                    self.job = None;
                }
            }
        }
        if let Some(ready) = &self.ready {
            if let Some(texture) = ready {
                painter.add(display_filter::image(
                    painter,
                    texture.id(),
                    request.region.screen(view),
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                    Color32::WHITE,
                ));
            }
            return ready.is_some();
        }
        if now < self.deadline {
            ctx.request_repaint_after(Duration::from_secs_f64(self.deadline - now));
        } else {
            let (tx, result) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let request = request.clone();
            let viewport = ctx.viewport_id();
            let ctx = ctx.clone();
            let worker = std::thread::Builder::new()
                .name("phyro-onion-detail".into())
                .spawn(move || {
                    let image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        request.render(&stop)
                    }))
                    .ok()
                    .flatten();
                    if tx.send(image).is_ok() {
                        ctx.request_repaint_of(viewport);
                    }
                });
            if worker.is_ok() {
                self.job = Some(Job { cancel, result });
            } else {
                self.ready = Some(None);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::Document;

    #[test]
    fn visible_crop_matches_the_tinted_masks_across_tile_boundaries() {
        let drawing = |x| {
            let mut doc = Document {
                width: 128,
                height: 128,
                ..Default::default()
            };
            doc.add_rectangle([x, 20.125], [12.5, 40.], [255, 190, 80, 173]);
            Arc::new(doc)
        };
        let front = Ghost {
            drawing: drawing(30.125),
            frame: 0,
            tint: [200, 90, 30, 160],
        };
        let back = Ghost {
            drawing: drawing(34.625),
            frame: 0,
            tint: [20, 100, 255, 90],
        };
        let visible = Region {
            page: [1024; 2],
            min: [241, 158],
            max: [392, 512],
        };
        let request = Request {
            region: visible,
            ghosts: vec![front, back],
        };
        let pixels = request.render(&AtomicBool::new(false)).unwrap();
        let reference: Vec<_> = request
            .ghosts
            .iter()
            .rev()
            .map(|g| {
                let [r, gc, b, a] = g.tint;
                (
                    renderer::render_scaled(&g.drawing, g.frame, 8.).unwrap(),
                    Color32::from_rgba_unmultiplied(r, gc, b, a),
                )
            })
            .collect();
        assert_eq!(pixels.size, [151, 354]);
        for y in visible.min[1]..visible.max[1] {
            for x in visible.min[0]..visible.max[0] {
                let expected = reference
                    .iter()
                    .fold(Color32::TRANSPARENT, |base, (p, tint)| {
                        base.blend(tint.gamma_multiply_u8(p.pixel(x, y).unwrap().alpha()))
                    });
                assert_eq!(
                    pixels[((x - visible.min[0]) as usize, (y - visible.min[1]) as usize)],
                    expected
                );
            }
        }
        assert!(request.render(&AtomicBool::new(true)).is_none());
        assert!(request.matches(visible, &request.ghosts));
        let mut recolored = request.ghosts.clone();
        recolored[1].tint[2] = 70;
        assert!(!request.matches(visible, &recolored));
    }
}
