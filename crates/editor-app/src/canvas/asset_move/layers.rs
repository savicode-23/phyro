use super::{display_filter, egui, Image, Pos2, Rect, Sprite, VectorObject, View};
use std::{collections::VecDeque, sync::Arc};

pub(super) struct Layer {
    pub(super) texture: egui::TextureHandle,
    corners: [Pos2; 4],
}

impl Layer {
    pub(super) fn at_view(&self, view: View) -> display_filter::Image {
        display_filter::Image::affine(self.texture.id(), self.corners.map(|p| view.screen(p)))
    }
}

const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 24;

#[derive(Default)]
pub(super) struct Sprites(VecDeque<Sprite>);

impl Sprites {
    // Capture the largest visible representative when several placements share
    // the same pose. Keep separate candidates for very different aspect ratios.
    pub(super) fn missing<'a>(
        &self,
        objects: impl Iterator<Item = &'a VectorObject>,
        frame: u32,
        scale: f32,
        area: Rect,
        selected: Option<u64>,
    ) -> Vec<&'a VectorObject> {
        let mut targets = Vec::<&VectorObject>::new();
        for object in objects {
            if !(object.is_instance() || object.image.is_some())
                || !object.visible_at(frame)
                || object.opacity_at(frame) <= 0.
                || !same_sample(object, frame, object, frame)
                || self.find(object, frame, scale).is_some()
            {
                continue;
            }
            let world = object.local_to_world_at(frame);
            let corners = [[0., 0.], [1., 0.], [0., 1.], [1., 1.]].map(|p| Pos2::from(world(p)));
            if Some(object.id) != selected && !area.intersects(Rect::from_points(&corners)) {
                continue;
            }
            let size = object.scaled_box_at(frame).1;
            let covers = |a: [f32; 2], b: [f32; 2]| a.into_iter().zip(b).all(|(a, b)| a >= b * 0.8);
            if targets.iter().any(|o| {
                same_sample(o, frame, object, frame) && covers(o.scaled_box_at(frame).1, size)
            }) {
                continue;
            }
            if let Some(smaller) = targets.iter_mut().find(|o| {
                same_sample(o, frame, object, frame) && covers(size, o.scaled_box_at(frame).1)
            }) {
                *smaller = object;
            } else if targets.len() < 6 {
                targets.push(object);
            }
        }
        targets
    }
    pub(super) fn remember(&mut self, sprite: &Sprite) {
        self.0.retain(|old| {
            !(old.scale == sprite.scale
                && old
                    .captured_size
                    .into_iter()
                    .zip(sprite.captured_size)
                    .all(|(a, b)| (0.85..1.18).contains(&(a / b)))
                && same_sample(&old.original, old.frame, &sprite.original, sprite.frame))
        });
        self.0.push_front(sprite.clone());
        let mut bytes: usize = self.0.iter().map(|s| s.image.texture.byte_size()).sum();
        while self.0.len() > CACHE_ENTRIES || bytes > CACHE_BYTES {
            let Some(last) = self.0.pop_back() else {
                break;
            };
            bytes -= last.image.texture.byte_size();
        }
    }

    pub(super) fn find(&self, object: &VectorObject, frame: u32, density: f32) -> Option<&Sprite> {
        if !(object.is_instance() || object.image.is_some())
            || object.clip_source.is_some()
            || !object.visible_at(frame)
            || object.opacity_at(frame) <= 0.
        {
            return None;
        }
        self.0
            .iter()
            .filter(|sprite| {
                sprite.scale == density
                    && same_sample(&sprite.original, sprite.frame, object, frame)
            })
            .filter_map(|s| {
                let error = quality_error(s, object, frame, density);
                (error <= 1.25).then_some((error, s))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
            .map(|(_, sprite)| sprite)
    }

    // Preserve the exact stacking order. Consecutive ordinary vectors and
    // shared masks form one static run between independently positioned clips.
    pub(super) fn stack(
        &self,
        ctx: &egui::Context,
        objects: &[&VectorObject],
        frame: u32,
        density: f32,
        visible: Option<Rect>,
    ) -> Option<Vec<Layer>> {
        let mut layers = Vec::new();
        let mut start = 0;
        let mut bytes = 0;
        for (index, object) in objects.iter().enumerate() {
            if let Some(sprite) = self.find(object, frame, density) {
                static_run(
                    ctx,
                    &objects[start..index],
                    frame,
                    density,
                    visible,
                    &mut layers,
                    &mut bytes,
                )?;
                let corners = sprite.corners_at(object, frame);
                if visible.is_none_or(|view| view.intersects(Rect::from_points(&corners))) {
                    layers.push(Layer {
                        corners,
                        texture: sprite.image.texture.clone(),
                    });
                }
                start = index + 1;
            }
        }
        static_run(
            ctx,
            &objects[start..],
            frame,
            density,
            visible,
            &mut layers,
            &mut bytes,
        )?;
        Some(layers)
    }
}

fn static_run(
    ctx: &egui::Context,
    objects: &[&VectorObject],
    frame: u32,
    density: f32,
    visible: Option<Rect>,
    layers: &mut Vec<Layer>,
    bytes: &mut usize,
) -> Option<()> {
    if objects
        .iter()
        .any(|o| o.visible_at(frame) && o.opacity_at(frame) > 0.)
    {
        let region = visible.map(|r| [r.left(), r.top(), r.right(), r.bottom()]);
        let Some(pixels) = renderer::foreground_in_region(objects, frame, density, region).ok()?
        else {
            return Some(());
        };
        *bytes += pixels.pixels.data().len();
        if *bytes > CACHE_BYTES {
            return None;
        }
        let image = Image::upload(ctx, "artwork above move", pixels);
        let rect = image.bounds;
        layers.push(Layer {
            corners: [
                rect.left_top(),
                rect.right_top(),
                rect.left_bottom(),
                rect.right_bottom(),
            ],
            texture: image.texture,
        });
    }
    Some(())
}

// Prefer enough texels in both axes. A higher-resolution copy can be sampled
// down; a ten-pixel capture stretched over a character cannot restore detail.
fn quality_error(sprite: &Sprite, object: &VectorObject, frame: u32, density: f32) -> f32 {
    let [tl, tr, bl, _] = sprite.corners_at(object, frame);
    let width = tl.distance(tr) * density;
    let height = tl.distance(bl) * density;
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return f32::INFINITY;
    }
    // Apply the same portable limit as the renderer. Asymmetrical scaling of
    // a rotated capture affects each texture axis differently.
    let cap = (renderer::MAX_SPRITE_EDGE as f32 / width.max(height)).min(1.);
    let [w, h] = sprite.image.texture.size();
    let quality = (w as f32 / (width * cap)).min(h as f32 / (height * cap));
    if quality >= 0.999 {
        (1. - 1. / quality).abs()
    } else {
        1. / quality
    }
}

pub(super) fn same_sample(
    original: &VectorObject,
    original_frame: u32,
    other: &VectorObject,
    frame: u32,
) -> bool {
    if original.opacity_at(original_frame) != other.opacity_at(frame)
        || original.stencil.is_some()
        || other.stencil.is_some()
        || original.clip_source.is_some()
        || other.clip_source.is_some()
    {
        return false;
    }
    if let (Some((a, at)), Some((b, bt))) = (
        original.instance_at(original_frame),
        other.instance_at(frame),
    ) {
        let viewport = |o: &VectorObject| {
            o.clip_instance
                .as_ref()
                .map(|clip| (clip.clip_id, clip.source_size))
        };
        return at == bt && viewport(original) == viewport(other) && std::ptr::eq(a, b);
    }
    let simple_image = |o: &VectorObject, tick| {
        o.shape == document_core::Shape::Rectangle
            && !o.has_stroke_at(tick)
            && o.effects.is_empty()
            && o.skin.is_none()
            && o.animated.geometry_keys.is_empty()
    };
    original
        .image
        .as_ref()
        .zip(other.image.as_ref())
        .is_some_and(|(a, b)| {
            Arc::ptr_eq(a, b)
                && simple_image(original, original_frame)
                && simple_image(other, frame)
        })
}
