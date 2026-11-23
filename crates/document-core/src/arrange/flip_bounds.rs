use super::union;
use crate::{FillRule, VectorObject};
use tiny_skia_path::{Path, PathBuilder, Rect, Transform};

const EMPTY: [f32; 4] = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
type Mask = (Path, FillRule);

pub(super) fn measure(object: &VectorObject, frame: u32) -> [f32; 4] {
    if !object.is_instance() && object.stencil.is_none() {
        return object.bounds(frame);
    }
    ink(object, frame, Transform::identity(), &mut Vec::new())
        .unwrap_or_else(|| object.bounds(frame))
}

fn ink(
    object: &VectorObject,
    frame: u32,
    parent: Transform,
    masks: &mut Vec<Mask>,
) -> Option<[f32; 4]> {
    if !object.visible_at(frame) || object.opacity_at(frame) <= 0. {
        return Some(EMPTY);
    }
    let prior = masks.len();
    if let Some(stencil) = &object.stencil {
        if stencil.alpha == Some([0, 0]) {
            return Some(EMPTY);
        }
        let space = object.paint_space(frame)?.post_concat(parent);
        masks.push((stencil.outline(space)?, stencil.fill_rule));
    }
    let mut result = EMPTY;
    if let Some((art, tick)) = object.instance_at(frame) {
        let transform = object.symbol_transform(frame)?.post_concat(parent);
        if object.clip_instance.is_some() {
            let page = Rect::from_xywh(0., 0., art.width as f32, art.height as f32)?;
            masks.push((
                PathBuilder::from_rect(page).transform(transform)?,
                FillRule::NonZero,
            ));
        }
        // Transform the actual child ink; rotating all four corners of its
        // bounding box would shift the pivot of asymmetric artwork.
        for member in art.paint_order() {
            result = union(result, ink(member, tick, transform, masks)?);
        }
    } else if let Some(outline) = object.outline(frame) {
        if let Some(path) = object.stroked(&outline, frame) {
            result = clipped(&path.transform(parent)?, FillRule::NonZero, masks);
        }
        if object.has_fill_at(frame) {
            result = union(
                result,
                clipped(&outline.transform(parent)?, object.fill_rule, masks),
            );
        }
    }
    masks.truncate(prior);
    Some(result)
}

fn clipped(path: &Path, rule: FillRule, masks: &[Mask]) -> [f32; 4] {
    let tight = |p: &Path| {
        let b = p.compute_tight_bounds().unwrap_or_else(|| p.bounds());
        [b.left(), b.top(), b.right(), b.bottom()]
    };
    let bounds = tight(path);
    if masks.is_empty() {
        return bounds;
    }
    let limit = masks.iter().fold(bounds, |a, (mask, _)| {
        let b = tight(mask);
        [
            a[0].max(b[0]),
            a[1].max(b[1]),
            a[2].min(b[2]),
            a[3].min(b[3]),
        ]
    });
    if limit[0] >= limit[2] || limit[1] >= limit[3] {
        return EMPTY;
    }
    // Retain a conservative clipped extent beyond the Boolean complexity limit.
    match crate::boolean::stencil_intersection_bounds(path, rule, masks) {
        Ok(Some(b)) => b,
        Ok(None) => EMPTY,
        Err(_) => limit,
    }
}
