//! Editor transactions and selection commands. IDs in the selection refer to authored objects.

use document_core::{
    arrange::SelectionScope, AnimationProject, Document, Layer, Shape, Skeleton, VectorObject,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::{self, size_of};
use std::sync::Arc;
mod clipboard_symbols;
mod paste_style;
#[cfg(test)]
pub(crate) use paste_style::has_style_target;
pub(crate) use paste_style::has_style_target_in;

pub(crate) fn has_vector_paint(object: &VectorObject) -> bool {
    !object.is_instance() && object.image.is_none()
}
mod stacking;
pub use stacking::{arrange_in, Arrange};
#[cfg(test)]
mod history_tests;

#[derive(Default)]
pub struct History {
    pub(crate) revision: u64,
    past: VecDeque<Transaction>,
    future: VecDeque<Transaction>,
    location: Option<(usize, u32)>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn resize_artboard(
        &mut self,
        project: &mut AnimationProject,
        clip: usize,
        frame: u32,
        size: [u32; 2],
    ) -> Result<(), String> {
        let doc = project.clips[clip].drawing_at(frame);
        let previous = [doc.width, doc.height];
        if size != previous {
            project.resize_artboard(size)?;
            self.future.clear();
            self.past.push_back(Transaction {
                reviewed: false,
                clip,
                frame,
                edit: ProjectEdit::Artboard(previous),
            });
            self.finish_transaction();
        }
        Ok(())
    }

    /// The clip and frame to display after the most recent undo or redo.
    pub fn location(&self) -> Option<(usize, u32)> {
        self.location
    }

    pub fn record(
        &mut self,
        before: &AnimationProject,
        after: &AnimationProject,
        clip: usize,
        frame: u32,
    ) {
        self.record_project(before, after, clip, frame, false);
    }

    pub(crate) fn record_reviewed(
        &mut self,
        before: &AnimationProject,
        after: &AnimationProject,
        clip: usize,
        frame: u32,
    ) {
        self.record_project(before, after, clip, frame, true);
    }

    fn record_project(
        &mut self,
        before: &AnimationProject,
        after: &AnimationProject,
        clip: usize,
        frame: u32,
        reviewed: bool,
    ) {
        if before == after {
            return;
        }
        self.future.clear();
        self.past.push_back(Transaction {
            reviewed,
            clip: if before.clips.len() == after.clips.len() + 1 {
                // The editor may already have selected the next clip after a deletion.
                before
                    .clips
                    .iter()
                    .position(|old| after.clip_index(old.id).is_none())
                    .unwrap_or(clip)
            } else {
                clip
            },
            frame,
            edit: ProjectEdit::between(before, after, clip, frame),
        });
        self.finish_transaction();
    }

    pub fn undo(&mut self, project: &mut AnimationProject) -> bool {
        let Some(mut change) = self.past.pop_back() else {
            return false;
        };
        let Some(location) = change.swap(project) else {
            self.past.push_back(change);
            return false;
        };
        self.location = Some(location);
        self.future.push_back(change);
        self.finish_transaction();
        true
    }

    pub fn redo(&mut self, project: &mut AnimationProject) -> bool {
        let Some(mut change) = self.future.pop_back() else {
            return false;
        };
        let Some(location) = change.swap(project) else {
            self.future.push_back(change);
            return false;
        };
        self.location = Some(location);
        self.past.push_back(change);
        self.finish_transaction();
        true
    }

    fn finish_transaction(&mut self) {
        self.revision += 1;
        const BUDGET: usize = 64 * 1024 * 1024;
        self.trim_to_budget(BUDGET);
    }

    fn trim_to_budget(&mut self, budget: usize) {
        const MAX_ENTRIES: usize = 128;
        let single = self.past.len() + self.future.len() == 1;
        let mut seen = HashSet::new();
        let mut remaining = budget;
        let mut kept = 0;
        // Retain the same contiguous history obtained by evicting the oldest undo
        // entries first, then the farthest redo entries. Count each shared allocation once.
        for entry in self
            .future
            .iter()
            .rev()
            .chain(self.past.iter().rev())
            .take(MAX_ENTRIES)
        {
            if single && entry.reviewed {
                return;
            }
            let bytes = entry.edit.bytes(&mut seen);
            if bytes > remaining {
                // A reviewed batch keeps its single undo/redo entry even if it
                // uses the entire history budget.
                if kept == 0 && entry.reviewed {
                    kept = 1;
                }
                break;
            }
            remaining -= bytes;
            kept += 1;
        }
        let keep_future = kept.min(self.future.len());
        let keep_past = kept - keep_future;
        self.past.drain(..self.past.len() - keep_past);
        self.future.drain(..self.future.len() - keep_future);
    }
}

struct Transaction {
    reviewed: bool,
    clip: usize,
    frame: u32,
    edit: ProjectEdit,
}

impl Transaction {
    fn swap(&mut self, project: &mut AnimationProject) -> Option<(usize, u32)> {
        match &mut self.edit {
            ProjectEdit::Drawing(change) => {
                let doc = project.clips[self.clip].drawing_at_mut(self.frame);
                change.swap(doc);
            }
            ProjectEdit::Structure(snapshot) => mem::swap(project, snapshot),
            ProjectEdit::Artboard(size) => {
                let doc = project.clips[0].drawing_at(0);
                let current = [doc.width, doc.height];
                project.resize_artboard(*size).ok()?;
                *size = current;
            }
        }
        let clip = self.clip.min(project.clips.len() - 1);
        Some((clip, self.frame.min(project.clips[clip].frames - 1)))
    }
}

enum ProjectEdit {
    // A project-wide resize can affect every cel. Undo only needs the old dimensions.
    Artboard([u32; 2]),
    Drawing(Change),
    // Exposures share their artwork through Arc, including unchanged cels across snapshots.
    Structure(AnimationProject),
}

impl ProjectEdit {
    fn between(
        before: &AnimationProject,
        after: &AnimationProject,
        clip: usize,
        frame: u32,
    ) -> Self {
        if let (Some(old), Some(new)) = (before.clips.get(clip), after.clips.get(clip)) {
            if let (Some((&start, a)), Some((&new_start, b))) = (
                old.cels.range(..=frame).next_back(),
                new.cels.range(..=frame).next_back(),
            ) {
                if start == new_start && !Arc::ptr_eq(a, b) {
                    // Compare after substituting the edited cel. The remaining comparison
                    // covers every project field and usually follows shared Arc pointers.
                    let mut candidate = before.clone();
                    candidate.clips[clip].cels.insert(start, Arc::clone(b));
                    if &candidate == after {
                        return Self::Drawing(Change::between(a, b));
                    }
                }
            }
        }
        Self::Structure(before.clone())
    }

    fn bytes(&self, seen: &mut HashSet<*const ()>) -> usize {
        match self {
            Self::Artboard(_) => size_of::<Self>(),
            Self::Drawing(change) => change.bytes(seen),
            Self::Structure(project) => {
                let mut bytes =
                    size_of::<Self>() + project.clips.capacity() * size_of::<document_core::Clip>();
                bytes += project.animation_sets.capacity()
                    * size_of::<document_core::AnimationSet>()
                    + project
                        .animation_sets
                        .iter()
                        .map(|set| set.name.capacity())
                        .sum::<usize>();
                bytes += project.presets.len() * 128
                    + project.presets.keys().map(String::capacity).sum::<usize>();
                for symbol in project.symbols.values() {
                    bytes +=
                        128 + symbol.name.capacity() + shared_document_bytes(&symbol.artwork, seen);
                }
                bytes += project
                    .clips
                    .iter()
                    .map(|clip| clip_heap_bytes(clip, seen))
                    .sum::<usize>();
                bytes
            }
        }
    }
}

fn clip_heap_bytes(clip: &document_core::Clip, seen: &mut HashSet<*const ()>) -> usize {
    let mut bytes = clip.name.capacity() + clip.cels.len() * 128;
    bytes += clip
        .events
        .values()
        .map(|names| {
            64 + names.capacity() * size_of::<String>()
                + names.iter().map(String::capacity).sum::<usize>()
        })
        .sum::<usize>();
    if !clip.layer_starts.is_empty() {
        // Include sparse owner sets and their lazily built per-layer lookup arrays.
        bytes += clip.cels.len() * size_of::<u32>();
        for owners in clip.layer_starts.values() {
            bytes += 160 + owners.len() * (size_of::<u64>() + 2 * size_of::<u32>());
        }
    }
    bytes
        + clip
            .cels
            .values()
            .map(|doc| shared_document_bytes(doc, seen))
            .sum::<usize>()
}

fn skeleton_heap_bytes(skeleton: &Skeleton) -> usize {
    skeleton.bones.capacity() * size_of::<document_core::Bone>()
        + skeleton
            .bones
            .iter()
            // The derived rig shares one copy of each animated joint across its attachments.
            .map(|bone| bone.name.capacity() + bone.rotation_keys.len() * 2 * 128)
            .sum::<usize>()
        + skeleton
            .foot_pins
            .values()
            .map(|events| {
                // Authored plant/release events and the shared derived tracks for
                // the two joints and optional articulated foot.
                128 + events.len() * 4 * 128
            })
            .sum::<usize>()
}

// Documents and clipping sources can each be shared by many history entries.
fn shared_document_bytes(doc: &Arc<Document>, seen: &mut HashSet<*const ()>) -> usize {
    if !seen.insert(Arc::as_ptr(doc).cast()) {
        return 0;
    }
    size_of::<Document>()
        + skeleton_heap_bytes(&doc.skeleton)
        + doc.objects.capacity() * size_of::<VectorObject>()
        + doc
            .objects
            .iter()
            .map(|o| object_heap_bytes(o, seen))
            .sum::<usize>()
        + doc.layers.capacity() * size_of::<Layer>()
        + doc
            .layers
            .iter()
            .map(|layer| layer.name.capacity())
            .sum::<usize>()
        + (doc.guides_x.capacity() + doc.guides_y.capacity()) * size_of::<f32>()
}

fn object_heap_bytes(object: &VectorObject, seen: &mut HashSet<*const ()>) -> usize {
    let keys = object.keys.len()
        + object.rotation_keys.len()
        + object.scale_keys.len()
        + object.opacity_keys.len()
        + object.animated.skew_keys.len()
        + object.animated.fill_keys.len()
        + object.animated.stroke_keys.len()
        + object.animated.width_keys.len()
        + object.animated.visible_keys.len()
        + object.animated.geometry_keys.len()
        + object.animated.progress_keys.len()
        + object
            .clip_instance
            .as_ref()
            .map_or(0, |i| i.drawing_keys.len())
        + object
            .animated
            .easing
            .values()
            .map(std::collections::BTreeMap::len)
            .sum::<usize>();
    let shape_bytes = |shape: &Shape| {
        let contours = shape.contours();
        let nodes = contours.iter().map(Vec::capacity).sum::<usize>()
            * size_of::<document_core::PathNode>();
        match shape {
            Shape::Compound { contours } => {
                nodes + contours.capacity() * size_of::<Vec<document_core::PathNode>>()
            }
            _ => nodes,
        }
    };
    keys * 128
        + object.name.capacity()
        + object
            .image
            .as_ref()
            .filter(|image| seen.insert(Arc::as_ptr(image).cast()))
            .map_or(0, |image| {
                size_of::<document_core::RasterImage>()
                    + image.rgba().len()
                    + image.encoded_base64().len()
            })
        + object
            .group_name
            .as_ref()
            .filter(|name| seen.insert(Arc::as_ptr(name).cast()))
            .map_or(0, |name| name.len() + 2 * size_of::<usize>())
        + object.clip_source.as_ref().map_or(0, |binding| {
            if seen.insert(Arc::as_ptr(&binding.source).cast()) {
                size_of::<document_core::ClipSource>()
                    + object_heap_bytes(&binding.source.object, seen)
            } else {
                0
            }
        })
        + object
            .clip_instance
            .as_ref()
            .and_then(|i| i.shared_clip())
            .filter(|clip| seen.insert(Arc::as_ptr(clip).cast()))
            .map_or(0, |clip| {
                size_of::<document_core::Clip>() + clip_heap_bytes(clip, seen)
            })
        + shape_bytes(&object.shape)
        + object.skin.as_ref().map_or(0, |skin| {
            size_of::<document_core::SkinBinding>()
                + skin.weights.capacity() * size_of::<Vec<f32>>()
                + skin.weights.iter().map(Vec::capacity).sum::<usize>() * size_of::<f32>()
        })
        + object
            .animated
            .geometry_keys
            .values()
            .map(shape_bytes)
            .sum::<usize>()
        + object
            .stencil
            .as_ref()
            .map_or(0, |stencil| shape_bytes(&stencil.shape))
        + object.gradient.as_ref().map_or(0, |gradient| {
            gradient.stops.capacity() * size_of::<document_core::GradientStop>()
        })
        + object.effects.capacity() * size_of::<document_core::PathEffect>()
        + object.stroke_profile.capacity() * size_of::<[f32; 2]>()
        + object
            .text
            .as_ref()
            .map_or(0, |text| text.content.capacity())
        + object
            .static_artwork()
            .map_or(0, |art| shared_document_bytes(art, seen))
}

// Each transaction stores only the replaced objects. On undo or redo the document trades
// its current values with the transaction, so geometry is never duplicated on both stacks.
struct Change {
    objects: ObjectEdits,
    layers: Option<Vec<Layer>>,
    skeleton: Option<Skeleton>,
    guides_x: Option<Vec<f32>>,
    guides_y: Option<Vec<f32>>,
    dimensions: [u32; 4],
    next_id: u64,
}

enum ObjectEdits {
    InPlace(Vec<(usize, VectorObject)>),
    Structural {
        objects: Vec<(u64, Option<VectorObject>)>,
        order: Vec<u64>,
    },
}

/// The middle slices remaining after matching both ends, without double-counting
/// a shared prefix in a pure insertion or deletion.
fn changed_ranges<T>(
    before: &[T],
    after: &[T],
    retained: impl Fn(&T, &T) -> bool,
) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let same = |(a, b): &(&T, &T)| retained(a, b);
    let prefix = before.iter().zip(after).take_while(same).count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(same)
        .count();
    (prefix..before.len() - suffix, prefix..after.len() - suffix)
}

impl ObjectEdits {
    fn between(before: &[VectorObject], after: &[VectorObject]) -> Self {
        let old_ids = before.iter().map(|o| o.id);
        if before.len() == after.len() && old_ids.clone().eq(after.iter().map(|o| o.id)) {
            let mut changed = Vec::new();
            for (index, (old, new)) in before.iter().zip(after).enumerate() {
                if old != new {
                    changed.push((index, old.clone()));
                }
            }
            return Self::InPlace(changed);
        }
        let (old_range, new_range) = changed_ranges(before, after, PartialEq::eq);
        let mut new: HashMap<_, _> = after[new_range].iter().map(|o| (o.id, o)).collect();
        let mut objects = Vec::new();
        for old in &before[old_range] {
            if new.remove(&old.id) != Some(old) {
                objects.push((old.id, Some(old.clone())));
            }
        }
        objects.extend(new.into_keys().map(|id| (id, None)));
        Self::Structural {
            objects,
            order: old_ids.collect(),
        }
    }

    fn swap(&mut self, art: &mut Vec<VectorObject>) {
        match self {
            Self::InPlace(changes) => {
                for (index, saved) in changes {
                    // History normally restores each structural edit before reaching
                    // this entry. Keep identity if the caller reordered the drawing.
                    if art.get(*index).is_none_or(|o| o.id != saved.id) {
                        let Some(found) = art.iter().position(|o| o.id == saved.id) else {
                            continue;
                        };
                        *index = found;
                    }
                    mem::swap(&mut art[*index], saved);
                }
            }
            Self::Structural { objects, order } => {
                let target = mem::replace(order, art.iter().map(|o| o.id).collect());
                // Keep matching objects at either end in place. IDs whose geometry
                // changes (including the back operand of a Boolean) must still swap.
                let edited: HashSet<_> = objects.iter().map(|(id, _)| *id).collect();
                let (wanted, current) =
                    changed_ranges(&target, order, |a, b| a == b && !edited.contains(a));
                let start = current.start;
                let mut scene: HashMap<_, _> = art.drain(current).map(|o| (o.id, o)).collect();
                for (id, saved) in objects {
                    let current = scene.remove(id);
                    if let Some(old) = mem::replace(saved, current) {
                        scene.insert(*id, old);
                    }
                }
                let restored = target[wanted].iter().filter_map(|id| scene.remove(id));
                art.splice(start..start, restored);
                debug_assert!(
                    scene.is_empty(),
                    "object order and undo transaction diverged"
                );
            }
        }
    }

    fn bytes(&self, seen: &mut HashSet<*const ()>) -> usize {
        match self {
            Self::InPlace(changes) => {
                changes.capacity() * size_of::<(usize, VectorObject)>()
                    + changes
                        .iter()
                        .map(|(_, o)| object_heap_bytes(o, seen))
                        .sum::<usize>()
            }
            Self::Structural { objects, order } => {
                objects.capacity() * size_of::<(u64, Option<VectorObject>)>()
                    + order.capacity() * size_of::<u64>()
                    + objects
                        .iter()
                        .filter_map(|(_, o)| o.as_ref())
                        .map(|o| object_heap_bytes(o, seen))
                        .sum::<usize>()
            }
        }
    }
}

impl Change {
    fn between(before: &Document, after: &Document) -> Self {
        Self {
            objects: ObjectEdits::between(&before.objects, &after.objects),
            skeleton: (before.skeleton != after.skeleton).then(|| before.skeleton.clone()),
            layers: (before.layers != after.layers).then(|| before.layers.clone()),
            guides_x: (before.guides_x != after.guides_x).then(|| before.guides_x.clone()),
            guides_y: (before.guides_y != after.guides_y).then(|| before.guides_y.clone()),
            dimensions: [before.width, before.height, before.frames, before.fps],
            next_id: before.next_id,
        }
    }

    fn swap(&mut self, doc: &mut Document) {
        self.objects.swap(&mut doc.objects);
        if let Some(skeleton) = &mut self.skeleton {
            mem::swap(skeleton, &mut doc.skeleton);
        }
        if let Some(layers) = &mut self.layers {
            mem::swap(layers, &mut doc.layers);
        }
        if let Some(guides) = &mut self.guides_x {
            mem::swap(guides, &mut doc.guides_x);
        }
        if let Some(guides) = &mut self.guides_y {
            mem::swap(guides, &mut doc.guides_y);
        }
        for (old, current) in self.dimensions.iter_mut().zip([
            &mut doc.width,
            &mut doc.height,
            &mut doc.frames,
            &mut doc.fps,
        ]) {
            mem::swap(old, current);
        }
        mem::swap(&mut self.next_id, &mut doc.next_id);
    }

    /// Approximate owned memory, including a conservative allowance for B-tree nodes.
    fn bytes(&self, seen: &mut HashSet<*const ()>) -> usize {
        let mut bytes = size_of::<Self>() + self.objects.bytes(seen);
        if let Some(skeleton) = &self.skeleton {
            bytes += skeleton_heap_bytes(skeleton);
        }
        if let Some(layers) = &self.layers {
            bytes += layers.capacity() * size_of::<Layer>();
            bytes += layers
                .iter()
                .map(|layer| layer.name.capacity())
                .sum::<usize>();
        }
        for guides in [&self.guides_x, &self.guides_y].into_iter().flatten() {
            bytes += guides.capacity() * size_of::<f32>();
        }
        bytes
    }
}

pub use document_core::arrange::{editable_selection, is_editable, Alignment};

/// Remove missing or uneditable IDs in place, retaining their order. Returns whether any
/// remaining object could move to the specified destination layer.
pub fn retain_editable_selection(doc: &Document, selected: &mut Vec<u64>, target: u64) -> bool {
    let mut remaining = doc.objects.iter();
    let mut by_id: Option<HashMap<u64, u64>> = None;
    let small = selected.len() <= 16;
    let mut on_other_layer = false;
    selected.retain(|id| {
        // A marquee returns IDs in document order. Walk the document once,
        // building an index only when a larger selection arrives in another order.
        let layer = if let Some(index) = &by_id {
            index.get(id).copied()
        } else if let Some(object) = remaining.find(|o| o.id == *id) {
            (!object.locked).then_some(object.layer)
        } else if small {
            doc.objects
                .iter()
                .find(|o| o.id == *id && !o.locked)
                .map(|o| o.layer)
        } else {
            let index = by_id.insert(
                doc.objects
                    .iter()
                    .filter(|o| !o.locked)
                    .map(|o| (o.id, o.layer))
                    .collect(),
            );
            index.get(id).copied()
        };
        let Some(layer) = layer.filter(|&layer| is_editable(doc, layer)) else {
            return false;
        };
        on_other_layer |= layer != target;
        true
    });
    on_other_layer && is_editable(doc, target)
}

fn next_free(used: &HashSet<u64>) -> u64 {
    let mut id = used
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .wrapping_add(1)
        .max(1);
    while used.contains(&id) {
        id = id.wrapping_add(1).max(1);
    }
    id
}

#[cfg(test)]
pub fn add_layer(doc: &mut Document) -> u64 {
    let id = next_free(&doc.layers.iter().map(|l| l.id).collect());
    add_layer_with_id(doc, id)
}

pub fn add_layer_with_id(doc: &mut Document, id: u64) -> u64 {
    doc.layers.push(Layer {
        id,
        name: format!("Layer {id}"),
        visible: true,
        locked: false,
        blend_mode: Default::default(),
    });
    id
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupAction {
    Group,
    Ungroup,
}

impl GroupAction {
    pub(crate) fn enabled(self, doc: &Document, ids: &[u64]) -> bool {
        let options = grouping_options(doc, ids);
        match self {
            Self::Group => options.can_group,
            Self::Ungroup => options.can_ungroup,
        }
    }

    pub(crate) fn apply(self, doc: &mut Document, selected: &mut Vec<u64>) {
        if !self.enabled(doc, selected) {
            return;
        }
        // A selection can identify a whole group by just one of its members.
        // Expand it before ungrouping so all released objects stay selected.
        *selected = editable_selection(doc, selected);
        let members: HashSet<_> = selected.iter().copied().collect();
        let group = match self {
            Self::Group => Some(next_free(
                &doc.objects.iter().filter_map(|o| o.group).collect(),
            )),
            Self::Ungroup => None,
        };
        for object in doc.objects.iter_mut().filter(|o| members.contains(&o.id)) {
            object.group = group;
            object.group_name = None;
        }
    }
}

#[derive(Default)]
pub(crate) struct GroupingOptions {
    pub can_group: bool,
    pub can_ungroup: bool,
}

pub(crate) fn grouping_options(doc: &Document, ids: &[u64]) -> GroupingOptions {
    if ids.is_empty() {
        return GroupingOptions::default();
    }
    let selected: HashSet<_> = ids.iter().copied().collect();
    let mut members = doc
        .objects
        .iter()
        .filter(|o| selected.contains(&o.id) && is_editable(doc, o.layer));
    let Some(first) = members.next() else {
        return GroupingOptions::default();
    };
    let mut same_layer = true;
    let mut separate_items = false;
    let mut grouped = first.group.is_some();
    for object in members {
        same_layer &= object.layer == first.layer;
        separate_items |= first.group.is_none() || object.group != first.group;
        grouped |= object.group.is_some();
    }
    GroupingOptions {
        can_group: same_layer && separate_items,
        can_ungroup: grouped,
    }
}

#[cfg(test)]
pub(crate) fn group(doc: &mut Document, ids: &[u64]) {
    GroupAction::Group.apply(doc, &mut ids.to_vec());
}

#[cfg(test)]
pub fn delete(doc: &mut Document, ids: &[u64]) {
    delete_in(doc, ids, Default::default());
}

pub fn delete_in(doc: &mut Document, ids: &[u64], scope: SelectionScope) {
    let selected: HashSet<_> = scope.resolve(doc, ids).into_iter().collect();
    doc.objects.retain(|o| !selected.contains(&o.id));
}

#[cfg(test)]
pub fn duplicate(doc: &mut Document, ids: &[u64]) -> Vec<u64> {
    duplicate_in(doc, ids, Default::default())
}

pub fn duplicate_in(doc: &mut Document, ids: &[u64], scope: SelectionScope) -> Vec<u64> {
    let selected = scope.resolve(doc, ids);
    if selected.is_empty() {
        return Vec::new();
    }
    let mut used_ids: HashSet<_> = doc.objects.iter().map(|o| o.id).collect();
    let mut groups = CopiedGroups::new(doc);
    let mut copy_ids = Vec::with_capacity(selected.len());
    let original_count = doc.objects.len();
    doc.objects.reserve(selected.len());
    // editable_selection returns objects in drawing order, including group members.
    let mut selected = selected.into_iter().peekable();
    for index in 0..original_count {
        let source = &doc.objects[index];
        if selected.next_if_eq(&source.id).is_none() {
            continue;
        }
        let mut copy = source.clone();
        copy.id = take_free_id(&mut doc.next_id, &mut used_ids);
        match scope {
            SelectionScope::Groups => copy.group = groups.for_object(source),
            SelectionScope::Objects => {
                copy.group = None;
                copy.group_name = None;
            }
            SelectionScope::InGroup(_) => {}
        }
        copy_ids.push(copy.id);
        // Appending keeps copies on their original layers and preserves their paint order.
        doc.objects.push(copy);
    }
    copy_ids
}

struct CopiedGroups {
    used: HashSet<u64>,
    next: u64,
    copies: HashMap<(u64, u64), u64>,
}

fn take_free_id(next: &mut u64, used: &mut HashSet<u64>) -> u64 {
    let id = *next;
    used.insert(id);
    while used.contains(next) {
        *next = next.wrapping_add(1).max(1);
    }
    id
}

impl CopiedGroups {
    fn new(doc: &Document) -> Self {
        let used = doc.objects.iter().filter_map(|o| o.group).collect();
        Self {
            next: next_free(&used),
            used,
            copies: HashMap::new(),
        }
    }

    fn for_object(&mut self, source: &VectorObject) -> Option<u64> {
        let group = source.group?;
        Some(
            *self
                .copies
                .entry((source.layer, group))
                .or_insert_with(|| take_free_id(&mut self.next, &mut self.used)),
        )
    }
}

#[derive(Clone, Default)]
pub struct Clipboard {
    source_frame: u32,
    clip_sources: Option<clipboard_symbols::ClipSources>,
    objects: Vec<VectorObject>,
    symbols: std::collections::BTreeMap<u64, document_core::SymbolDefinition>,
}

const CLIPBOARD_HEADER: &str = "vector-objects/1";
const CLIPBOARD_LIMIT: usize = 16 * 1024 * 1024;

// Copies can share a large drawing in memory. Stop the external clipboard
// encoding at its byte limit even when those drawings expand repeatedly in JSON.
fn encode_clipboard(header: &str, content: &impl serde::Serialize) -> Option<String> {
    struct Payload(Vec<u8>);
    impl std::io::Write for Payload {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > CLIPBOARD_LIMIT - self.0.len() {
                return Err(std::io::Error::other("Clipboard size limit exceeded."));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Payload(format!("Phyro/{header}").into_bytes());
    output.0.push(b'\n');
    serde_json::to_writer(&mut output, content).ok()?;
    String::from_utf8(output.0).ok()
}

impl Clipboard {
    pub fn strip_format_prefix(text: &str) -> Option<&str> {
        text.strip_prefix("Phyro/")
            .or_else(|| text.strip_prefix("Loopline/"))
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn encode(&self) -> Option<String> {
        // Images live in the native project resource table. The vector-only
        // system clipboard payload cannot carry them; keep the full session copy.
        if self.is_empty() || self.contains_image() {
            return None;
        }
        if self.source_frame != 0 || self.clip_sources.is_some() || !self.symbols.is_empty() {
            return self.encode_with_symbols();
        }
        encode_clipboard(CLIPBOARD_HEADER, &self.objects)
    }

    fn contains_image(&self) -> bool {
        let mut pending = vec![self.objects.as_slice()];
        pending.extend(self.symbols.values().map(|d| d.artwork.objects.as_slice()));
        let mut seen = HashSet::new();
        while let Some(objects) = pending.pop() {
            for object in objects {
                if object.image.is_some() {
                    return true;
                }
                if let Some(art) = &object.embedded_artwork {
                    if seen.insert(Arc::as_ptr(art)) {
                        pending.push(art.objects.as_slice());
                    }
                }
            }
        }
        false
    }

    pub fn decode(text: &str) -> Option<Self> {
        if text.len() > CLIPBOARD_LIMIT {
            return None;
        }
        let (header, payload) = Self::strip_format_prefix(text)?.split_once('\n')?;
        let mut decoded = match header.trim_end_matches('\r') {
            CLIPBOARD_HEADER => Self {
                objects: serde_json::from_str(payload).ok()?,
                ..Self::default()
            },
            clipboard_symbols::HEADER
            | clipboard_symbols::COMPOSITION_HEADER
            | clipboard_symbols::FRAME_HEADER => Self::decode_with_symbols(payload)?,
            _ => return None,
        };
        let objects = std::mem::take(&mut decoded.objects);
        if objects.is_empty() {
            return None;
        }

        // Reuse the native scene validator before clipboard contents can enter a project.
        // Each source layer needs its own identity until paste remaps the groups.
        let layer_ids: HashSet<_> = objects.iter().map(|o| o.layer).collect();
        let layers = layer_ids
            .into_iter()
            .map(|id| Layer {
                id,
                ..Layer::default()
            })
            .collect();
        let next_id = next_free(&objects.iter().map(|o| o.id).collect());
        let mut document = Document {
            next_id,
            layers,
            objects,
            frames: document_core::MAX_FRAMES,
            ..Document::default()
        };
        // Clip caches belong to the destination project. Check the remaining authored
        // geometry now and validate the actual references before publishing the paste.
        let clip_links: Vec<_> = document
            .objects
            .iter_mut()
            .map(|o| o.clip_instance.take())
            .collect();
        if decoded.clip_sources.is_none() && clip_links.iter().any(Option::is_some)
            || clip_links
                .iter()
                .flatten()
                .any(|i| i.validate_timing(document.frames).is_err())
        {
            return None;
        }
        document.validate().ok()?;
        for (object, link) in document.objects.iter_mut().zip(clip_links) {
            object.clip_instance = link;
        }
        decoded.objects = document.objects;
        Some(decoded)
    }

    #[cfg(test)]
    pub fn copy(&mut self, doc: &Document, ids: &[u64], frame: u32) {
        self.copy_in(doc, ids, frame, SelectionScope::Groups);
    }

    fn copy_in(&mut self, doc: &Document, ids: &[u64], frame: u32, scope: SelectionScope) {
        let selected: HashSet<_> = scope.resolve(doc, ids).into_iter().collect();
        if !selected.is_empty() {
            self.objects = doc
                .paint_order()
                .into_iter()
                .filter(|o| selected.contains(&o.id))
                .cloned()
                .map(|mut object| {
                    // Bone IDs are drawing-local. Carry the posed artwork into other drawings.
                    object.release_rig_at(frame);
                    if scope != SelectionScope::Groups {
                        object.group = None;
                        object.group_name = None;
                    }
                    object
                })
                .collect();
            // Only animated paint needs a source-frame marker. Static copies
            // keep the existing native clipboard formats and their paste behavior.
            self.source_frame = if frame != 0
                && self.style_source().is_some_and(|o| {
                    o.fill_at(frame) != o.fill_at(0)
                        || o.stroke_at(frame) != o.stroke_at(0)
                        || o.stroke_width_at(frame) != o.stroke_width_at(0)
                }) {
                frame
            } else {
                0
            };
            self.capture_definitions();
        }
    }

    pub fn paste(&self, doc: &mut Document, layer: u64) -> Vec<u64> {
        if !is_editable(doc, layer) {
            return Vec::new();
        }
        let mut groups = CopiedGroups::new(doc);
        let mut pasted = Vec::with_capacity(self.objects.len());
        for src in &self.objects {
            let group = groups.for_object(src);
            let id = doc.add_shape(src.position, src.size, src.fill, Shape::Rectangle, layer);
            let mut copy = src.clone();
            copy.id = id;
            copy.layer = layer;
            copy.group = group;
            // Copy between projects without losing authored keys outside the destination clip.
            if let Some(last) = copy.last_keyframe() {
                doc.frames = doc.frames.max(last + 1);
            }
            *doc.objects
                .last_mut()
                .expect("add_shape appended an object") = copy;
            pasted.push(id);
        }
        pasted
    }
}

pub fn move_to_layer(doc: &mut Document, ids: &[u64], layer: u64) {
    if !is_editable(doc, layer) {
        return;
    }
    let selection: HashSet<_> = editable_selection(doc, ids).into_iter().collect();
    let mut groups = CopiedGroups::new(doc);
    let order: HashMap<_, _> = doc
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id, i))
        .collect();
    let (mut moved, remaining): (Vec<_>, Vec<_>) = mem::take(&mut doc.objects)
        .into_iter()
        .partition(|o| o.layer != layer && selection.contains(&o.id));
    moved.sort_by_key(|o| order[&o.layer]);
    for o in &mut moved {
        o.group = groups.for_object(o);
        o.layer = layer;
    }
    doc.objects = remaining;
    doc.objects.extend(moved);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(doc: &mut Document, x: f32) -> u64 {
        doc.add_rectangle([x, 12.0], [8.0, 8.0], [255; 4])
    }

    #[test]
    fn copying_an_embedded_image_preserves_pixels_without_exposing_vector_paint() {
        let mut doc = Document::default();
        let id = rectangle(&mut doc, 30.);
        let image = Arc::new(document_core::RasterImage::from_base64(
            "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAF0lEQVR4nGP4z8DwnwEM/jM0MDD8/w8AK2kFfIvAcEgAAAAASUVORK5CYII=",
        ).unwrap());
        doc.objects[0].image = Some(image.clone());
        let mut clipboard = Clipboard::default();
        clipboard.copy(&doc, &[id], 0);
        assert!(clipboard.encode().is_none());
        assert!(!clipboard.has_style());
        assert!(!has_style_target(&doc, &[id]));
        let copies = clipboard.paste(&mut doc, 1);
        assert_eq!(copies.len(), 1);
        let copy = doc.objects.last().unwrap();
        assert!(Arc::ptr_eq(copy.image.as_ref().unwrap(), &image));
        let mut seen = HashSet::new();
        let first = object_heap_bytes(&doc.objects[0], &mut seen);
        let second = object_heap_bytes(copy, &mut seen);
        assert_eq!(
            first - second,
            size_of::<document_core::RasterImage>()
                + image.encoded_base64().len()
                + image.rgba().len()
        );
    }

    #[test]
    fn duplicate_keeps_layers_and_rig_tracks_and_gives_groups_independent_ids() {
        let mut doc = Document::default();
        let ids = [8., 20., 40., 64.].map(|x| rectangle(&mut doc, x));
        let second_layer = add_layer(&mut doc);
        let locked_layer = add_layer(&mut doc);
        doc.layers.last_mut().unwrap().locked = true;
        doc.objects[2].layer = second_layer;
        doc.objects[3].layer = locked_layer;
        for object in &mut doc.objects {
            object.group = Some(u64::MAX);
        }
        doc.rename_group(1, u64::MAX, "Lanterns").unwrap();
        doc.rename_group(second_layer, u64::MAX, "Poles").unwrap();
        let arm = doc.add_bone("Arm", None, [8., 12.], [20., 12.]).unwrap();
        doc.bind_rigid(ids[0], Some(arm)).unwrap();
        doc.set_bone_rotation_key(arm, 3, 45.).unwrap();
        doc.objects[1].rotation_keys.insert(2, 40.);
        doc.next_id = u64::MAX;
        let sources = doc.objects.clone();
        let copies = duplicate(&mut doc, &[ids[3], ids[2], ids[0]]);
        assert_eq!(copies.len(), 3);
        assert_eq!(copies[0], u64::MAX);
        assert_eq!(&doc.objects[..4], &sources);
        let objects = &doc.objects[4..];
        assert_eq!(objects[0].group, objects[1].group);
        assert_ne!(objects[1].group, objects[2].group);
        for ((original, copy), &id) in sources.iter().zip(objects).zip(&copies) {
            assert_ne!(original.group, copy.group);
            let mut expected = original.clone();
            expected.id = id;
            expected.group = copy.group;
            assert_eq!(copy, &expected);
        }
        doc.validate().unwrap();
    }

    #[test]
    fn rig_history_restores_empty_skeleton_bindings_and_cached_poses() {
        let mut drawing = Document::default();
        let art = rectangle(&mut drawing, 10.);
        let mut p = AnimationProject::from_document(drawing);
        let original = p.clone();
        let mut history = History::default();
        let doc = p.clips[0].drawing_at_mut(0);
        let hip = doc.add_bone("Hip", None, [10., 10.], [18., 10.]).unwrap();
        let foot = doc
            .add_bone("Foot", Some(hip), [18., 10.], [22., 10.])
            .unwrap();
        // An edit containing only bone IDs and hierarchy still needs its own undo step.
        history.record(&original, &p, 0, 0);
        let hierarchy = p.clone();
        let doc = p.clips[0].drawing_at_mut(0);
        doc.bind_rigid(art, Some(foot)).unwrap();
        doc.set_bone_rotation(hip, 90.).unwrap();
        history.record(&hierarchy, &p, 0, 0);
        let posed = p.clone();
        let pose = posed.clips[0].drawing_at(0).objects[0].position_at(0);
        assert_ne!(
            pose,
            original.clips[0].drawing_at(0).objects[0].position_at(0)
        );
        for expected in [&hierarchy, &original] {
            assert!(history.undo(&mut p));
            p.validate().unwrap();
            assert_eq!(&p, expected);
        }
        for expected in [&hierarchy, &posed] {
            assert!(history.redo(&mut p));
            p.validate().unwrap();
            assert_eq!(&p, expected);
        }
        assert_eq!(p.clips[0].drawing_at(0).objects[0].position_at(0), pose);
    }

    #[test]
    fn copying_a_tweened_bone_at_the_playhead_detaches_the_visible_pose() {
        let mut doc = Document::default();
        let art = rectangle(&mut doc, 22.);
        let arm = doc.add_bone("Arm", None, [20., 20.], [32., 20.]).unwrap();
        doc.bind_rigid(art, Some(arm)).unwrap();
        doc.set_bone_rotation_key(arm, 0, 0.).unwrap();
        doc.set_bone_rotation_key(arm, 4, 90.).unwrap();
        let project = AnimationProject::from_document(doc);
        let mut clipboard = Clipboard::default();
        clipboard.copy_project(&project, 0, 2, &[art]);
        let decoded = Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
        let mut target = Document::default();
        decoded.paste(&mut target, 1);
        let doc = project.clips[0].drawing_at(2);
        let selected_pose = renderer::render(doc, 2).unwrap();
        assert_ne!(
            selected_pose.data(),
            renderer::render(doc, 0).unwrap().data()
        );
        assert_eq!(
            selected_pose.data(),
            renderer::render(&target, 2).unwrap().data()
        );
        assert_eq!(
            selected_pose.data(),
            renderer::render(&target, 4).unwrap().data()
        );
        assert_eq!(target.objects[0].rigid_bone, None);
    }

    #[test]
    fn clipboard_transfers_a_bent_path_to_a_drawing_without_bones() {
        let mut doc = Document::default();
        let upper = doc.add_bone("Arm", None, [20., 32.], [40., 32.]).unwrap();
        let lower = doc
            .add_bone("Wrist", Some(upper), [40., 32.], [60., 32.])
            .unwrap();
        let path = document_core::path_edit::primitive_path(&Shape::Rectangle).unwrap();
        let art = doc.add_shape([20., 30.], [40., 4.], [255; 4], path, 1);
        doc.bind_skin_at(art, lower, 0).unwrap();
        doc.set_bone_rotation_key(lower, 0, 0.).unwrap();
        doc.set_bone_rotation_key(lower, 4, 90.).unwrap();
        let mut clipboard = Clipboard::default();
        clipboard.copy(&doc, &[art], 2);
        let decoded = Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
        let mut target = Document::default();
        decoded.paste(&mut target, 1);
        target.validate().unwrap();
        assert!(target.objects[0].skin.is_none());
        let posed = renderer::render(&doc, 2).unwrap();
        assert_ne!(posed.data(), renderer::render(&doc, 0).unwrap().data());
        assert_eq!(posed.data(), renderer::render(&target, 2).unwrap().data());
    }

    #[test]
    fn copy_carries_posed_keyframes_and_duplicate_frame_keeps_the_local_binding() {
        let mut source = Document::default();
        let art = rectangle(&mut source, 22.);
        source.objects[0]
            .keys
            .extend([(0, [22., 12.]), (4, [35., 17.])]);
        source.objects[0].rotation_keys.extend([(0, 6.), (4, 44.)]);
        let arm = source
            .add_bone("Arm", None, [20., 20.], [32., 20.])
            .unwrap();
        source.bind_rigid(art, Some(arm)).unwrap();
        source.set_bone_rotation(arm, 35.).unwrap();
        let mut clipboard = Clipboard::default();
        clipboard.copy(&source, &[art], 0);
        let decoded = Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
        let mut target = Document::default();
        let foreign = target.add_bone("Door", None, [4., 4.], [4., 16.]).unwrap();
        assert_eq!(arm, foreign);
        decoded.paste(&mut target, 1);
        target.set_bone_rotation(foreign, 170.).unwrap();
        assert_eq!(target.objects[0].rigid_bone, None);
        for t in 0..source.frames {
            assert_eq!(
                target.objects[0].position_at(t),
                source.objects[0].position_at(t)
            );
            assert_eq!(
                target.objects[0].rotation_at(t),
                source.objects[0].rotation_at(t)
            );
            assert_eq!(
                renderer::render(&target, t).unwrap().data(),
                renderer::render(&source, t).unwrap().data()
            );
        }
        let mut project = AnimationProject::from_document(source);
        let clip = &mut project.clips[0];
        let new = clip.duplicate_frame(2).unwrap();
        assert_eq!(clip.drawing_at(new).objects[0].rigid_bone, Some(arm));
        assert_eq!(
            clip.drawing_at(2).objects[0].position_at(2),
            clip.drawing_at(new).objects[0].position_at(new)
        );
        clip.drawing_at_mut(new)
            .set_bone_rotation(arm, 90.)
            .unwrap();
        assert_eq!(clip.drawing_at(2).skeleton.bones[0].rotation, 35.);
        project.validate().unwrap();
    }

    #[test]
    fn history_restores_content_identity_order_and_guides_and_discards_a_redo_branch() {
        let mut doc = Document::default();
        let a = rectangle(&mut doc, 10.0);
        let b = rectangle(&mut doc, 30.0);
        let initial = AnimationProject::from_document(doc);
        let mut project = initial.clone();
        let mut history = History::default();
        let doc = project.clips[0].drawing_at_mut(0);
        let layer = add_layer(doc);
        move_to_layer(doc, &[a], layer);
        doc.guides_x.push(16.0);
        doc.objects[0].keys.insert(4, [23.0, 19.0]);
        history.record(&initial, &project, 0, 0);
        let moved = project.clone();
        let doc = project.clips[0].drawing_at_mut(0);
        delete(doc, &[b]);
        rectangle(doc, 50.0);
        history.record(&moved, &project, 0, 0);
        let replaced = project.clone();

        assert!(history.undo(&mut project));
        assert_eq!(project, moved);
        assert!(history.undo(&mut project));
        assert_eq!(project, initial);
        assert!(history.redo(&mut project));
        assert!(history.redo(&mut project));
        assert_eq!(project, replaced);
        history.undo(&mut project);
        let mut branched = project.clone();
        branched.clips[0].drawing_at_mut(0).guides_y.push(36.0);
        history.record(&project, &branched, 0, 0);
        assert!(!history.can_redo());
    }

    #[test]
    fn undo_across_clip_deletion_recovers_each_edited_drawing_and_focus() {
        let mut doc = Document::default();
        rectangle(&mut doc, 7.0);
        let mut project = AnimationProject::from_document(doc);
        project.duplicate_clip(0, "Walk").unwrap();
        project.clips[1].duplicate_frame(0).unwrap();
        project.clips[1].set_exposure(1, 2).unwrap();
        let initial = project.clone();
        let mut history = History::default();
        project.clips[1].drawing_at_mut(1).objects[0].position = [51., 2.];
        history.record(&initial, &project, 1, 2);
        assert!(matches!(
            history.past.back().unwrap().edit,
            ProjectEdit::Drawing(_)
        ));
        let moved = project.clone();
        project.clips.remove(1);
        history.record(&moved, &project, 0, 2);
        assert!(history.undo(&mut project));
        assert_eq!(history.location(), Some((1, 2)));
        assert_eq!(project, moved);
        assert!(history.undo(&mut project));
        assert_eq!(project, initial);
        assert!(history.redo(&mut project));
        assert_eq!(project, moved);
        assert!(history.redo(&mut project));
        assert_eq!(history.location(), Some((0, 2)));
        assert_eq!(project.clips.len(), 1);
    }

    #[test]
    fn single_editable_selection_expands_only_its_layer_and_preserves_document_order() {
        let mut doc = Document::default();
        let a = rectangle(&mut doc, 5.);
        let plain = rectangle(&mut doc, 15.);
        let b = rectangle(&mut doc, 25.);
        let last = rectangle(&mut doc, 35.);
        let top = add_layer(&mut doc);
        doc.objects[0].group = Some(71);
        doc.objects[2].group = Some(71);
        doc.objects[3].group = Some(71);
        doc.objects[3].layer = top;
        assert_eq!(editable_selection(&doc, &[b]), [a, b]);
        assert_eq!(editable_selection(&doc, &[plain]), [plain]);
        assert_eq!(editable_selection(&doc, &[last]), [last]);
        assert!(editable_selection(&doc, &[u64::MAX]).is_empty());
        doc.layers[0].locked = true;
        assert!(editable_selection(&doc, &[a]).is_empty());
        assert!(editable_selection(&doc, &[plain]).is_empty());
        doc.layers[1].visible = false;
        assert!(editable_selection(&doc, &[last]).is_empty());
    }

    #[test]
    fn paste_remaps_each_group_and_preserves_paint_order_and_keyed_poses() {
        let mut doc = Document::default();
        let a = rectangle(&mut doc, 10.0);
        let b = rectangle(&mut doc, 30.0);
        group(&mut doc, &[a, b]);
        let group_id = doc.objects[0].group.unwrap();
        doc.objects[1].keys.insert(4, [26.0, 17.0]);
        let dst = add_layer(&mut doc);
        let mut clipboard = Clipboard::default();
        clipboard.copy(&doc, &[b], 0);
        let pasted = clipboard.paste(&mut doc, dst);
        assert_eq!(pasted.len(), 2);
        assert!(!pasted.contains(&a) && !pasted.contains(&b));
        assert_eq!(doc.objects[2].position, doc.objects[0].position);
        assert_eq!(doc.objects[3].keys, doc.objects[1].keys);
        assert_ne!(doc.objects[2].group, Some(group_id));
        assert_eq!(doc.objects[2].group, doc.objects[3].group);
        assert_eq!(doc.objects[3].layer, dst);
        doc.validate().unwrap();

        let before = doc.clone();
        doc.layers[1].locked = true;
        delete(&mut doc, &pasted);
        assert!(clipboard.paste(&mut doc, dst).is_empty());
        assert_eq!(doc.objects, before.objects);
    }

    #[test]
    fn grouping_options_follow_editable_items_and_layer_local_group_identity() {
        let mut doc = Document::default();
        let other_layer = add_layer(&mut doc);
        let a = doc.add_rectangle([1.; 2], [8.; 2], [28; 4]);
        let b = doc.add_rectangle([11.; 2], [8.; 2], [44; 4]);
        let c = doc.add_rectangle([21.; 2], [8.; 2], [78; 4]);
        doc.objects[2].layer = other_layer;
        assert!(GroupAction::Group.enabled(&doc, &[a, b]));
        assert!(!GroupAction::Ungroup.enabled(&doc, &[a, b]));
        assert!(!GroupAction::Group.enabled(&doc, &[a, c]));
        group(&mut doc, &[a, b]);
        doc.objects[2].group = doc.objects[0].group;
        let group = doc.objects[0].group.unwrap();
        doc.rename_group(1, group, "Pair").unwrap();
        doc.rename_group(other_layer, group, "Single").unwrap();
        assert!(!GroupAction::Group.enabled(&doc, &[a, b]));
        assert!(GroupAction::Ungroup.enabled(&doc, &[b]));
        let external = doc.objects[2].clone();
        let mut picked = vec![b];
        GroupAction::Ungroup.apply(&mut doc, &mut picked);
        assert_eq!(picked, [a, b]);
        assert!(doc.objects[..2].iter().all(|o| o.group_name.is_none()));
        assert_eq!(doc.objects[2], external);
        doc.validate().unwrap();
        doc.layers[1].locked = true;
        assert!(!GroupAction::Ungroup.enabled(&doc, &[c]));
        doc.layers[1].locked = false;
        doc.layers[1].visible = false;
        assert!(!GroupAction::Ungroup.enabled(&doc, &[c]));
    }

    #[test]
    fn align_group_keeps_member_spacing_and_inserts_only_the_current_key() {
        let mut doc = Document::default();
        let a = rectangle(&mut doc, 20.0);
        let b = rectangle(&mut doc, 30.0);
        let c = rectangle(&mut doc, 8.0);
        group(&mut doc, &[a, b]);
        doc.objects[0]
            .keys
            .extend([(0, [20.0, 12.0]), (4, [28.0, 12.0])]);
        crate::editor::arrange::apply(
            &mut doc,
            &[b, c],
            2,
            crate::timeline::KeyMode::Tracked,
            crate::editor::arrange::Action::Align(
                Alignment::Left,
                crate::editor::arrange::Reference::Bounds,
            ),
        )
        .unwrap();
        assert_eq!(doc.objects[0].position_at(2), [8.0, 12.0]);
        assert_eq!(doc.objects[1].position, [14.0, 12.0]);
        assert_eq!(doc.objects[0].position, [20.0, 12.0]);
        assert_eq!(doc.objects[0].keys[&4], [28.0, 12.0]);
        GroupAction::Ungroup.apply(&mut doc, &mut vec![b]);
        assert!(doc.objects.iter().all(|o| o.group.is_none()));
    }

    #[test]
    fn system_clipboard_round_trip_preserves_editable_paths_and_outlying_keys() {
        let mut source = Document {
            frames: 12,
            ..Document::default()
        };
        let layer = add_layer(&mut source);
        let shape = Shape::Path {
            nodes: vec![
                document_core::PathNode::corner([0.0, 0.0]),
                document_core::PathNode {
                    handle_style: Default::default(),
                    point: [1.0, 1.0],
                    incoming: [0.8, 0.1],
                    outgoing: [1.0, 1.0],
                },
            ],
            closed: false,
        };
        let source_id = source.add_shape([2.0, 4.0], [8.0, 8.0], [0; 4], shape, layer);
        source.objects[0].stroke_width = 1.25;
        source.objects[0].rotation = 35.0;
        source.objects[0].keys.insert(11, [10.0, 15.0]);
        let mut copy = Clipboard::default();
        copy.copy(&source, &[source_id], 0);
        let payload = copy.encode().unwrap();
        assert!(Clipboard::decode(&payload.replacen("Phyro/", "Loopline/", 1)).is_some());
        let restored = Clipboard::decode(&payload.replace('\n', "\r\n")).unwrap();
        let mut destination = Document::default();
        let existing = rectangle(&mut destination, 16.0);
        let ids = restored.paste(&mut destination, 1);
        assert_ne!(ids, vec![existing]);
        let pasted = destination.objects.last().unwrap();
        assert_eq!(pasted.id, ids[0]);
        assert_eq!(pasted.layer, 1);
        assert_eq!(pasted.shape, source.objects[0].shape);
        assert_eq!(pasted.keys, source.objects[0].keys);
        assert_eq!(pasted.rotation, 35.0);
        assert_eq!(pasted.stroke_width, 1.25);
        assert_eq!(destination.frames, 12);
        destination.validate().unwrap();
    }

    #[test]
    fn system_clipboard_rejects_text_and_invalid_geometry() {
        assert!(Clipboard::decode("ordinary text").is_none());
        assert!(Clipboard::decode("Phyro/vector-objects/2\n[]").is_none());
        let mut doc = Document::default();
        let id = rectangle(&mut doc, 1.0);
        doc.objects[0].size[1] = -5.0;
        let mut invalid = Clipboard::default();
        invalid.copy(&doc, &[id], 0);
        assert!(Clipboard::decode(&invalid.encode().unwrap()).is_none());
    }
}
