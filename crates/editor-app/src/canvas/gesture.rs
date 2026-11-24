use super::*;
use crate::path_editor::NodeSet;
use document_core::{Property, VectorObject};
use paths::Part;

mod group_resize;

/// Node edits can extend a path outside its original transform frame.
pub(super) fn handles(object: &VectorObject, frame: u32) -> [Pos2; 4] {
    let mut bounds = Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.));
    for n in object.skinned_shape_at(frame).contours().iter().flatten() {
        for p in [n.point, n.incoming, n.outgoing] {
            bounds.extend_with(p.into());
        }
    }
    [
        bounds.left_top(),
        bounds.right_top(),
        bounds.right_bottom(),
        bounds.left_bottom(),
    ]
}

pub(super) fn rotation_handle(object: &VectorObject, frame: u32, top: Pos2, scale: f32) -> Pos2 {
    let (sin, cos) = object.rotation_at(frame).to_radians().sin_cos();
    top + egui::vec2(sin, -cos) * (18.0 * object.scale_at(frame)[1].signum() / scale)
}

pub(super) fn group_bounds(doc: &Document, frame: u32, ids: &GestureIds) -> Option<Rect> {
    let mut bounds = Rect::NOTHING;
    for object in doc
        .objects
        .iter()
        .filter(|o| ids.contains(o.id) && is_editable(doc, o.layer))
    {
        let to_world = object.local_to_world_at(frame);
        for local in handles(object, frame) {
            bounds.extend_with(to_world(local.into()).into());
        }
    }
    (bounds.is_finite() && bounds.is_positive()).then_some(bounds)
}

pub(super) fn box_corners(bounds: Rect) -> [Pos2; 4] {
    [
        bounds.left_top(),
        bounds.right_top(),
        bounds.right_bottom(),
        bounds.left_bottom(),
    ]
}

fn grip(corners: &[Pos2; 4], i: usize) -> Pos2 {
    let corner = i / 2;
    if i.is_multiple_of(2) {
        corners[corner]
    } else {
        corners[corner].lerp(corners[(corner + 1) % 4], 0.5)
    }
}

pub(super) fn resize_handles(
    corners: [Pos2; 4],
    scale: f32,
) -> impl Iterator<Item = (usize, Pos2, Pos2)> {
    // Keep the side grips clear of the corners and leave room to grab tiny selections.
    let show_sides = corners[0]
        .distance(corners[1])
        .min(corners[0].distance(corners[3]))
        * scale
        >= 20.;
    (0..8)
        .filter(move |i| show_sides || i % 2 == 0)
        .map(move |i| (i, grip(&corners, i), grip(&corners, (i + 4) % 8)))
}

pub(super) fn group_resize_handles(bounds: Rect, scale: f32) -> impl Iterator<Item = (Pos2, Pos2)> {
    resize_handles(box_corners(bounds), scale).map(|(_, handle, fixed)| (handle, fixed))
}

pub(super) const HANDLE_HIT: f32 = 9.;

pub(super) fn over_resize_grip(point: Pos2, grip: Pos2, scale: f32, shift: bool) -> bool {
    // Leave a smaller target when Shift-click can also extend the selection.
    if shift {
        (grip - point).abs().max_elem() * scale <= 3.5
    } else {
        grip.distance(point) * scale <= HANDLE_HIT
    }
}

pub(super) fn object_handle(
    object: &VectorObject,
    frame: u32,
    point: Pos2,
    scale: f32,
    shift: bool,
) -> Option<Gesture> {
    let to_world = object.local_to_world_at(frame);
    let corners = handles(object, frame).map(|p| Pos2::from(to_world(p.into())));
    let top = corners[0] + 0.5 * (corners[1] - corners[0]);
    let pivot = Pos2::from(object.pivot_at(frame));
    let offset = pivot - point;
    let grabbing_pivot = !shift && pivot::over_marker(offset * scale);
    let grab = || Gesture::Pivot {
        start: point,
        id: object.id,
        grab_offset: Some(offset),
    };
    let rotate = rotation_handle(object, frame, top, scale);
    // A grip directly on a custom pivot has no lever arm. The crosshair painted
    // over it should be grabbable from the center as well as from its exposed arms.
    let under_pivot = |grip: Pos2| pivot.distance(grip) * scale <= 0.5;
    if grabbing_pivot
        && object.pivot.is_some()
        && (under_pivot(rotate)
            || resize_handles(corners, scale).any(|(_, grip, _)| under_pivot(grip)))
    {
        return Some(grab());
    }
    if rotate.distance(point) * scale <= HANDLE_HIT {
        return Some(Gesture::Rotate {
            start: point,
            id: object.id,
        });
    }
    if let Some((handle, grip, _)) = resize_handles(corners, scale)
        .filter(|(_, grip, _)| over_resize_grip(point, *grip, scale, shift))
        .min_by(|(_, a, _), (_, b, _)| a.distance_sq(point).total_cmp(&b.distance_sq(point)))
    {
        return Some(Gesture::Resize {
            start: point,
            id: object.id,
            handle,
            offset: grip - point,
        });
    }
    grabbing_pivot.then(grab)
}

pub(super) fn turn(start: Pos2, point: Pos2, pivot: Pos2, shift: bool) -> f32 {
    if (point - pivot).length_sq() < f32::EPSILON {
        return 0.;
    }
    let mut degrees = ((point - pivot).angle() - (start - pivot).angle()).to_degrees();
    if shift {
        degrees = (degrees / 15.).round() * 15.;
    }
    degrees
}

pub(super) fn rotate_around(point: Pos2, pivot: Pos2, angle: f32) -> Pos2 {
    let (sin, cos) = angle.to_radians().sin_cos();
    let d = point - pivot;
    pivot + egui::vec2(cos * d.x - sin * d.y, sin * d.x + cos * d.y)
}

fn resize_about_center(object: &mut VectorObject, frame: u32, size: Vec2, center: Pos2) {
    if object.scale_keys.is_empty() {
        object.size = (size / Vec2::from(object.scale).abs()).into();
    } else {
        let signs = Vec2::from(object.scale_at(frame).map(f32::signum));
        object.set_scale_at(frame, (signs * size / Vec2::from(object.size)).into());
    }
    object.set_position_at(frame, (center - Vec2::from(object.size) * 0.5).into());
}

fn set_angle(object: &mut VectorObject, frame: u32, angle: f32) {
    let value = if object.rotation_keys.is_empty() {
        angle.rem_euclid(360.)
    } else {
        angle
    };
    object.set_rotation_at(frame, value);
}

// Only the gesture's private target IDs are sorted. The authored selection and
// document order are preserved for painting, grouping, and the clipboard.
pub(super) struct GestureIds(Vec<u64>);

impl GestureIds {
    pub(super) fn new(mut ids: Vec<u64>) -> Self {
        ids.sort_unstable();
        Self(ids)
    }

    pub(super) fn contains(&self, id: u64) -> bool {
        if self.0.len() <= 8 {
            self.0.contains(&id)
        } else {
            includes(&self.0, id)
        }
    }
}

fn includes(sorted_ids: &[u64], id: u64) -> bool {
    sorted_ids.binary_search(&id).is_ok()
}

pub(super) enum Gesture {
    Gradient {
        start: Pos2,
        id: u64,
        handle: gradient::Handle,
        channel: PaintChannel,
    },
    Pivot {
        start: Pos2,
        id: u64,
        // Direct grabs preserve the distance from the pointer to the crosshair.
        // The toolbar placement mode instead sets the pivot at the pointer on click.
        grab_offset: Option<Vec2>,
    },
    Guide {
        start: Pos2,
        id: u64,
        index: usize,
        part: Part,
    },
    Draw {
        start: Pos2,
        tool: Tool,
    },
    Move {
        start: Pos2,
        ids: GestureIds,
        reference: Pos2,
        flips: [bool; 2],
    },
    Marquee {
        start: Pos2,
        previous: Vec<u64>,
    },
    Resize {
        start: Pos2,
        id: u64,
        handle: usize,
        offset: Vec2,
    },
    Rotate {
        start: Pos2,
        id: u64,
    },
    GroupResize {
        start: Pos2,
        fixed: Pos2,
        handle: Pos2,
        ids: GestureIds,
    },
    GroupRotate {
        start: Pos2,
        bounds: Rect,
        ids: GestureIds,
    },
    NodeMarquee {
        start: Pos2,
        id: u64,
        previous: Option<NodeSet>,
        preview: Option<(Rect, NodeSet)>,
    },
    Node {
        start: Pos2,
        id: u64,
        edit: paths::NodeDrag,
        cycle_on_click: Option<paths::NodeIndex>,
    },
    Pen(paths::PenDrag),
}

impl Gesture {
    pub(super) fn placement_transform(&self) -> Option<u64> {
        match self {
            Self::Move { .. } | Self::Resize { .. } | Self::Rotate { .. } => {
                self.single_transform()
            }
            _ => None,
        }
    }

    pub(super) fn single_transform(&self) -> Option<u64> {
        match self {
            Self::Move {
                ids,
                flips: [false, false],
                ..
            } if ids.0.len() == 1 => Some(ids.0[0]),
            Self::Resize { id, .. } | Self::Rotate { id, .. } | Self::Gradient { id, .. } => {
                Some(*id)
            }
            _ => None,
        }
    }

    pub(super) fn transforms_group(&self) -> bool {
        matches!(self, Self::GroupRotate { .. } | Self::GroupResize { .. })
            || matches!(self, Self::Move { ids, .. } if ids.0.len() > 1)
    }

    pub(super) fn reuse_preview(
        &self,
        original: &Document,
        previous: Option<std::sync::Arc<Document>>,
    ) -> Option<Document> {
        let mut draft = std::sync::Arc::into_inner(previous?)?;
        let id = self.single_transform()?;
        let index = original.objects.iter().position(|o| o.id == id)?;
        let target = draft.objects.get_mut(index)?;
        if target.id != id {
            return None;
        }
        target.clone_from(&original.objects[index]);
        // Inspector edits, layer changes and replacements can arrive between
        // pointer events. Only reuse a complete match to the authored drawing.
        (&draft == original).then_some(draft)
    }

    pub(super) fn snap_subject(&self, point: Pos2) -> Option<(Pos2, snapping::Subject<'_>)> {
        use snapping::Subject;
        Some(match self {
            Self::Move {
                start,
                reference,
                ids,
                ..
            } => {
                let delta = point - *start;
                (*reference + delta, Subject::Moving(ids, delta))
            }
            Self::GroupResize {
                start,
                handle,
                fixed,
                ids,
            } => {
                let mut target = *handle + (point - *start);
                if handle.x == fixed.x {
                    target.x = handle.x;
                }
                if handle.y == fixed.y {
                    target.y = handle.y;
                }
                (target, Subject::Objects(ids))
            }
            Self::Node {
                start, id, edit, ..
            } => edit.snap_subject(*id, *start, point),
            Self::Resize { id, offset, .. } => (point + *offset, Subject::Object(*id)),
            Self::Draw { .. } => (point, Subject::Point),
            _ => return None,
        })
    }

    pub(super) fn has_dragged(&self, point: Pos2, scale: f32) -> bool {
        match self {
            Self::Node { edit, .. } => edit.has_dragged(),
            _ => point.distance(self.start()) * scale >= 2.,
        }
    }

    fn start(&self) -> Pos2 {
        match *self {
            Self::Pivot { start, .. }
            | Self::Gradient { start, .. }
            | Self::Guide { start, .. }
            | Self::Draw { start, .. }
            | Self::Move { start, .. }
            | Self::Marquee { start, .. }
            | Self::NodeMarquee { start, .. }
            | Self::Resize { start, .. }
            | Self::Rotate { start, .. }
            | Self::GroupResize { start, .. }
            | Self::GroupRotate { start, .. }
            | Self::Node { start, .. } => start,
            Self::Pen(drag) => drag.anchor,
        }
    }

    /// Canvas preview and commit apply the same document-space edit.
    pub(super) fn apply(
        &self,
        doc: &mut Document,
        frame: u32,
        point: Pos2,
        style: &CanvasStyle,
        mods: egui::Modifiers,
    ) -> (bool, Option<u64>) {
        let target = point;
        let (ids, channels): (&[u64], &[Property]) = match self {
            Self::Move { ids, .. } => (&ids.0, &[Property::Position]),
            Self::Rotate { id, .. } => (std::slice::from_ref(id), &[Property::Rotation]),
            Self::Resize { id, .. } => (std::slice::from_ref(id), &[Property::Scale]),
            Self::GroupRotate { ids, .. } => (&ids.0, &[Property::Rotation, Property::Position]),
            Self::Node { id, .. } => (std::slice::from_ref(id), &[Property::Geometry]),
            _ => (&[], &[]),
        };
        let selected = |id| includes(ids, id);
        if !ids.is_empty()
            && doc.objects.iter().filter(|o| selected(o.id)).any(|o| {
                channels
                    .iter()
                    .any(|&p| !style.key_mode.allows(o, frame, p))
            })
        {
            return (false, None);
        }
        let prepare = |o: &mut VectorObject| {
            for &p in channels {
                style.key_mode.prepare(o, frame, p);
            }
        };

        match self {
            Self::Gradient {
                id,
                start,
                handle,
                channel,
            } => {
                let changed = doc
                    .objects
                    .iter_mut()
                    .find(|o| o.id == *id)
                    .is_some_and(|o| {
                        gradient::apply(o, frame, *start, point, *handle, *channel, style)
                    });
                (changed, None)
            }
            Self::Guide {
                id, index, part, ..
            } => {
                let changed = doc
                    .objects
                    .iter_mut()
                    .find(|o| o.id == *id)
                    .is_some_and(|o| motion::move_handle(o, *index, *part, target, frame));
                (changed, None)
            }
            Self::Draw { start, tool } => {
                let mut delta = target - *start;
                if mods.shift {
                    let extent = (delta.x.abs() + delta.y.abs()) * 0.5;
                    delta = egui::vec2(extent.copysign(delta.x), extent.copysign(delta.y));
                }
                let opposite = if mods.ctrl { *start - delta } else { *start };
                let bounds = Rect::from_two_pos(opposite, *start + delta);
                if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                    return (false, None);
                }
                let shape = if *tool == Tool::Ellipse {
                    Shape::Ellipse
                } else {
                    Shape::Rectangle
                };
                let id = doc.add_shape(
                    bounds.min.into(),
                    bounds.size().into(),
                    style.fill,
                    shape,
                    style.layer,
                );
                style.paint_new_object(doc);
                (true, Some(id))
            }
            Self::Move {
                reference, flips, ..
            } => {
                let delta = target - *reference;
                let mut changed = false;
                if delta != Vec2::ZERO {
                    for object in doc.objects.iter_mut().filter(|o| selected(o.id)) {
                        let pos = Pos2::from(object.position_at(frame)) + delta;
                        prepare(object);
                        object.set_position_at(frame, pos.into());
                        changed = true;
                    }
                }
                for (axis, flip) in flips.iter().enumerate() {
                    if *flip {
                        changed |= super::flip_selection_in(
                            doc,
                            ids,
                            frame,
                            axis == 0,
                            style.key_mode,
                            SelectionScope::Objects,
                        );
                    }
                }
                (changed, None)
            }
            Self::Resize { id, handle, .. } => {
                let Some(object) = doc.objects.iter_mut().find(|o| o.id == *id) else {
                    return (false, None);
                };
                let corners = handles(object, frame);
                let moving = grip(&corners, *handle);
                let opposite = grip(&corners, (handle + 4) % 8);
                let anchor = object.pivot.map(Pos2::from).unwrap_or_else(|| {
                    if mods.ctrl {
                        opposite.lerp(moving, 0.5)
                    } else {
                        opposite
                    }
                });
                let to_world = object.local_to_world_at(frame);
                let fixed = Pos2::from(to_world(anchor.into()));
                let (sin, cos) = object.rotation_at(frame).to_radians().sin_cos();
                let shear = object.skew_at(frame).to_radians().tan();
                let d = target - fixed;
                let span = moving - anchor;
                let old_size = Vec2::from(object.scaled_box_at(frame).1);
                let signs = Vec2::from(object.scale_at(frame).map(f32::signum));
                let size = if mods.shift {
                    let axis = Pos2::from(to_world(moving.into())) - fixed;
                    if axis.length_sq() <= f32::EPSILON {
                        return (false, None);
                    }
                    let factor = d.dot(axis) / axis.length_sq();
                    old_size * factor.max(0.1 / old_size.min_elem())
                } else {
                    // Measure in the object's axes, including shear from a group stretch.
                    let dy = -d.x * sin + d.y * cos;
                    let dx = d.x * cos + d.y * sin - shear * dy;
                    egui::vec2(
                        if moving.x == opposite.x || span.x.abs() <= f32::EPSILON {
                            old_size.x
                        } else {
                            (signs.x * dx / span.x).max(0.1)
                        },
                        if moving.y == opposite.y || span.y.abs() <= f32::EPSILON {
                            old_size.y
                        } else {
                            (signs.y * dy / span.y).max(0.1)
                        },
                    )
                };
                if !size.is_finite() || size == old_size {
                    return (false, None);
                }
                if object.pivot.is_some() {
                    let scale = signs * size / Vec2::from(object.size);
                    if !scale.is_finite() {
                        return (false, None);
                    }
                    prepare(object);
                    object.set_scale_at(frame, scale.into());
                    return (true, None);
                }
                if !style.key_mode.prepare(object, frame, Property::Position) {
                    return (false, None);
                }
                let half = signs * size * (egui::pos2(0.5, 0.5) - anchor);
                let x = half.x + shear * half.y;
                let center = fixed + egui::vec2(x * cos - half.y * sin, x * sin + half.y * cos);
                prepare(object);
                resize_about_center(object, frame, size, center);
                (true, None)
            }
            Self::Rotate { start, id } => {
                let Some(object) = doc.objects.iter_mut().find(|o| o.id == *id) else {
                    return (false, None);
                };
                let pivot = Pos2::from(object.pivot_at(frame));
                let offset = point - pivot;
                let start_offset = *start - pivot;
                if offset.length_sq() < f32::EPSILON || start_offset.length_sq() < f32::EPSILON {
                    return (false, None);
                }
                let mut angle = object.rotation_at(frame)
                    + (offset.angle() - start_offset.angle()).to_degrees();
                if mods.shift {
                    angle = (angle / 15.).round() * 15.;
                }
                if angle == object.rotation_at(frame) {
                    return (false, None);
                }
                prepare(object);
                set_angle(object, frame, angle);
                (true, None)
            }
            Self::GroupRotate { start, bounds, .. } => {
                let pivot = bounds.center();
                let angle = turn(*start, point, pivot, mods.shift);
                if angle.rem_euclid(360.) == 0.0 {
                    return (false, None);
                }
                for object in doc.objects.iter_mut().filter(|o| selected(o.id)) {
                    let half = Vec2::from(object.size) * 0.5;
                    let center = Pos2::from(object.position_at(frame)) + half;
                    let position = rotate_around(center, pivot, angle) - half;
                    prepare(object);
                    set_angle(object, frame, object.rotation_at(frame) + angle);
                    object.set_position_at(frame, position.into());
                }
                (true, None)
            }
            Self::Node { id, edit, .. } => {
                let changed = doc
                    .objects
                    .iter_mut()
                    .find(|o| o.id == *id)
                    .is_some_and(|o| {
                        let Some(shape) = edit.shape_at(o, frame, target) else {
                            return false;
                        };
                        prepare(o);
                        o.set_shape_at(frame, shape).is_ok()
                    });
                (changed, None)
            }
            Self::GroupResize {
                fixed, handle, ids, ..
            } => (
                group_resize::apply(
                    doc,
                    ids,
                    frame,
                    if mods.ctrl {
                        fixed.lerp(*handle, 0.5)
                    } else {
                        *fixed
                    },
                    *handle,
                    target,
                    mods.shift,
                    style.key_mode,
                ),
                None,
            ),
            Self::Pivot { .. } | Self::NodeMarquee { .. } | Self::Marquee { .. } | Self::Pen(_) => {
                (false, None)
            }
        }
    }
}
