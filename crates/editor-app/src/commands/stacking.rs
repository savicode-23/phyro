use super::editable_selection;
use document_core::{Document, VectorObject};
use eframe::egui::{pos2, Rect};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub enum Arrange {
    Forward,
    Backward,
    ToFront,
    ToBack,
}

pub fn arrange(doc: &mut Document, ids: &[u64], frame: u32, order: Arrange) {
    let selected: HashSet<_> = editable_selection(doc, ids).into_iter().collect();
    if selected.is_empty() {
        return;
    }
    // Layer order is independent of storage order. Only permute the slots of
    // affected layers so other layers keep their objects and indices.
    let mut layers: HashMap<_, Vec<usize>> = doc
        .objects
        .iter()
        .filter(|o| selected.contains(&o.id))
        .map(|o| (o.layer, Vec::new()))
        .collect();
    for (slot, object) in doc.objects.iter().enumerate() {
        if let Some(layer) = layers.get_mut(&object.layer) {
            layer.push(slot);
        }
    }
    for slots in layers.values_mut() {
        if matches!(order, Arrange::Backward | Arrange::ToBack) {
            slots.reverse();
        }
        let moving: Vec<_> = slots
            .iter()
            .map(|&slot| selected.contains(&doc.objects[slot].id))
            .collect();
        if moving.iter().all(|&selected| selected) {
            continue;
        }
        let targets = if matches!(order, Arrange::Forward | Arrange::Backward) {
            step_targets(&doc.objects, slots, &moving, frame)
        } else {
            vec![slots.len(); slots.len()]
        };
        let mut desired: Vec<_> = (0..slots.len()).collect();
        desired.sort_by_key(|&i| {
            if moving[i] {
                (targets[i], true)
            } else {
                (i, false)
            }
        });
        // Apply each cycle once. Large paths and animated objects stay owned
        // by the document throughout the reorder.
        for first in 0..desired.len() {
            let mut at = first;
            while desired[at] != first {
                let next = desired[at];
                doc.objects.swap(slots[at], slots[next]);
                desired[at] = at;
                at = next;
            }
            desired[at] = at;
        }
    }
}

fn visual_bounds(object: &VectorObject, frame: u32) -> Rect {
    if !object.visible_at(frame) || object.opacity_at(frame) <= 0. {
        return Rect::NOTHING;
    }
    let [left, top, right, bottom] = object.bounds(frame);
    Rect::from_min_max(pos2(left, top), pos2(right, bottom))
}

fn step_targets(
    objects: &[VectorObject],
    slots: &[usize],
    moving: &[bool],
    frame: u32,
) -> Vec<usize> {
    let mut groups: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, &slot) in slots.iter().enumerate() {
        if let Some(group) = objects[slot].group {
            groups.entry(group).or_default().push(i);
        }
    }
    let bounds = |i: usize| visual_bounds(&objects[slots[i]], frame);
    let selection = moving
        .iter()
        .enumerate()
        .filter(|(_, selected)| **selected)
        .fold(Rect::NOTHING, |rect, (i, _)| rect.union(bounds(i)));
    let edge = |i: usize| {
        objects[slots[i]]
            .group
            .map_or(i, |g| *groups[&g].last().unwrap())
    };
    let mut targets: Vec<_> = (0..slots.len()).collect();
    let mut group_bounds = HashMap::new();
    let mut pending = Vec::new();
    for (i, &slot) in slots.iter().enumerate() {
        if moving[i] {
            if i == edge(i) {
                pending.push(i);
            }
        } else if !pending.is_empty() {
            let candidate = if let Some(group) = objects[slot].group {
                *group_bounds.entry(group).or_insert_with(|| {
                    groups[&group]
                        .iter()
                        .fold(Rect::NOTHING, |r, &j| r.union(bounds(j)))
                })
            } else {
                bounds(i)
            };
            if candidate.intersects(selection) {
                for selected in pending.drain(..) {
                    targets[selected] = edge(i);
                }
            }
        }
    }
    // Flat groups in existing drawings can have other items between their
    // members. Cross the full neighboring group and keep selected items ordered.
    let mut previous = 0;
    for i in 0..slots.len() {
        if moving[i] {
            let end = edge(i);
            let target = if targets[end] == end { i } else { targets[end] };
            previous = previous.max(target);
            targets[i] = previous;
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::add_layer;

    #[test]
    fn stacking_keeps_interleaved_groups_whole_and_each_layer_in_its_slots() {
        let mut doc = Document::default();
        let top = add_layer(&mut doc);
        let locked = add_layer(&mut doc);
        doc.layers[2].locked = true;
        let mut rect = |layer, x, group| {
            let id = doc.add_rectangle([x, 0.], [10.; 2], [255; 4]);
            let o = doc.objects.last_mut().unwrap();
            o.layer = layer;
            o.group = group;
            id
        };
        let a = rect(1, 40., Some(3));
        let x = rect(top, 0., None);
        let parked = rect(1, 300., None);
        let b = rect(1, 80., Some(3));
        let locked_a = rect(locked, 0., Some(3));
        let left = rect(1, 0., Some(4));
        let y = rect(top, 1., None);
        let right = rect(1, 130., Some(4));
        let far = rect(1, 500., None);
        let locked_b = rect(locked, 0., None);
        let ids = |doc: &Document| doc.objects.iter().map(|o| o.id).collect::<Vec<_>>();
        let before = doc.clone();
        arrange(&mut doc, &[b, x, locked_b], 0, Arrange::Forward);
        // Neither group member overlaps the other group's members individually;
        // their combined boxes do. The full group crosses in a single step.
        assert_eq!(
            ids(&doc),
            [parked, y, left, right, locked_a, a, x, b, far, locked_b]
        );
        let first = doc.clone();
        arrange(&mut doc, &[b, x, locked_b], 0, Arrange::Forward);
        assert_eq!(doc, first);
        arrange(&mut doc, &[b, x, locked_b], 0, Arrange::Backward);
        assert_eq!(
            ids(&doc),
            [parked, x, a, b, locked_a, left, y, right, far, locked_b]
        );
        for order in [Arrange::ToFront, Arrange::ToBack] {
            arrange(&mut doc, &[b, x, locked_b], 0, order);
            for (original, current) in before.objects.iter().zip(&doc.objects) {
                assert_eq!(current.layer, original.layer);
                assert_eq!(
                    current,
                    before.objects.iter().find(|o| o.id == current.id).unwrap()
                );
            }
        }
        doc.validate().unwrap();
    }
}
