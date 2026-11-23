//! Alignment of evaluated artwork, including grouped selections and posed objects.
#[cfg(test)]
mod flip_tests;
use crate::{Document, Property, VectorObject};
use std::collections::{HashMap, HashSet};

pub fn is_editable(doc: &Document, layer: u64) -> bool {
    doc.layers
        .iter()
        .any(|l| l.id == layer && l.visible && !l.locked)
}

pub fn is_object_editable(doc: &Document, object: &VectorObject) -> bool {
    !object.locked && is_editable(doc, object.layer)
}

/// Change the lock on each selected object and all members of its layer-local group.
pub fn set_object_locks(doc: &mut Document, ids: &[u64], locked: bool) {
    let ids: HashSet<_> = ids.iter().copied().collect();
    let units: HashSet<_> = doc
        .objects
        .iter()
        .filter(|o| ids.contains(&o.id) && is_editable(doc, o.layer))
        .map(unit)
        .collect();
    for object in &mut doc.objects {
        if units.contains(&unit(object)) {
            object.locked = locked;
        }
    }
}

pub fn editable_selection(doc: &Document, ids: &[u64]) -> Vec<u64> {
    if let [id] = ids {
        let Some(o) = doc
            .objects
            .iter()
            .find(|o| o.id == *id && is_object_editable(doc, o))
        else {
            return Vec::new();
        };
        let Some(group) = o.group else {
            return vec![*id];
        };
        return doc
            .objects
            .iter()
            .filter(|member| {
                member.layer == o.layer && member.group == Some(group) && !member.locked
            })
            .map(|member| member.id)
            .collect();
    }
    let ids: HashSet<_> = ids.iter().copied().collect();
    let layers: HashSet<_> = doc
        .layers
        .iter()
        .filter(|l| l.visible && !l.locked)
        .map(|l| l.id)
        .collect();
    let groups: HashSet<_> = doc
        .objects
        .iter()
        .filter(|o| !o.locked && ids.contains(&o.id))
        .filter_map(|o| o.group.map(|g| (o.layer, g)))
        .collect();
    doc.objects
        .iter()
        .filter(|o| !o.locked && layers.contains(&o.layer))
        .filter(|o| ids.contains(&o.id) || o.group.is_some_and(|g| groups.contains(&(o.layer, g))))
        .map(|o| o.id)
        .collect()
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

impl Alignment {
    pub fn coordinate(self, bounds: [f32; 4]) -> f32 {
        match self {
            Self::Left => bounds[0],
            Self::CenterX => bounds[0] * 0.5 + bounds[2] * 0.5,
            Self::Right => bounds[2],
            Self::Top => bounds[1],
            Self::CenterY => bounds[1] * 0.5 + bounds[3] * 0.5,
            Self::Bottom => bounds[3],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Unit {
    Group(u64, u64),
    Object(u64),
}

fn unit(object: &VectorObject) -> Unit {
    object
        .group
        .map_or(Unit::Object(object.id), |g| Unit::Group(object.layer, g))
}

#[derive(Clone, Copy)]
pub enum Action {
    Align(Alignment, Reference),
    DistributeX,
    DistributeY,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reference {
    #[default]
    #[serde(rename = "biggest")]
    BiggestByDirection,
    LargestArea,
    Bounds,
}

// UI enablement only needs to distinguish zero, one, two, and three or more
// independent units. Marquee and Select All normally follow document order.
pub fn available_units(doc: &Document, ids: &[u64]) -> usize {
    let mut seen = [None; 3];
    let mut count = 0;
    let mut remaining = doc.objects.iter();
    let mut by_id: Option<HashMap<_, _>> = None;
    for id in ids {
        let object = if let Some(index) = &by_id {
            index.get(id).copied()
        } else if let Some(object) = remaining.find(|o| o.id == *id) {
            Some(object)
        } else if ids.len() <= 3 {
            doc.objects.iter().find(|o| o.id == *id)
        } else {
            let index = by_id.insert(doc.objects.iter().map(|o| (o.id, o)).collect());
            index.get(id).copied()
        };
        if let Some(object) = object {
            let unit = unit(object);
            if !seen[..count].contains(&Some(unit)) && is_object_editable(doc, object) {
                seen[count] = Some(unit);
                count += 1;
                if count == 3 {
                    break;
                }
            }
        }
    }
    count
}

struct Selection {
    bounds: Vec<[f32; 4]>,
    // Document object index and its unit's index into bounds.
    members: Vec<(usize, usize)>,
}

// Visual bounds include stroked Bézier extrema. For large selections, evaluate
// immutable geometry in batches while retaining document order for group unions and ties.
fn parallel_member_bounds(
    doc: &Document,
    frame: u32,
    members: &[(usize, usize)],
    bounds_at: fn(&VectorObject, u32) -> [f32; 4],
) -> Option<Vec<[f32; 4]>> {
    const MIN_BATCH: usize = 1_024;
    if members.len() < 2 * MIN_BATCH {
        return None;
    }
    let workers = std::thread::available_parallelism()
        .ok()?
        .get()
        .min(4)
        .min(members.len() / MIN_BATCH);
    if workers < 2 {
        return None;
    }
    let mut rects = vec![[0.; 4]; members.len()];
    let size = members.len().div_ceil(workers);
    let measure = |chunk: &[(usize, usize)], out: &mut [[f32; 4]]| {
        for (&(index, _), rect) in chunk.iter().zip(out) {
            *rect = bounds_at(&doc.objects[index], frame);
        }
    };
    std::thread::scope(|scope| {
        let mut chunks = members.chunks(size).zip(rects.chunks_mut(size));
        let (here, out) = chunks.next()?;
        for (chunk, out) in chunks {
            std::thread::Builder::new()
                .spawn_scoped(scope, move || measure(chunk, out))
                .ok()?;
        }
        measure(here, out);
        Some(())
    })?;
    Some(rects)
}

impl Selection {
    fn new(doc: &Document, ids: &[u64], frame: u32) -> Result<Self, String> {
        Self::measure(doc, ids, frame, VectorObject::bounds)
    }
    fn measure(
        doc: &Document,
        ids: &[u64],
        frame: u32,
        bounds_at: fn(&VectorObject, u32) -> [f32; 4],
    ) -> Result<Self, String> {
        let selected: HashSet<_> = editable_selection(doc, ids).into_iter().collect();
        let mut units = HashMap::<Unit, usize>::new();
        let mut members = Vec::with_capacity(selected.len());
        for (index, object) in doc.objects.iter().enumerate() {
            if selected.contains(&object.id) {
                let next_unit = units.len();
                let unit = *units.entry(unit(object)).or_insert(next_unit);
                members.push((index, unit));
            }
        }
        let calculated = parallel_member_bounds(doc, frame, &members, bounds_at);
        let mut bounds = vec![
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY
            ];
            units.len()
        ];
        for (i, &(index, unit)) in members.iter().enumerate() {
            let rect = calculated
                .as_ref()
                .map_or_else(|| bounds_at(&doc.objects[index], frame), |b| b[i]);
            if !rect.into_iter().all(f32::is_finite) {
                return Err("A selected object has no finite visual bounds at this frame.".into());
            }
            bounds[unit] = union(bounds[unit], rect);
        }
        Ok(Self { bounds, members })
    }

    fn offsets(&self, doc: &Document, action: Action) -> Result<(usize, Vec<f64>), String> {
        let bounds = &self.bounds;
        let mut offsets = vec![0.; bounds.len()];
        match action {
            Action::Align(direction, reference) => {
                let axis = match direction {
                    Alignment::Left | Alignment::CenterX | Alignment::Right => 0,
                    Alignment::Top | Alignment::CenterY | Alignment::Bottom => 1,
                };
                let target_bounds = if reference == Reference::Bounds {
                    match bounds.as_slice() {
                        [] => return Err("Select an editable object or group to align.".into()),
                        [_] => [0., 0., doc.width as f32, doc.height as f32],
                        [first, rest @ ..] => rest.iter().copied().fold(*first, union),
                    }
                } else {
                    if bounds.len() < 2 {
                        return Err("Select at least two objects or groups to align.".into());
                    }
                    let size = |b: [f32; 4]| {
                        let width = f64::from(b[2]) - f64::from(b[0]);
                        let height = f64::from(b[3]) - f64::from(b[1]);
                        // Inkscape measures along the alignment line: height for
                        // vertical lines (Left / Center X / Right), width for horizontal.
                        if reference == Reference::LargestArea {
                            width * height
                        } else if axis == 0 {
                            height
                        } else {
                            width
                        }
                    };
                    // Keep the first unit in document order when the sizes tie.
                    let mut biggest = bounds[0];
                    let mut biggest_size = size(biggest);
                    for &rect in &bounds[1..] {
                        let candidate = size(rect);
                        if candidate > biggest_size {
                            biggest = rect;
                            biggest_size = candidate;
                        }
                    }
                    biggest
                };
                let target = f64::from(direction.coordinate(target_bounds));
                for (offset, &rect) in offsets.iter_mut().zip(bounds) {
                    *offset = target - f64::from(direction.coordinate(rect));
                }
                Ok((axis, offsets))
            }
            Action::DistributeX | Action::DistributeY => {
                if bounds.len() < 3 {
                    return Err("Select at least three objects or groups to distribute.".into());
                }
                let axis = usize::from(matches!(action, Action::DistributeY));
                let low = |i: usize| f64::from(bounds[i][axis]);
                let high = |i: usize| f64::from(bounds[i][axis + 2]);
                let center = |i: usize| (low(i) + high(i)) * 0.5;
                let mut order: Vec<_> = (0..bounds.len()).collect();
                order.sort_by(|&a, &b| center(a).total_cmp(&center(b)));
                let last = order.len() - 1;
                let interior = &order[1..last];
                let width: f64 = interior.iter().map(|&i| high(i) - low(i)).sum();
                let gap = (low(order[last]) - high(order[0]) - width) / last as f64;
                let mut next = high(order[0]) + gap;
                let mut previous_center = center(order[0]);
                for &i in interior {
                    offsets[i] = next - low(i);
                    let new_center = center(i) + offsets[i];
                    if new_center < previous_center || new_center > center(order[last]) {
                        return Err("The end objects are too close to distribute these sizes without changing their order.".into());
                    }
                    previous_center = new_center;
                    next += high(i) - low(i) + gap;
                }
                Ok((axis, offsets))
            }
        }
    }
}

/// Member transforms for a world-axis reflection of the evaluated selection.
/// Positions are the object's unscaled box origin, as in position_at.
pub struct Flip {
    pub index: usize,
    pub position: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
    pub skew: f32,
}

mod flip_bounds;

pub fn plan_flip(
    doc: &Document,
    ids: &[u64],
    frame: u32,
    horizontal: bool,
) -> Result<Vec<Flip>, String> {
    if frame >= doc.frames {
        return Err("Frame outside this drawing.".into());
    }
    let selected = Selection::measure(doc, ids, frame, flip_bounds::measure)?;
    let bounds = selected
        .bounds
        .into_iter()
        .reduce(union)
        .ok_or("Select editable artwork to flip.")?;
    if bounds[0] > bounds[2] || bounds[1] > bounds[3] {
        return Err("The clipped selection has no visible bounds to flip.".into());
    }
    let axis = usize::from(!horizontal);
    let twice_pivot = f64::from(bounds[axis]) + f64::from(bounds[axis + 2]);
    selected
        .members
        .into_iter()
        .map(|(index, _)| {
            let o = &doc.objects[index];
            let mut position = o.position_at(frame);
            position[axis] =
                (twice_pivot - f64::from(position[axis]) - f64::from(o.size[axis])) as f32;
            if !o.valid_bounds(position) {
                return Err("The flipped object lies outside the coordinate range.".into());
            }
            // Reflecting a rotated, skewed basis negates its rotation and shear.
            // Equivalent orientations differ by a half turn and an exchange of
            // the flipped local axis. Choose the nearest angle to preserve keyed turns.
            let original = o.rotation_at(frame);
            let angle = f64::from(original).rem_euclid(360.);
            let quarter = (angle / 90.).round_ties_even();
            let rotation = (f64::from(original) + 2. * (90. * quarter - angle)) as f32;
            let mut scale = o.scale_at(frame);
            scale[axis ^ (quarter as usize % 2)] *= -1.;
            Ok(Flip {
                index,
                position,
                rotation,
                scale,
                skew: -o.skew_at(frame),
            })
        })
        .collect()
}

/// Plan moves from the evaluated visual bounds. Object indices refer to the unchanged drawing.
pub fn position_updates(
    doc: &Document,
    ids: &[u64],
    frame: u32,
    action: Action,
) -> Result<Vec<(usize, [f32; 2])>, String> {
    if frame >= doc.frames {
        return Err("The playhead is outside this drawing.".into());
    }
    let selection = Selection::new(doc, ids, frame)?;
    let (axis, offsets) = selection.offsets(doc, action)?;
    let mut moves = Vec::new();
    for (index, unit) in selection.members {
        let distance = offsets[unit];
        let bounds = selection.bounds[unit];
        // A fresh evaluation of a translated and stroked curve can shift its
        // bounds by a couple of float steps, especially far out on the pasteboard.
        let extent = bounds[axis].abs().max(bounds[axis + 2].abs()).max(1.);
        if distance.abs() <= f64::from(2. * f32::EPSILON * extent) {
            continue;
        }
        let object = &doc.objects[index];
        if Property::Position.is_muted(object) {
            return Err("A moving object’s Position track is muted.".into());
        }
        let mut requested = [0.; 2];
        requested[axis] = distance as f32;
        if !requested[axis].is_finite() {
            return Err("The arranged position is outside the supported coordinate range.".into());
        }
        let delta = object.skin_position_delta_at(requested, frame).ok_or(
            "A posed skin in the selection cannot move this distance without changing its shape.",
        )?;
        let old = object.position_at(frame);
        let position = std::array::from_fn(|i| old[i] + delta[i]);
        if !position.into_iter().all(f32::is_finite) {
            return Err("The arranged position is outside the supported coordinate range.".into());
        }
        if position == old {
            return Err("A moving object’s position cannot represent this offset at the current coordinate precision.".into());
        }
        moves.push((index, position));
    }
    Ok(moves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Shape;
    #[test]
    fn large_selection_unions_exact_animated_bounds_across_batches_in_document_order() {
        let mut doc = Document::default();
        let mut expected_bounds = Vec::new();
        let mut selected = Vec::new();
        for i in 0..2_051 {
            let x = i as f32 * 9.;
            let y = (i % 13) as f32 * 4.;
            let id = doc.add_shape([x, y], [8., 12.], [190; 4], Shape::Ellipse, 1);
            let group = i % 3;
            let object = doc.objects.last_mut().unwrap();
            object.group = Some(group as u64);
            object.stroke_width = 2.;
            object.stroke = [20, 40, 60, 255];
            object.rotation = (i % 17) as f32;
            object.keys.insert(6, [x + 7., y - 3.]);
            let bounds = object.bounds(6);
            if i < 3 {
                selected.push(id);
                expected_bounds.push(bounds);
            } else {
                expected_bounds[group] = union(expected_bounds[group], bounds);
            }
        }
        let locked = 2;
        doc.layers.push(crate::Layer {
            id: locked,
            ..Default::default()
        });
        let excluded = doc.add_shape([-99_999.; 2], [5.; 2], [255; 4], Shape::Rectangle, locked);
        doc.layers.last_mut().unwrap().locked = true;
        selected.push(excluded);
        selected.reverse();

        let arranged = Selection::new(&doc, &selected, 6).unwrap();
        assert_eq!(arranged.bounds, expected_bounds);
        assert_eq!(
            arranged.members,
            (0..2_051).map(|i| (i, i % 3)).collect::<Vec<_>>()
        );
    }
}
