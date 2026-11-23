use super::{display_filter, egui, Arc, Document, SelectionMove, View};
use crate::canvas::raster::{PageImage, Refinement};

#[derive(Default)]
pub(super) struct Release {
    pending: bool,
    recapture: bool,
    page_scale: Option<f32>,
    job: Option<Refinement>,
}

pub(in crate::canvas) enum Progress {
    Inactive,
    Rendering,
    Page(PageImage),
    Tiles,
}

impl Release {
    pub(super) fn cancel(&mut self) {
        self.pending = false;
        self.recapture = false;
        self.page_scale = None;
        // Keep the occupied worker slot. A rapid sequence of releases can
        // produce at most one full page render in flight per active canvas.
    }
}

impl SelectionMove {
    pub(in crate::canvas) fn can_commit(&self, doc: &Document, ids: &[u64], frame: u32) -> bool {
        self.layers.is_some()
            && self.request.as_ref().is_some_and(|r| {
                r.ids == ids
                    && r.frame == frame
                    && (std::ptr::eq(r.source.as_ref(), doc) || r.source.as_ref() == doc)
            })
    }

    pub(in crate::canvas) fn commit(&mut self, doc: &Arc<Document>, view: View) {
        let (Some(r), Some(layers)) = (&mut self.request, &mut self.layers) else {
            return;
        };
        let Some(transform) = r.transform(doc, view) else {
            return;
        };
        if layers
            .iter()
            .filter(|l| l.moving)
            .any(|l| l.corners.iter().any(|p| !transform.map(*p).is_finite()))
        {
            return;
        }
        self.release.recapture |= r.tried_coarse
            || (transform.x - egui::Vec2::X).length_sq() > 1e-8
            || (transform.y - egui::Vec2::Y).length_sq() > 1e-8;
        for layer in layers.iter_mut().filter(|l| l.moving) {
            layer.corners = layer.corners.map(|p| transform.map(p));
        }
        if let Some(job) = &self.job {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.release.recapture = true;
        }
        r.source = doc.clone();
        self.release.pending = true;
        self.release.page_scale = None;
    }

    pub(in crate::canvas) fn advance(&mut self, ctx: &egui::Context, scale: f32) -> Progress {
        let done = self.release.job.as_ref().and_then(|job| job.completed());
        let finished = done.and_then(|result| self.release.job.take().map(|job| (job, result)));
        let Some(r) = self.request.as_ref().filter(|_| self.release.pending) else {
            return Progress::Inactive;
        };
        if self.release.page_scale == Some(scale) {
            return Progress::Tiles;
        }
        if let Some((_, result)) =
            finished.filter(|(job, _)| job.is_target(&r.source, r.frame, scale))
        {
            if let Ok(page) = result {
                self.release.page_scale = Some(scale);
                return Progress::Page(page);
            }
            self.release.cancel();
            return Progress::Inactive;
        }
        if self.release.job.is_none() {
            self.release.job = Refinement::page(ctx, &r.source, r.frame, scale);
            if self.release.job.is_none() {
                self.release.cancel();
                return Progress::Inactive;
            }
        }
        Progress::Rendering
    }

    pub(in crate::canvas) fn committed_images(&self, view: View) -> Vec<display_filter::Image> {
        self.layers
            .as_ref()
            .expect("committed selection layers")
            .iter()
            .map(|layer| {
                display_filter::Image::affine(
                    layer.texture.id(),
                    layer.corners.map(|p| view.screen(p)),
                )
            })
            .collect()
    }

    pub(in crate::canvas) fn finish_settlement(&mut self, ctx: &egui::Context) {
        if self.release.recapture {
            self.reset();
            ctx.request_repaint();
        } else {
            self.release.cancel();
        }
    }
}

#[cfg(test)]
#[test]
fn immediate_second_release_and_undo_never_install_a_stale_page() {
    use std::time::{Duration, Instant};
    let ctx = egui::Context::default();
    let mut doc = Document::default();
    let a = doc.add_rectangle([1., 6.], [70., 32.], [180, 42, 20, 160]);
    let b = doc.add_rectangle([4., 8.], [18., 32.], [40, 200, 100, 210]);
    let before = Arc::new(doc);
    let ids = [a, b];
    let rect = egui::Rect::from_min_size(egui::pos2(-40., -40.), egui::vec2(200., 200.));
    let view = View {
        board: rect,
        scale: 1.,
    };
    let mut cache = SelectionMove::default();
    let until = Instant::now() + Duration::from_secs(5);
    while !cache.can_commit(&before, &ids, 0) {
        cache.prepare(&ctx, &before, &ids, 0, (1., rect), true);
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let first_corner = cache.layers.as_ref().unwrap()[0].corners[0];
    let translate = |from: &Document, distance| {
        let mut drawing = from.clone();
        for object in &mut drawing.objects {
            object.position[0] += distance;
        }
        Arc::new(drawing)
    };
    let first = translate(&before, -20.);
    cache.commit(&first, view);
    assert_eq!(
        cache.layers.as_ref().unwrap()[0].corners[0],
        first_corner - egui::vec2(20., 0.)
    );
    assert!(matches!(cache.advance(&ctx, 1.), Progress::Rendering));
    let second = translate(&first, 35.);
    assert!(cache.can_commit(&first, &ids, 0));
    cache.commit(&second, view);
    assert_eq!(
        cache.layers.as_ref().unwrap()[0].corners[0],
        first_corner + egui::vec2(15., 0.)
    );
    loop {
        match cache.advance(&ctx, 1.) {
            Progress::Page((page, _)) => {
                let exact = renderer::render(&second, 0).unwrap();
                assert_eq!(
                    page,
                    egui::ColorImage::from_rgba_premultiplied([128; 2], exact.data())
                );
                break;
            }
            Progress::Rendering => (),
            _ => panic!("lost the second release"),
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(cache.advance(&ctx, 1.), Progress::Tiles));
    // Undo while the signed pasteboard tiles are still catching up.
    cache.prepare(&ctx, &first, &ids, 0, (1., rect), true);
    assert!(matches!(cache.advance(&ctx, 1.), Progress::Inactive));
}
