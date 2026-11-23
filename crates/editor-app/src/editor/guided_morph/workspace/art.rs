use document_core::{Shape, VectorObject};
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, TextureHandle, Vec2};

pub(super) struct Thumbnail {
    pub(super) tick: u32,
    pub(super) shape: Shape,
    pub(super) texture: PoseTexture,
}

pub(super) struct Art {
    pub(super) thumbnails: Vec<Thumbnail>,
    pub(super) world: Rect,
    pub(super) current: PoseTexture,
    object: Option<VectorObject>,
}

impl Default for Art {
    fn default() -> Self {
        Self {
            thumbnails: Vec::new(),
            world: Rect::NOTHING,
            current: Default::default(),
            object: None,
        }
    }
}

impl Art {
    pub(super) fn prepare(
        &mut self,
        source: &VectorObject,
        poses: impl Iterator<Item = (u32, Shape)>,
    ) {
        // Keep transforms, materials and effects on a working copy. Geometry
        // comes from the staged tween until the artist confirms the edit.
        let mut object = source.clone();
        object.animated.geometry_keys.clear();
        object.animated.guided_geometry = None;
        self.world = Rect::NOTHING;
        for (tick, shape) in poses {
            object.shape.clone_from(&shape);
            if let Some(bounds) = painted_bounds(&object, tick) {
                self.world = self.world.union(bounds);
            }
            self.thumbnails.push(Thumbnail {
                tick,
                shape,
                texture: Default::default(),
            });
        }
        self.object = Some(object);
    }

    pub(super) fn paint_current(&mut self, ui: &egui::Ui, rect: Rect, frame: u32, shape: &Shape) {
        if let Some(object) = &mut self.object {
            self.current
                .paint(ui, rect, self.world, frame, shape, object);
        }
    }

    pub(super) fn paint_thumbnail(&mut self, ui: &egui::Ui, rect: Rect, index: usize) {
        if let (Some(object), Some(thumb)) = (&mut self.object, self.thumbnails.get_mut(index)) {
            thumb
                .texture
                .paint(ui, rect, self.world, thumb.tick, &thumb.shape, object);
        }
    }
}

fn painted_bounds(o: &VectorObject, frame: u32) -> Option<Rect> {
    let [x1, y1, x2, y2] = o.painted_shape_bounds(frame)?;
    Some(Rect::from_min_max(pos2(x1, y1), pos2(x2, y2)))
}

#[derive(Default)]
pub(super) struct PoseTexture {
    key: Option<(Vec2, f32, u32)>,
    texture: Option<TextureHandle>,
    painted: Option<Rect>,
}

impl PoseTexture {
    fn paint(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        world: Rect,
        frame: u32,
        shape: &Shape,
        object: &mut VectorObject,
    ) {
        if !world.is_finite() || !rect.is_positive() {
            return;
        }
        // The viewport is arbitrary; upload only pixels touched by the pose.
        // Bound high-DPI playback surfaces independently of the document size.
        let ppp = ui
            .ctx()
            .pixels_per_point()
            .min(1152. / rect.size().max_elem());
        let key = (rect.size(), ppp, frame);
        if self.key != Some(key) {
            self.key = Some(key);
            self.painted = None;
            object.shape.clone_from(shape);
            if let Some(ink) = painted_bounds(object, frame) {
                let fit = (rect.width() / world.width().max(0.01))
                    .min(rect.height() / world.height().max(0.01))
                    * 0.83;
                let origin = rect.size() * 0.5 - world.center().to_vec2() * fit;
                let visible = Rect::from_min_max(
                    (origin + ink.min.to_vec2() * fit).to_pos2(),
                    (origin + ink.max.to_vec2() * fit).to_pos2(),
                )
                .expand(2.)
                .intersect(Rect::from_min_size(Pos2::ZERO, rect.size()));
                if visible.is_positive() {
                    let start = (visible.min.to_vec2() * ppp).floor();
                    let end = (visible.max.to_vec2() * ppp).ceil();
                    let size = [(end.x - start.x) as u32, (end.y - start.y) as u32];
                    if let Some(pixels) = renderer::render_object_viewport(
                        object,
                        frame,
                        size,
                        ppp * fit,
                        (origin * ppp - start).into(),
                    ) {
                        let image = egui::ColorImage::from_rgba_premultiplied(
                            size.map(|n| n as usize),
                            pixels.data(),
                        );
                        if let Some(texture) = &mut self.texture {
                            texture.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(ui.ctx().load_texture(
                                "morph-art",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        self.painted = Some(Rect::from_min_size(
                            (start / ppp).to_pos2(),
                            vec2(size[0] as f32, size[1] as f32) / ppp,
                        ));
                    }
                }
            }
        }
        if let (Some(local), Some(texture)) = (self.painted, &self.texture) {
            ui.painter_at(rect).image(
                texture.id(),
                local.translate(rect.min.to_vec2()),
                Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                Color32::WHITE,
            );
        }
    }
}
