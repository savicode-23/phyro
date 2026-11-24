use super::{display_filter, raster, Canvas, Document, FrameTexture, View};
use document_core::VectorObject;
use eframe::egui::{self, ColorImage, Pos2, Rect};
use std::sync::Arc;
mod layers;
mod warm;
use layers::{Layer, Sprites};

#[derive(Clone)]
struct Sprite {
    original: VectorObject,
    frame: u32,
    scale: f32,
    image: Image,
    local_corners: [[f32; 2]; 4],
    captured_size: [f32; 2],
}

impl Sprite {
    fn upload(
        ctx: &egui::Context,
        object: &VectorObject,
        frame: u32,
        sprite: renderer::MoveSprite,
    ) -> Option<Self> {
        let scale = sprite.requested_scale;
        let bounds = Image::bounds(&sprite);
        let local = object.world_to_local_at(frame);
        let local_corners = [
            bounds.left_top(),
            bounds.right_top(),
            bounds.left_bottom(),
            bounds.right_bottom(),
        ]
        .map(|p| local(p.into()));
        let world = object.local_to_world_at(frame);
        if local_corners
            .iter()
            .any(|&p| !p.into_iter().chain(world(p)).all(f32::is_finite))
        {
            return None;
        }
        Some(Self {
            original: object.clone(),
            frame,
            scale,
            image: Image::upload(ctx, "moving artwork", sprite),
            local_corners,
            captured_size: object.scaled_box_at(frame).1,
        })
    }

    fn corners_at(&self, pose: &VectorObject, frame: u32) -> [Pos2; 4] {
        let world = pose.local_to_world_at(frame);
        self.local_corners.map(|corner| Pos2::from(world(corner)))
    }

    fn refresh_density(&self) -> Option<(u64, f32)> {
        let size = self.original.scaled_box_at(self.frame).1;
        size.into_iter()
            .zip(self.captured_size)
            .any(|(current, captured)| !(0.67..1.5).contains(&(current / captured)))
            .then_some((self.original.id, self.scale))
    }
}

#[derive(Clone)]
struct Image {
    bounds: Rect,
    texture: egui::TextureHandle,
}

impl Image {
    fn bounds(sprite: &renderer::MoveSprite) -> Rect {
        Rect::from_min_size(
            Pos2::from(sprite.origin),
            egui::vec2(sprite.pixels.width() as f32, sprite.pixels.height() as f32) / sprite.scale,
        )
    }

    fn upload(ctx: &egui::Context, name: &str, sprite: renderer::MoveSprite) -> Self {
        let bounds = Self::bounds(&sprite);
        let pixels = sprite.pixels;
        let size = [pixels.width() as usize, pixels.height() as usize];
        let image = ColorImage::from_rgba_premultiplied(size, pixels.data());
        Self {
            bounds,
            texture: ctx.load_texture(name, image, display_filter::ARTWORK_TEXTURE),
        }
    }
}

struct Backdrop {
    source: Arc<Document>,
    below: Arc<Document>,
    under: Vec<Layer>,
    above: Vec<Layer>,
}

#[derive(Default)]
pub(super) struct AssetMove {
    sprite: Option<Sprite>,
    sprites: Sprites,
    warm: warm::Warm,
    backdrop: Option<Backdrop>,
    released: Option<Arc<Document>>,
    worker: Option<raster::Refinement<raster::SettledArtwork>>,
    rejected: Option<(u64, u32, f32)>,
    visible: Option<Rect>,
    before_placement: Option<(u64, FrameTexture)>,
}

pub(super) enum Settle {
    PaintNormally,
    KeepLayers,
    Finished(raster::PageImage),
}

impl Canvas {
    pub(super) fn retain_placement_background(&mut self, frame: u32, target: Option<u64>) {
        self.asset_move.before_placement = target.and_then(|id| {
            let page = self.texture.as_ref()?;
            if page.frame != frame
                || page.pending_split.is_some()
                || page.failed_refinement.is_some()
                || self.page_drag.is_some()
                || self.drag_preview.is_some()
                || !raster::dense_artboard(&page.document)
            {
                return None;
            }
            self.texture.take().map(|page| (id, page))
        });
    }

    pub(crate) fn prime_move_sprite(
        &mut self,
        ctx: &egui::Context,
        doc: &Arc<Document>,
        frame: u32,
        id: u64,
        sprite: renderer::MoveSprite,
    ) {
        if let Some(object) = doc.objects.iter().find(|o| o.id == id) {
            self.asset_move.install(ctx, object, frame, sprite);
        }
    }
}

impl AssetMove {
    pub(super) fn set_view(&mut self, visible: Rect) {
        if self.visible != Some(visible) {
            self.cancel();
            self.rejected = None;
            self.visible = Some(visible);
        }
    }

    pub(super) fn poll_warm(&mut self, ctx: &egui::Context, editable: bool) {
        if !editable || self.backdrop.is_none() {
            self.warm.poll(ctx, &mut self.sprites, editable);
        }
    }

    pub(super) fn warm_scene(
        &mut self,
        ctx: &egui::Context,
        doc: &Arc<Document>,
        frame: u32,
        scale: f32,
        selection: Option<u64>,
        visible: Rect,
    ) {
        self.warm
            .request(ctx, &self.sprites, doc, frame, scale, selection, visible);
    }

    pub(super) fn take_placement_background(
        &mut self,
        frame: u32,
        scale: f32,
    ) -> Option<FrameTexture> {
        let (id, cached) = self.before_placement.take()?;
        let backdrop = self.backdrop.as_mut()?;
        if self.sprite.as_ref()?.original.id != id
            || cached.frame != frame
            || cached.scale != scale
            || !backdrop.above.is_empty()
            || !backdrop.under.is_empty()
        {
            return None;
        }
        // Allocation advances when the item is inserted. Every painted property
        // must still match the previous page before reusing its texture.
        let below = Arc::make_mut(&mut backdrop.below);
        below.next_id = cached.document.next_id;
        if !cached.matches(below, frame) {
            return None;
        }
        backdrop.below = cached.document.clone();
        Some(cached)
    }

    fn install(
        &mut self,
        ctx: &egui::Context,
        object: &VectorObject,
        frame: u32,
        sprite: renderer::MoveSprite,
    ) -> bool {
        let Some(sprite) = Sprite::upload(ctx, object, frame, sprite) else {
            return false;
        };
        self.sprites.remember(&sprite);
        self.sprite = Some(sprite);
        self.backdrop = None;
        self.released = None;
        true
    }

    pub(super) fn prepare(
        &mut self,
        ctx: &egui::Context,
        doc: &Arc<Document>,
        object: &VectorObject,
        frame: u32,
        scale: f32,
    ) -> bool {
        if !(object.is_instance() || object.image.is_some()) || object.clip_source.is_some() {
            return false;
        }
        let key = (object.id, frame, scale);
        if self.rejected == Some(key) {
            return false;
        }
        let ready = !renderer::requires_scene_compositing(doc)
            && self.try_prepare(ctx, doc, object, frame, scale);
        self.rejected = (!ready).then_some(key);
        ready
    }

    pub(super) fn begin_gesture(&mut self) {
        self.rejected = None;
    }

    fn try_prepare(
        &mut self,
        ctx: &egui::Context,
        doc: &Arc<Document>,
        object: &VectorObject,
        frame: u32,
        scale: f32,
    ) -> bool {
        self.released = None;
        let same_sprite = self
            .sprite
            .as_ref()
            .is_some_and(|s| s.frame == frame && s.scale == scale && s.original == *object);
        if same_sprite
            && self
                .backdrop
                .as_ref()
                .is_some_and(|b| Arc::ptr_eq(&b.source, doc) || b.source.as_ref() == doc.as_ref())
        {
            return true;
        }
        if !same_sprite
            || self
                .sprite
                .as_ref()
                .and_then(Sprite::refresh_density)
                .is_some()
        {
            if let Some(cached) = self.sprites.find(object, frame, scale) {
                let mut sprite = cached.clone();
                sprite.original.clone_from(object);
                sprite.frame = frame;
                self.sprite = Some(sprite);
            } else {
                let Some(sprite) = renderer::move_sprite(object, frame, scale) else {
                    return false;
                };
                if !self.install(ctx, object, frame, sprite) {
                    return false;
                }
            }
        }
        let objects = doc.paint_order();
        let Some(slot) = objects.iter().position(|o| o.id == object.id) else {
            return false;
        };
        if let Some(view) = self.visible {
            // A grab immediately after scrubbing can beat the idle worker. Draw
            // each missing clip pose once in its cropped surface; flattening all
            // its duplicates into one viewport-sized layer is much more expensive.
            let clips = objects
                .iter()
                .rev()
                .take(128)
                .copied()
                .filter(|o| o.id != object.id && o.clip_instance.is_some());
            for sibling in self.sprites.missing(clips, frame, scale, view, None) {
                if let Some(sprite) = renderer::move_sprite(sibling, frame, scale)
                    .and_then(|image| Sprite::upload(ctx, sibling, frame, image))
                {
                    self.sprites.remember(&sprite);
                }
            }
        }
        let first = objects[..slot]
            .iter()
            .position(|o| self.sprites.find(o, frame, scale).is_some())
            .unwrap_or(slot);
        let Some(under) =
            self.sprites
                .stack(ctx, &objects[first..slot], frame, scale, self.visible)
        else {
            return false;
        };
        let Some(above) = self
            .sprites
            .stack(ctx, &objects[slot + 1..], frame, scale, self.visible)
        else {
            return false;
        };
        let mut background = doc.as_ref().clone();
        let below: std::collections::HashSet<_> = objects[..first].iter().map(|o| o.id).collect();
        background.objects.retain(|o| below.contains(&o.id));
        self.backdrop = Some(Backdrop {
            source: Arc::clone(doc),
            below: Arc::new(background),
            under,
            above,
        });
        true
    }

    pub(super) fn backdrop(&self) -> Arc<Document> {
        self.backdrop.as_ref().expect("prepared move").below.clone()
    }

    pub(super) fn paint(&self, painter: &egui::Painter, view: View, pose: Option<&VectorObject>) {
        let sprite = self.sprite.as_ref().expect("prepared move");
        let corners = sprite
            .corners_at(pose.unwrap_or(&sprite.original), sprite.frame)
            .map(|p| view.screen(p));
        let stack = self.backdrop.as_ref().expect("prepared placement layers");
        if stack.under.is_empty() && stack.above.is_empty() {
            painter.add(display_filter::affine_image(
                painter,
                sprite.image.texture.id(),
                corners,
            ));
            return;
        }
        let mut layers = Vec::with_capacity(stack.under.len() + 1 + stack.above.len());
        layers.extend(stack.under.iter().map(|layer| layer.at_view(view)));
        layers.push(display_filter::Image::affine(
            sprite.image.texture.id(),
            corners,
        ));
        layers.extend(stack.above.iter().map(|layer| layer.at_view(view)));
        painter.add(display_filter::batch(painter, layers));
    }

    pub(super) fn release(&mut self, document: &Arc<Document>, frame: u32, id: u64) {
        let (Some(sprite), Some(backdrop)) = (&mut self.sprite, &mut self.backdrop) else {
            return;
        };
        if sprite.original.id != id || sprite.frame != frame {
            return;
        }
        let Some(object) = document.objects.iter().find(|o| o.id == id) else {
            return;
        };
        sprite.original.clone_from(object);
        backdrop.source = document.clone();
        self.released = Some(document.clone());
    }
    pub(super) fn can_release(&self, source: &Document, frame: u32, id: u64) -> bool {
        self.sprite
            .as_ref()
            .is_some_and(|s| s.original.id == id && s.frame == frame)
            && self.backdrop.as_ref().is_some_and(|b| {
                std::ptr::eq(b.source.as_ref(), source) || b.source.as_ref() == source
            })
    }

    pub(super) fn cancel(&mut self) {
        self.released = None;
        self.backdrop = None;
    }

    // Only the matching committed page can replace the displayed layers. A
    // second move can present its latest pose while the first render finishes.
    pub(super) fn settle(
        &mut self,
        ctx: &egui::Context,
        current: &Document,
        frame: u32,
        scale: f32,
    ) -> Settle {
        let completed = self.worker.as_ref().and_then(|job| job.completed());
        let finished = completed.and_then(|result| self.worker.take().map(|job| (job, result)));
        let Some(released) = self.released.as_ref().filter(|d| {
            self.sprite.as_ref().is_some_and(|s| s.frame == frame)
                && (std::ptr::eq(d.as_ref(), current) || d.as_ref() == current)
        }) else {
            self.cancel();
            return Settle::PaintNormally;
        };
        if let Some((_, result)) = finished.filter(|(w, _)| w.is_target(released, frame, scale)) {
            self.cancel();
            let Ok((page, refreshed)) = result else {
                return Settle::PaintNormally;
            };
            if let Some(sprite) = refreshed {
                let id = self
                    .sprite
                    .as_ref()
                    .expect("settling placement")
                    .original
                    .id;
                if let Some(object) = current.objects.iter().find(|o| o.id == id) {
                    self.install(ctx, object, frame, sprite);
                }
            }
            return Settle::Finished(page);
        }
        if self.worker.is_none() {
            let refresh = self.sprite.as_ref().and_then(Sprite::refresh_density);
            self.worker =
                raster::Refinement::settled_transform(ctx, released, frame, scale, refresh);
        }
        if self.worker.is_some() {
            Settle::KeepLayers
        } else {
            self.cancel();
            Settle::PaintNormally
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn scene() -> Arc<Document> {
        let mut src = Document::default();
        src.add_rectangle([-8., 0.], [34., 24.], [20, 200, 150, 255]);
        let mut scene = Document::default();
        scene.add_rectangle([0., 0.], [96., 96.], [44, 40, 77, 255]);
        scene.add_rectangle([8., 8.], [128., 128.], [0; 4]);
        scene.objects[1].embedded_artwork = Some(Arc::new(src).into());
        scene.add_rectangle([12., 16.], [22., 42.], [220, 70, 30, 144]);
        Arc::new(scene)
    }

    fn shift(doc: &Arc<Document>, delta: [f32; 2]) -> Arc<Document> {
        let mut copy = doc.as_ref().clone();
        let [x, y] = copy.objects[1].position;
        copy.objects[1].position = [x + delta[0], y + delta[1]];
        Arc::new(copy)
    }

    fn wait_settle(
        cache: &mut AssetMove,
        ctx: &egui::Context,
        doc: &Arc<Document>,
        frame: u32,
        scale: f32,
    ) -> Settle {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let result = cache.settle(ctx, doc, frame, scale);
            if !matches!(result, Settle::KeepLayers) {
                return result;
            }
            assert!(Instant::now() < until, "move did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn overlapping_move_then_second_release_uses_latest_exact_pixels() {
        let ctx = egui::Context::default();
        let before = scene();
        let first = shift(&before, [42., 5.]);
        let second = shift(&first, [-58., 2.]);
        let id = before.objects[1].id;
        let mut cache = AssetMove::default();
        assert!(cache.prepare(&ctx, &before, &before.objects[1], 0, 1.));
        let lower = cache.backdrop();
        assert_eq!(lower.objects.len(), 1);
        assert_eq!(lower.objects[0], before.objects[0]);
        assert!(!cache.backdrop.as_ref().unwrap().above.is_empty());
        let sprite_id = cache.sprite.as_ref().unwrap().image.texture.id();
        cache.release(&first, 0, id);
        assert!(matches!(
            cache.settle(&ctx, &first, 0, 1.),
            Settle::KeepLayers
        ));
        assert!(cache.prepare(&ctx, &first, &first.objects[1], 0, 1.));
        assert_eq!(sprite_id, cache.sprite.as_ref().unwrap().image.texture.id());
        cache.release(&second, 0, id);
        let Settle::Finished((pixels, _)) = wait_settle(&mut cache, &ctx, &second, 0, 1.) else {
            panic!("expected the exact page");
        };
        let exact = renderer::render(&second, 0).unwrap();
        assert_eq!(
            pixels,
            ColorImage::from_rgba_premultiplied([128; 2], exact.data())
        );
    }

    #[test]
    fn transformed_sprite_preserves_padding_mirrors_and_animated_pivot() {
        let ctx = egui::Context::default();
        let mut scene = scene();
        let frame = 2;
        let object = &mut Arc::make_mut(&mut scene).objects[1];
        object.pivot = Some([0.2, 0.8]);
        object.animated.skew = 12.;
        object.scale_keys.insert(frame, [-0.9, 0.8]);
        object.rotation_keys.insert(frame, 19.);
        let object = &scene.objects[1];
        let mut cache = AssetMove::default();
        assert!(cache.prepare(&ctx, &scene, object, frame, 1.));
        let sprite = cache.sprite.as_ref().unwrap();
        let captured = sprite.image.bounds;
        let texture = sprite.image.texture.id();
        let mut posed = scene.as_ref().clone();
        let changed = &mut posed.objects[1];
        changed.scale_keys.insert(frame, [-2.7, 1.3]);
        changed.rotation_keys.insert(frame, 106.);
        changed.set_position_at(frame, [-14., 20.]);
        let transform = object
            .symbol_transform(frame)
            .unwrap()
            .invert()
            .unwrap()
            .post_concat(changed.symbol_transform(frame).unwrap());
        for (corner, preview) in [
            captured.left_top(),
            captured.right_top(),
            captured.left_bottom(),
            captured.right_bottom(),
        ]
        .into_iter()
        .zip(sprite.corners_at(changed, frame))
        {
            let mut expected = tiny_skia::Point::from_xy(corner.x, corner.y);
            transform.map_point(&mut expected);
            assert!(preview.distance(egui::pos2(expected.x, expected.y)) < 0.001);
        }
        let posed = Arc::new(posed);
        cache.release(&posed, frame, object.id);
        let Settle::Finished((page, _)) = wait_settle(&mut cache, &ctx, &posed, frame, 1.) else {
            panic!("missing final page");
        };
        let exact = renderer::render(&posed, frame).unwrap();
        assert_eq!(
            page,
            ColorImage::from_rgba_premultiplied([128; 2], exact.data())
        );
        let refreshed = cache.sprite.as_ref().unwrap();
        assert_ne!(refreshed.image.texture.id(), texture);
        assert_eq!(
            refreshed.captured_size,
            posed.objects[1].scaled_box_at(frame).1
        );
        assert!(cache.prepare(&ctx, &posed, &posed.objects[1], frame, 1.));
    }

    #[test]
    fn a_tiny_copy_keeps_the_large_source_texture_for_other_placements() {
        let ctx = egui::Context::default();
        let mut drawing = scene();
        let large = drawing.objects[1].clone();
        let mut tiny = large.clone();
        tiny.id = 42;
        tiny.size = [8., 6.];
        tiny.rotation = 25.;
        Arc::make_mut(&mut drawing).objects.push(tiny.clone());
        let mut cache = AssetMove::default();
        cache.install(&ctx, &tiny, 0, renderer::move_sprite(&tiny, 0, 1.).unwrap());
        let small_id = cache.sprite.as_ref().unwrap().image.texture.id();
        assert!(cache.sprites.find(&large, 0, 1.).is_none());
        cache.install(
            &ctx,
            &large,
            0,
            renderer::move_sprite(&large, 0, 1.).unwrap(),
        );
        let large_id = cache.sprite.as_ref().unwrap().image.texture.id();
        assert_ne!(large_id, small_id);
        assert_eq!(
            cache
                .sprites
                .find(&large, 0, 1.)
                .unwrap()
                .image
                .texture
                .id(),
            large_id
        );
        assert_eq!(
            cache.sprites.find(&tiny, 0, 1.).unwrap().image.texture.id(),
            small_id
        );
        assert!(cache.prepare(&ctx, &drawing, &large, 0, 1.));
        assert_eq!(cache.sprite.as_ref().unwrap().image.texture.id(), large_id);
        let stack = cache.backdrop.as_ref().unwrap();
        assert_eq!(stack.under.len(), 0);
        // Translucent artwork and then the tiny placed copy are above the target.
        assert_eq!(stack.above.len(), 2);
    }

    #[test]
    fn immediate_drag_after_scrubbing_shares_the_current_sibling_pose() {
        let mut project = document_core::AnimationProject::default();
        let source = project.add_clip("Actor").unwrap();
        project.clips[source].set_length(3).unwrap();
        project.clips[source].drawing_at_mut(0).add_rectangle(
            [8., 8.],
            [16., 16.],
            [110, 180, 30, 255],
        );
        let sibling = project.add_clip("Extra").unwrap();
        project.clips[sibling].set_length(3).unwrap();
        project.clips[sibling].drawing_at_mut(0).add_rectangle(
            [8., 8.],
            [16., 16.],
            [40, 90, 220, 128],
        );
        for (i, src) in [source, sibling, sibling].into_iter().enumerate() {
            project
                .place_clip(project.clips[src].id, 0, 0, 1, [20. * i as f32, 0.])
                .unwrap();
        }
        let doc = &project.clips[0].cels[&0];
        let ctx = egui::Context::default();
        let mut cache = AssetMove::default();
        cache.set_view(Rect::from_min_max(Pos2::ZERO, egui::pos2(256., 256.)));
        let mut last = None;
        for tick in [0, 1] {
            assert!(cache.prepare(&ctx, doc, &doc.objects[0], tick, 1.));
            let layers = &cache.backdrop.as_ref().unwrap().above;
            assert_eq!(layers.len(), 2);
            let texture = layers[0].texture.id();
            assert_eq!(texture, layers[1].texture.id());
            assert_ne!(last, Some(texture));
            last = Some(texture);
        }
    }

    #[test]
    fn warming_a_cold_scene_reuses_one_sample_for_duplicates_and_skips_hidden_sources() {
        let ctx = egui::Context::default();
        let mut drawing = scene();
        let mut copy = drawing.objects[1].clone();
        copy.id = 31;
        copy.position = [31.; 2];
        copy.size = [8.; 2];
        let mut hidden = copy.clone();
        hidden.id = 32;
        hidden.opacity = 0.;
        Arc::make_mut(&mut drawing).objects.extend([copy, hidden]);
        let mut cache = AssetMove::default();
        let visible = Rect::from_min_max(Pos2::ZERO, egui::pos2(128., 128.));
        cache.warm_scene(&ctx, &drawing, 0, 1., Some(31), visible);
        let until = Instant::now() + Duration::from_secs(5);
        while cache.sprites.find(&drawing.objects[1], 0, 1.).is_none() {
            assert!(Instant::now() < until);
            cache.poll_warm(&ctx, true);
            std::thread::sleep(Duration::from_millis(2));
        }
        let id = cache
            .sprites
            .find(&drawing.objects[1], 0, 1.)
            .unwrap()
            .image
            .texture
            .id();
        assert_eq!(
            id,
            cache
                .sprites
                .find(&drawing.objects[3], 0, 1.)
                .unwrap()
                .image
                .texture
                .id()
        );
        assert!(cache.sprites.find(&drawing.objects[4], 0, 1.).is_none());
    }

    #[test]
    fn changing_the_view_rebuilds_visible_layers_and_culls_distant_copies() {
        let ctx = egui::Context::default();
        let mut drawing = scene();
        let object = drawing.objects[1].clone();
        let mut far = object.clone();
        far.id = 44;
        far.position = [3010.; 2];
        Arc::make_mut(&mut drawing).objects.push(far);
        let mut cache = AssetMove::default();
        for origin in [0., 3000.] {
            cache.set_view(Rect::from_min_size(
                egui::pos2(origin, origin),
                egui::vec2(100., 100.),
            ));
            assert!(!cache.can_release(&drawing, 0, object.id));
            assert!(cache.prepare(&ctx, &drawing, &object, 0, 1.));
            let stack = cache.backdrop.as_ref().unwrap();
            assert_eq!(stack.above.len(), 1);
            // The near viewport contains the regular translucent rectangle. The
            // far viewport instead contains only the independently placed copy.
            assert_eq!(
                stack.above[0].texture.id() == cache.sprite.as_ref().unwrap().image.texture.id(),
                origin > 0.
            );
        }
    }

    #[test]
    fn nonfinite_inverse_of_raster_padding_falls_back_before_upload() {
        let ctx = egui::Context::default();
        let mut object = scene().objects[1].clone();
        object.size = [f32::from_bits(2), 1.];
        let raster = renderer::MoveSprite {
            origin: [-2., -2.],
            scale: 1.,
            requested_scale: 1.,
            pixels: renderer::Pixmap::new(4, 4).unwrap(),
        };
        assert!(Sprite::upload(&ctx, &object, 0, raster).is_none());
    }

    #[test]
    fn dense_placement_reuses_only_the_matching_previous_page() {
        let ctx = egui::Context::default();
        let mut base = Document::default();
        for i in 0..512 {
            base.add_rectangle(
                [(i % 30) as f32 * 3., (i / 30) as f32 * 3.],
                [2.; 2],
                [90, 60, 120, 200],
            );
        }
        let placement = scene().objects[1].embedded_artwork.clone();
        for case in ["same", "different ink", "above", "density", "frame", "park"] {
            let mut canvas = Canvas::default();
            let previous = canvas.frame_texture(&ctx, &base, 0, 1.).unwrap();
            let resolution = canvas.placement_resolution(&ctx).unwrap();
            let mut drawing = base.clone();
            let id = drawing.add_rectangle([12.; 2], [64.; 2], [0; 4]);
            drawing.objects.last_mut().unwrap().embedded_artwork = placement.clone();
            if case == "above" {
                // Both the old page and the new drawing contain this translucent
                // shape. The original page cannot serve as everything below the move.
                let end = drawing.objects.len() - 1;
                drawing.objects.swap(end - 1, end);
            }
            let mut drawing = Arc::new(drawing);
            let page =
                raster::placement::PlacementPage::render(&drawing, 0, id, resolution).unwrap();
            canvas.install_placement_page(&ctx, page);
            assert!(canvas.asset_move.before_placement.is_some());
            if case == "park" {
                canvas.park();
                assert!(canvas.asset_move.before_placement.is_none());
                continue;
            }
            if case == "different ink" {
                Arc::make_mut(&mut drawing).objects[0].fill = [255; 4];
            }
            let object = drawing.objects.iter().find(|o| o.id == id).unwrap();
            assert!(canvas.asset_move.prepare(&ctx, &drawing, object, 0, 1.));
            let frame = u32::from(case == "frame");
            let scale = if case == "density" { 2. } else { 1. };
            let reused = canvas.asset_move.take_placement_background(frame, scale);
            assert_eq!(
                reused.map(|page| page.handle.id()),
                (case == "same").then_some(previous)
            );
        }
    }

    #[test]
    fn unsupported_foreground_is_reconsidered_on_the_next_press() {
        let ctx = egui::Context::default();
        let mut drawing = scene();
        Arc::make_mut(&mut drawing).objects[2].size = [5000., 100.];
        let mut cache = AssetMove::default();
        assert!(!cache.prepare(&ctx, &drawing, &drawing.objects[1], 0, 1.));
        Arc::make_mut(&mut drawing).objects[2].size = [22., 42.];
        assert!(!cache.prepare(&ctx, &drawing, &drawing.objects[1], 0, 1.));
        cache.begin_gesture();
        assert!(cache.prepare(&ctx, &drawing, &drawing.objects[1], 0, 1.));
    }

    #[test]
    fn undo_frame_changes_and_renderer_failure_return_to_normal_painting() {
        let ctx = egui::Context::default();
        let before = scene();
        let after = shift(&before, [4., 2.]);
        let mut cache = AssetMove::default();
        for (current, frame, scale) in [(&before, 0, 1.), (&after, 1, 1.), (&after, 0, 0.)] {
            let object = &before.objects[1];
            assert!(cache.prepare(&ctx, &before, object, 0, 1.));
            cache.release(&after, 0, object.id);
            assert!(matches!(
                cache.settle(&ctx, &after, 0, scale),
                Settle::KeepLayers
            ));
            assert!(matches!(
                wait_settle(&mut cache, &ctx, current, frame, scale),
                Settle::PaintNormally
            ));
            assert!(cache.released.is_none());
            assert!(cache.backdrop.is_none());
        }
    }
}
