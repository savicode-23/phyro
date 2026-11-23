use super::{Row, Tracks};
use document_core::{Clip, Easing, Interpolation, Property};
use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive};

struct KeyWindow {
    exposure: Range<u32>,
    first: u32,
    last: u32,
}

impl KeyWindow {
    fn pairs<T>(&self, keys: &BTreeMap<u32, T>, mut visit: impl FnMut(u32, &T, u32, &T)) {
        // One neighboring key on either side includes long tweens with both ends offscreen.
        let mut previous = keys.range(self.exposure.start..self.first).next_back();
        let after = keys.range(self.last + 1..self.exposure.end).next();
        for (end, to) in keys.range(self.first..=self.last).chain(after) {
            if let Some((&start, from)) = previous {
                visit(start, from, *end, to);
            }
            previous = Some((end, to));
        }
    }

    fn changes<T: PartialEq>(&self, keys: &BTreeMap<u32, T>, mut visit: impl FnMut(u32, u32)) {
        self.pairs(keys, |start, from, end, to| {
            if from != to {
                visit(start, end);
            }
        });
    }
}

/// Report authored tweens intersecting the viewport, constrained to the row's drawing exposure.
pub(super) fn visible(
    row: Row,
    clip: &Clip,
    tracks: &Tracks,
    visible: RangeInclusive<u32>,
    mut emit: impl FnMut(u32, u32),
) {
    let exposure = tracks.row_exposure(row);
    let first = exposure.start.max(*visible.start());
    let last = exposure.end.saturating_sub(1).min(*visible.end());
    if first > last || exposure.is_empty() {
        return;
    }
    let window = KeyWindow {
        exposure,
        first,
        last,
    };
    if row == Row::Bone {
        if let Some(bone) = tracks
            .bone
            .and_then(|track| track.bone(clip))
            .filter(|bone| bone.interpolation == Interpolation::Linear)
        {
            window.changes(&bone.rotation_keys, emit);
        }
        return;
    }
    let (Row::Property(property), Some(object)) = (row, tracks.object(clip)) else {
        return;
    };
    if property.is_muted(object) {
        return;
    }
    let mut tween = |start, end| {
        if property.easing_at(object, start) != Easing::Hold {
            emit(start, end);
        }
    };
    let animated = &object.animated;
    match property {
        Property::Position => window.changes(&object.keys, tween),
        Property::Rotation => window.changes(&object.rotation_keys, tween),
        Property::Scale => window.pairs(&object.scale_keys, |a, x, b, y| {
            // Only scale magnitudes tween. A flip takes effect on its authored frame.
            if x.map(f32::abs) != y.map(f32::abs) {
                tween(a, b);
            }
        }),
        Property::Opacity => window.changes(&object.opacity_keys, tween),
        Property::Skew => window.changes(&animated.skew_keys, tween),
        Property::Fill => window.changes(&animated.fill_keys, tween),
        Property::Stroke => window.changes(&animated.stroke_keys, tween),
        Property::StrokeWidth => window.changes(&animated.width_keys, tween),
        Property::MotionProgress => window.changes(&animated.progress_keys, tween),
        Property::Geometry => window.pairs(&animated.geometry_keys, |a, x, b, y| {
            if property.easing_at(object, a) == Easing::Hold {
                return;
            }
            // A guided correspondence can move the outline even between identical poses.
            let guides_move = object
                .morph_guides(a, b)
                .is_some_and(|guides| guides.iter().any(|g| g.from != g.to));
            let old = x.contours();
            let new = y.contours();
            if guides_move
                || old.len() != new.len()
                || old.iter().zip(new).any(|(p, q)| {
                    p.len() != q.len()
                        || p.iter().zip(q).any(|(p, q)| {
                            p.point != q.point
                                || p.incoming != q.incoming
                                || p.outgoing != q.outgoing
                        })
                })
            {
                emit(a, b);
            }
        }),
        Property::Visibility => {}
    }
}
