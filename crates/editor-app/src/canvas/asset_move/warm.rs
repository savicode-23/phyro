use super::{egui, Document, Rect, Sprite, Sprites, VectorObject};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Weak,
};

#[derive(Default)]
pub(super) struct Warm {
    seen: Option<Seen>,
    job: Option<Job>,
}

struct Seen {
    drawing: Weak<Document>,
    frame: u32,
    density: f32,
    selected: Option<u64>,
    area: Rect,
}

struct Job {
    result: mpsc::Receiver<(VectorObject, u32, renderer::MoveSprite)>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Warm {
    pub(super) fn poll(&mut self, ctx: &egui::Context, sprites: &mut Sprites, editable: bool) {
        let Some(job) = self.job.as_ref() else {
            return;
        };
        if !editable {
            job.cancel.store(true, Ordering::Relaxed);
            self.seen = None;
        }
        // Small character poses often finish together. Publish them in one paint
        // while still limiting the transfer cost of a large preview on input.
        let mut bytes = 0;
        for _ in 0..4 {
            match job.result.try_recv() {
                Ok((object, frame, image)) => {
                    if !job.cancel.load(Ordering::Relaxed)
                        && sprites
                            .find(&object, frame, image.requested_scale)
                            .is_none()
                    {
                        bytes += image.pixels.data().len();
                        if let Some(sprite) = Sprite::upload(ctx, &object, frame, image) {
                            sprites.remember(&sprite);
                        }
                    }
                    ctx.request_repaint();
                    if bytes >= 1024 * 1024 {
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    ctx.request_repaint();
                    break;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn request(
        &mut self,
        ctx: &egui::Context,
        sprites: &Sprites,
        doc: &Arc<Document>,
        frame: u32,
        scale: f32,
        selection: Option<u64>,
        visible: Rect,
    ) {
        let snapshot = Arc::downgrade(doc);
        if self.seen.as_ref().is_some_and(|seen| {
            seen.drawing.ptr_eq(&snapshot)
                && seen.frame == frame
                && seen.density == scale
                && seen.selected == selection
                && seen.area.contains_rect(visible)
        }) {
            return;
        }
        if let Some(job) = self.job.as_ref() {
            // A new selection in the same drawing can use the in-flight results.
            if self.seen.as_ref().is_none_or(|seen| {
                seen.frame != frame || seen.density != scale || !seen.area.intersects(visible)
            }) {
                job.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        let area = visible.expand2(visible.size() * 0.25);
        self.seen = Some(Seen {
            drawing: snapshot,
            frame,
            density: scale,
            selected: selection,
            area,
        });
        if renderer::requires_scene_compositing(doc) {
            return;
        }
        // Keep idle work bounded even for a vast pasteboard. The active selection
        // and frontmost nearby objects have priority. Sorting child vectors or
        // traversing their geometry would make opening the parent unnecessarily expensive.
        let selected = selection.and_then(|id| doc.objects.iter().find(|o| o.id == id));
        let candidates = selected
            .into_iter()
            .chain(doc.objects.iter().rev().take(128))
            .filter(|o| doc.layers.iter().any(|l| l.id == o.layer && l.visible));
        let targets: Vec<_> = sprites
            .missing(candidates, frame, scale, area, selection)
            .into_iter()
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (send, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let viewport = ctx.viewport_id();
        if std::thread::Builder::new()
            .name("phyro-placement-preview".into())
            .spawn(move || {
                for object in targets {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(image) = renderer::move_sprite(&object, frame, scale) {
                        if stop.load(Ordering::Relaxed)
                            || send.send((object, frame, image)).is_err()
                        {
                            break;
                        }
                        wake.request_repaint_of(viewport);
                    }
                }
                drop(send);
                wake.request_repaint_of(viewport);
            })
            .is_ok()
        {
            self.job = Some(Job { result, cancel });
        }
    }
}
