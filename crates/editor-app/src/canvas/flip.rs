use crate::timeline::KeyMode;
use document_core::{
    arrange::{plan_flip, Flip},
    Document, KeyValue, Property, VectorObject,
};

fn basis_tracks(o: &VectorObject, t: u32, flip: &Flip) -> impl Iterator<Item = Property> {
    [
        (Property::Scale, true),
        (Property::Rotation, flip.rotation != o.rotation_at(t)),
        (Property::Skew, flip.skew != o.skew_at(t)),
    ]
    .into_iter()
    .filter_map(|(property, changed)| changed.then_some(property))
}

fn apply(o: &mut VectorObject, t: u32, flip: &Flip, mode: KeyMode) -> bool {
    for property in basis_tracks(o, t, flip) {
        mode.prepare(o, t, property);
        match property {
            Property::Rotation => o.set_rotation_at(t, flip.rotation),
            Property::Scale => o.set_scale_at(t, flip.scale),
            Property::Skew => {
                mode.set(o, t, property, KeyValue::Scalar(flip.skew));
            }
            _ => unreachable!(),
        }
    }
    // Reflect about the selection bounds. The new basis may have already
    // moved the visible position of a member with its own pivot.
    if o.position_at(t) != flip.position {
        if !mode.prepare(o, t, Property::Position) {
            return false;
        }
        o.set_position_at(t, flip.position);
    }
    true
}

pub(crate) fn flip_selection(
    doc: &mut Document,
    ids: &[u64],
    frame: u32,
    horizontal: bool,
    mode: KeyMode,
) -> bool {
    let Ok(plan) = plan_flip(doc, ids, frame, horizontal) else {
        return false;
    };
    let mut staged = Vec::new();
    for flip in &plan {
        let o = &doc.objects[flip.index];
        if !basis_tracks(o, frame, flip).all(|p| mode.allows(o, frame, p)) {
            return false;
        }
        if o.skin.is_some() || o.pivot.is_some() {
            let mut next = o.clone();
            if !apply(&mut next, frame, flip, mode) {
                return false;
            }
            match next.skin_source_after_transform(o, frame) {
                Ok(Some(shape)) if mode.prepare(&mut next, frame, Property::Geometry) => {
                    if next.set_shape_at(frame, shape).is_err() {
                        return false;
                    }
                }
                Ok(None) => (),
                _ => return false,
            }
            staged.push((flip.index, next));
        } else if o.position_at(frame) != flip.position
            && !mode.allows(o, frame, Property::Position)
        {
            return false;
        }
    }
    for flip in &plan {
        let o = &mut doc.objects[flip.index];
        if o.skin.is_none() && o.pivot.is_none() {
            apply(o, frame, flip, mode);
        }
    }
    for (i, next) in staged {
        doc.objects[i] = next;
    }
    true
}
