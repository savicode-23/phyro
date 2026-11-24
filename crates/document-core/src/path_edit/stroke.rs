use crate::{
    arrange::is_object_editable, geometry::editable_contours, Document, FillRule, Gradient,
    GradientKind, Shape, VectorObject, MAX_OBJECT_NAME_CHARS,
};
use std::collections::HashSet;
use tiny_skia_path::{Point, Transform};

const STROKE_NAME_SUFFIX: &str = " stroke";

pub fn has_convertible_stroke(object: &VectorObject, frame: u32) -> bool {
    !object.is_instance() && object.image.is_none() && object.has_stroke_at(frame)
}

/// Expand visible strokes in paint order. Retain the original fill immediately
/// below its outline and keep its ID, or reuse that ID for stroke-only artwork.
pub fn stroke_to_paths(
    doc: &mut Document,
    selection: &[u64],
    frame: u32,
) -> Result<Vec<u64>, String> {
    let selected: HashSet<_> = selection.iter().copied().collect();
    let mut edits = Vec::new();
    for (i, object) in doc.objects.iter().enumerate() {
        if selected.contains(&object.id)
            && is_object_editable(doc, object)
            && object.visible_at(frame)
            && has_convertible_stroke(object, frame)
        {
            edits.push((i, stroke_path(object, frame)?, object.has_fill_at(frame)));
        }
    }
    if edits.is_empty() {
        return Ok(Vec::new());
    }

    let mut staged = doc.clone();
    let mut ids = Vec::new();
    for (i, mut outline, keep_fill) in edits.into_iter().rev() {
        if keep_fill {
            let fill = &mut staged.objects[i];
            clear_stroke(fill);
            outline.name = fill
                .display_name()
                .chars()
                .take(MAX_OBJECT_NAME_CHARS - STROKE_NAME_SUFFIX.len())
                .chain(STROKE_NAME_SUFFIX.chars())
                .collect();
            outline.id = staged.add_shape(
                outline.position,
                outline.size,
                outline.fill,
                Shape::Rectangle,
                outline.layer,
            );
            staged.objects.pop();
            ids.extend([outline.id, staged.objects[i].id]);
            staged.objects.insert(i + 1, outline);
        } else {
            ids.push(outline.id);
            staged.objects[i] = outline;
        }
    }
    staged.validate()?;
    *doc = staged;
    ids.reverse();
    Ok(ids)
}

fn clear_stroke(object: &mut VectorObject) {
    object.no_stroke = true;
    object.stroke_width = 0.;
    object.stroke_gradient = None;
    object.stroke_blend_mode = Default::default();
    object.stroke_profile.clear();
}

fn stroke_path(object: &VectorObject, frame: u32) -> Result<VectorObject, String> {
    if object.last_keyframe().is_some()
        || object.motion.is_some()
        || object.skin.is_some()
        || object.rigid_bone.is_some()
        || object.stencil.is_some()
        || object.clip_source.is_some()
    {
        return Err(format!(
            "Stroke to path requires static artwork without clipping or rig bindings. {} still uses one of those features.",
            object.display_name()
        ));
    }

    let path = object
        .outline(frame)
        .and_then(|path| object.stroked(&path, frame))
        .ok_or("The stroke has no outline to convert.")?;
    let local = object.world_to_local_at(frame);
    let mut contours = editable_contours(&path, |p| local([p.x, p.y]))
        .ok_or("Could not convert the stroke outline.")?;
    // The stroker can emit degenerate contours with no painted area.
    contours.retain(|nodes| nodes.len() >= 3);
    let shape = if contours.len() == 1 {
        Shape::Path {
            nodes: contours.pop().unwrap(),
            closed: true,
        }
    } else {
        Shape::Compound { contours }
    };
    let mut result = object.clone();
    result.shape = shape;
    result.text = None;
    result.effects.clear();
    result.fill_rule = FillRule::NonZero;
    result.no_fill = false;
    result.fill = object.stroke_at(frame);
    result.fill_blend_mode = object.stroke_blend_mode;
    result.gradient = object
        .stroke_gradient
        .as_ref()
        .map(|gradient| {
            let old = object
                .stroke_paint_space(frame)
                .ok_or("Invalid stroke gradient bounds.")?;
            let new = result
                .paint_space(frame)
                .ok_or("Invalid outline gradient bounds.")?;
            remap_gradient(gradient, old, new)
        })
        .transpose()?;
    clear_stroke(&mut result);
    Ok(result)
}

fn remap_gradient(gradient: &Gradient, old: Transform, new: Transform) -> Result<Gradient, String> {
    let from_new = new.post_concat(old.invert().ok_or("Invalid stroke gradient transform.")?);
    let to_new = from_new
        .invert()
        .ok_or("Invalid outline gradient transform.")?;
    let map = |[x, y]: [f32; 2]| {
        let mut point = Point::from_xy(x, y);
        to_new.map_point(&mut point);
        [point.x, point.y]
    };
    let kind = match gradient.kind {
        GradientKind::Linear { start, end } => {
            // Preserve the gradient's rate of change on each axis as well as its origin.
            let [x, y] = [end[0] - start[0], end[1] - start[1]];
            let square = x * x + y * y;
            let dx = (from_new.sx * x + from_new.ky * y) / square;
            let dy = (from_new.kx * x + from_new.sy * y) / square;
            let inverse_square = 1. / (dx * dx + dy * dy);
            let start = map(start);
            let end = [
                start[0] + dx * inverse_square,
                start[1] + dy * inverse_square,
            ];
            GradientKind::Linear { start, end }
        }
        GradientKind::Radial { center, radius } | GradientKind::Rectangular { center, radius } => {
            let sx = from_new.sx;
            if (sx - from_new.sy).abs() > sx.abs() * 1e-5 {
                return Err("This center-out stroke gradient needs different scales on each axis to preserve its appearance.".into());
            }
            let center = map(center);
            let radius = radius / sx;
            if matches!(gradient.kind, GradientKind::Radial { .. }) {
                GradientKind::Radial { center, radius }
            } else {
                GradientKind::Rectangular { center, radius }
            }
        }
    };
    Ok(Gradient {
        kind,
        stops: gradient.stops.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filled_object_with_a_maximum_length_name_keeps_its_fill_and_outline() {
        let mut doc = Document::default();
        let id = doc.add_rectangle([10.; 2], [40.; 2], [255; 4]);
        let name = "ä".repeat(MAX_OBJECT_NAME_CHARS);
        doc.objects[0].name = name.clone();
        doc.objects[0].stroke_width = 4.;

        let ids = stroke_to_paths(&mut doc, &[id], 0).unwrap();
        assert_eq!(ids, [id, doc.objects[1].id]);
        assert_eq!(doc.objects[0].name, name);
        assert_eq!(doc.objects[1].name.chars().count(), MAX_OBJECT_NAME_CHARS);
        assert!(doc.objects[1].name.ends_with(STROKE_NAME_SUFFIX));
        assert!(doc.validate().is_ok());
    }
}
