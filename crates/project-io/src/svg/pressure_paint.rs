use document_core::{Document, GradientKind, Shape, VectorObject, MAX_DIMENSION};
use std::io;

/// The exported center-out pressure stroke is a gradient rectangle clipped
/// to its vector silhouette. Its paint box can differ from the silhouette's.
pub(super) fn plane(o: &VectorObject) -> bool {
    !o.is_instance()
        && !o.no_fill
        && matches!(o.shape, Shape::Rectangle)
        && !o.has_stroke_at(0)
        && o.gradient.as_ref().is_some_and(|g| {
            matches!(
                g.kind,
                GradientKind::Radial { .. } | GradientKind::Rectangular { .. }
            )
        })
        && o.clip_source.is_none()
        && o.stencil
            .as_ref()
            .is_some_and(|s| s.alpha.is_none() && !s.shape.contours().is_empty())
}

pub(super) fn paints(objects: &[VectorObject], allow_unclipped_outline: bool) -> bool {
    match objects {
        [stroke] => plane(stroke),
        [fill, stroke] => {
            let filled_outline = allow_unclipped_outline
                && matches!(
                    stroke.shape,
                    Shape::Path { closed: true, .. } | Shape::Compound { .. }
                )
                && !stroke.is_instance()
                && stroke.stencil.is_none()
                && !stroke.has_stroke_at(0);
            fill.layer == stroke.layer
                && fill.group == stroke.group
                && !fill.is_instance()
                && fill.stencil.is_none()
                && !fill.has_stroke_at(0)
                && (plane(stroke) || filled_outline)
        }
        _ => false,
    }
}

/// Only tag a group we can emit as exactly the visible, ordered paints that
/// the reader will isolate. Other detached artwork uses regular SVG groups.
pub(super) fn exported_composite(art: &Document, frame: u32) -> bool {
    art.layers.len() == 1
        && art.layers[0].visible
        && art
            .objects
            .iter()
            .all(|o| o.group.is_none() && o.visible_at(frame))
        && paints(&art.objects, true)
}

/// Isolate the paints before applying the outer stencil or group opacity.
/// A detached placement keeps each child vector and its original paint box.
pub(super) fn composite(mut objects: Vec<VectorObject>, opacity: f32) -> io::Result<VectorObject> {
    let (id, layer, group) = (objects[0].id, objects[0].layer, objects[0].group);
    let mut bounds = objects[0].bounds(0);
    for o in &objects[1..] {
        let [l, t, r, b] = o.bounds(0);
        bounds = [
            bounds[0].min(l),
            bounds[1].min(t),
            bounds[2].max(r),
            bounds[3].max(b),
        ];
    }
    let origin = [bounds[0].floor(), bounds[1].floor()];
    // Detached artwork can extend past its page. Keep the placement 1:1
    // even for a pressure paint plane larger than the maximum page size.
    let [width, height] =
        [0, 1].map(|i| (bounds[i + 2].ceil() - origin[i]).clamp(1., MAX_DIMENSION as f32) as u32);
    for (index, o) in objects.iter_mut().enumerate() {
        o.position[0] -= origin[0];
        o.position[1] -= origin[1];
        o.layer = 1;
        o.id = index as u64 + 1;
        o.group = None;
        o.group_name = None;
    }
    let artwork = Document {
        width,
        height,
        frames: 1,
        next_id: objects.len() as u64 + 1,
        objects,
        ..Document::default()
    };
    artwork.validate().map_err(crate::invalid)?;
    let mut placement = Document::default();
    placement.add_rectangle(origin, [width as f32, height as f32], [0; 4]);
    let mut object = placement.objects.pop().unwrap();
    object.id = id;
    object.layer = layer;
    object.group = group;
    object.opacity = opacity;
    object.embedded_artwork = Some(std::sync::Arc::new(artwork).into());
    Ok(object)
}
