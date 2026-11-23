use super::{display_filter, egui, Document, Pos2, Rect, View};
mod release;
#[cfg(test)]
mod tests;
pub(super) use release::Progress;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

#[derive(Default)]
pub(super) struct SelectionMove {
    release: release::Release,
    request: Option<Request>,
    job: Option<Job>,
    layers: Option<Vec<Layer>>,
}

struct Request {
    source: Arc<Document>,
    ids: Vec<u64>,
    frame: u32,
    density: f32,
    visible: Rect,
    representative: usize,
    members: Vec<usize>,
    sorted_ids: Vec<u64>,
    scheduled: bool,
    tried_coarse: bool,
}

struct Layer {
    moving: bool,
    corners: [Pos2; 4],
    texture: egui::TextureHandle,
}

type Captures = Vec<(bool, renderer::MoveSprite)>;

impl Layer {
    fn upload(ctx: &egui::Context, layers: Captures) -> Vec<Self> {
        layers
            .into_iter()
            .map(|(moving, sprite)| {
                let size = [
                    sprite.pixels.width() as usize,
                    sprite.pixels.height() as usize,
                ];
                let rect = Rect::from_min_size(
                    Pos2::from(sprite.origin),
                    egui::vec2(size[0] as f32, size[1] as f32) / sprite.scale,
                );
                let color = egui::ColorImage::from_rgba_premultiplied(size, sprite.pixels.data());
                Self {
                    moving,
                    corners: [
                        rect.left_top(),
                        rect.right_top(),
                        rect.left_bottom(),
                        rect.right_bottom(),
                    ],
                    texture: ctx.load_texture(
                        "selection transform",
                        color,
                        display_filter::ARTWORK_TEXTURE,
                    ),
                }
            })
            .collect()
    }
}

struct Job {
    receiver: mpsc::Receiver<Option<Captures>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl SelectionMove {
    pub(super) fn reset(&mut self) {
        self.release.cancel();
        self.request = None;
        self.layers = None;
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
            if !matches!(job.receiver.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                self.job = None;
            }
        }
    }

    pub(super) fn prepare(
        &mut self,
        ctx: &egui::Context,
        source: &Arc<Document>,
        ids: &[u64],
        frame: u32,
        (density, visible): (f32, Rect),
        publish: bool,
    ) {
        if self.request.as_ref().is_some_and(|r| {
            r.frame != frame
                || r.density != density
                || r.ids != ids
                || !r.visible.contains_rect(visible)
                || !(Arc::ptr_eq(&r.source, source) || r.source.as_ref() == source.as_ref())
        }) {
            self.reset();
        }
        // Once an interim pose is under the pointer, a large upload can take
        // longer than the complete gesture update. Its exact replacement can
        // be published when idle. Canceled work can always be drained.
        if let Some(job) = self
            .job
            .as_ref()
            .filter(|job| publish || job.cancel.load(Ordering::Relaxed))
        {
            match job.receiver.try_recv() {
                Ok(result) => {
                    if !job.cancel.load(Ordering::Relaxed) {
                        if let Some(images) = result {
                            self.layers = Some(Layer::upload(ctx, images));
                        }
                    }
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.job = None,
            }
        }
        if ids.len() < 2 {
            return;
        }
        if self.request.is_none() {
            // Capturing the static artwork visits the whole paint stack. A
            // small marquee on a dense sheet often only changes selection;
            // leave release and idle cheap until the artist grabs a handle.
            if publish && source.objects.len() >= 1024 && ids.len() <= 32 {
                return;
            }
            let mut selected = ids.to_vec();
            selected.sort_unstable();
            // Derive the shared affine map from a well-conditioned member. The
            // evaluated poses below always come from the real gesture, including
            // its keying, axis constraints, pivot and representable-skew limits.
            let members: Vec<_> = source
                .objects
                .iter()
                .enumerate()
                .filter(|(_, o)| selected.binary_search(&o.id).is_ok())
                .map(|(i, _)| i)
                .collect();
            let Some((representative, _)) = members
                .iter()
                .map(|&i| {
                    let o = &source.objects[i];
                    (
                        i,
                        o.scaled_box_at(frame)
                            .1
                            .into_iter()
                            .fold(f32::INFINITY, f32::min),
                    )
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
            else {
                return;
            };
            let area = visible.expand2(visible.size() * 0.1);
            self.request = Some(Request {
                source: source.clone(),
                ids: ids.to_vec(),
                frame,
                density,
                visible: area,
                representative,
                members,
                sorted_ids: selected,
                scheduled: false,
                tried_coarse: false,
            });
        }
        let request = self.request.as_mut().expect("prepared selection");
        if self.job.is_some() || request.scheduled {
            return;
        }
        request.scheduled = true;
        let selected = request.sorted_ids.clone();
        let area = request.visible;
        let drawing = source.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let wake = ctx.clone();
        let viewport = ctx.viewport_id();
        if std::thread::Builder::new()
            .name("phyro-selection-preview".into())
            .spawn(move || {
                let images = capture(
                    &drawing,
                    &selected,
                    frame,
                    density,
                    area,
                    renderer::MAX_SPRITE_EDGE,
                    &stop,
                );
                let _ = sender.send(if stop.load(Ordering::Relaxed) {
                    None
                } else {
                    images
                });
                wake.request_repaint_of(viewport);
            })
            .is_ok()
        {
            self.job = Some(Job { receiver, cancel });
        }
    }

    pub(super) fn drag_images(
        &mut self,
        ctx: &egui::Context,
        posed: &Document,
        view: View,
    ) -> Option<Vec<display_filter::Image>> {
        let request = self.request.as_mut()?;
        if self.layers.is_none() && !request.tried_coarse {
            // A quick grab can beat the background renderer. Show the real
            // selection on that first movement at a bounded interim density.
            request.tried_coarse = true;
            const EDGE: u32 = 384;
            let density = request
                .density
                .min(EDGE as f32 / request.visible.size().max_elem());
            if let Some(layers) = capture(
                &request.source,
                &request.sorted_ids,
                request.frame,
                density,
                request.visible,
                EDGE,
                &AtomicBool::new(false),
            ) {
                self.layers = Some(Layer::upload(ctx, layers));
            }
        }
        self.images(posed, view)
    }

    pub(super) fn images(
        &self,
        posed: &Document,
        view: View,
    ) -> Option<Vec<display_filter::Image>> {
        let r = self.request.as_ref()?;
        let layers = self.layers.as_ref()?;
        let transform = r.transform(posed, view)?;
        let mut images = Vec::with_capacity(layers.len());
        for layer in layers {
            let points = layer
                .corners
                .map(|p| view.screen(if layer.moving { transform.map(p) } else { p }));
            if !points.iter().all(|p| p.is_finite()) {
                return None;
            }
            images.push(display_filter::Image::affine(layer.texture.id(), points));
        }
        Some(images)
    }
}

struct SelectionTransform {
    origin: Pos2,
    target: Pos2,
    x: egui::Vec2,
    y: egui::Vec2,
}

impl SelectionTransform {
    fn map(&self, p: Pos2) -> Pos2 {
        let delta = p - self.origin;
        self.target + self.x * delta.x + self.y * delta.y
    }
}

impl Request {
    fn transform(&self, posed: &Document, view: View) -> Option<SelectionTransform> {
        let before = &self.source.objects[self.representative];
        let after = posed.objects.get(self.representative)?;
        if before.id != after.id {
            return None;
        }
        let inverse = before.world_to_local_at(self.frame);
        let forward = after.local_to_world_at(self.frame);
        let map = |p: Pos2| Pos2::from(forward(inverse(p.into())));
        let origin = Pos2::from(before.local_to_world([0.5, 0.5], self.frame));
        let target = map(origin);
        let step = before
            .scaled_box_at(self.frame)
            .1
            .into_iter()
            .fold(32., f32::max);
        let transform = SelectionTransform {
            origin,
            target,
            x: (map(origin + egui::vec2(step, 0.)) - target) / step,
            y: (map(origin + egui::vec2(0., step)) - target) / step,
        };
        // Locked members or a transform with an unusual pivot can have a
        // different evaluated pose. Fall back to the normal vector painter
        // when they cannot all be represented by one screen-space transform.
        if !self.members.iter().all(|&i| {
            let original = &self.source.objects[i];
            let Some(new) = posed.objects.get(i).filter(|o| o.id == original.id) else {
                return false;
            };
            let from = original.local_to_world_at(self.frame);
            let to = new.local_to_world_at(self.frame);
            [[0., 0.], [1., 0.], [0., 1.]].into_iter().all(|p| {
                transform
                    .map(Pos2::from(from(p)))
                    .distance(Pos2::from(to(p)))
                    * view.scale
                    <= 0.1
            })
        }) {
            return None;
        }
        Some(transform)
    }
}

fn capture(
    doc: &Document,
    ids: &[u64],
    frame: u32,
    density: f32,
    view: Rect,
    max_edge: u32,
    stop: &AtomicBool,
) -> Option<Captures> {
    let objects = doc.paint_order();
    let moving = |o: &document_core::VectorObject| ids.binary_search(&o.id).is_ok();
    if objects
        .iter()
        .any(|o| moving(o) && (o.skin.is_some() || o.rigid_bone.is_some()))
    {
        return None;
    }
    let mut images = Vec::new();
    let mut bytes = 0;
    let region = [view.left(), view.top(), view.right(), view.bottom()];
    let static_density =
        density.min((renderer::MAX_SPRITE_EDGE - 12) as f32 / view.size().max_elem());
    let mut rest = objects.as_slice();
    while let Some(&first) = rest.first() {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let selected = moving(first);
        let count = rest.iter().take_while(|o| moving(o) == selected).count();
        let (run, remaining) = rest.split_at(count);
        // A group clip composites its consecutive members before applying
        // opacity from the mask. Never split that operation into two textures.
        if remaining
            .first()
            .is_some_and(|next| run[count - 1].shares_clip_mask(next, frame))
        {
            return None;
        }
        rest = remaining;
        if !run
            .iter()
            .any(|o| o.visible_at(frame) && o.opacity_at(frame) > 0.)
        {
            continue;
        }
        let image = if selected {
            renderer::selection_run(run, frame, density, max_edge)?
        } else {
            let Some(image) =
                renderer::foreground_in_region(run, frame, static_density, Some(region)).ok()?
            else {
                continue;
            };
            image
        };
        bytes += image.pixels.data().len();
        if bytes > 32 * 1024 * 1024 || images.len() >= 64 {
            return None;
        }
        images.push((selected, image));
    }
    Some(images)
}
