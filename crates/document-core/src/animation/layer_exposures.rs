use super::{Clip, Document};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::RangeInclusive;
use std::sync::Arc;
mod structure;

// Layer IDs are nonzero. This owner preserves a whole-drawing rig boundary
// when a new independent layer spans existing drawings with separate rigs.
pub(super) const RIG: u64 = 0;

#[derive(Debug, Default)]
pub(super) struct LayerIndex {
    global: Vec<u32>,
    by_layer: BTreeMap<u64, Vec<u32>>,
}

fn bounds(starts: &[u32], frame: u32) -> (Option<u32>, Option<u32>) {
    let after = starts.partition_point(|t| *t <= frame);
    (starts[..after].last().copied(), starts.get(after).copied())
}

fn visible_after(starts: &[u32], first: u32, last: u32) -> &[u32] {
    let a = starts.partition_point(|t| *t <= first);
    let b = starts.partition_point(|t| *t <= last);
    &starts[a..b.max(a)]
}

impl Clip {
    pub fn rig_exposure(&self, frame: u32) -> (u32, u32) {
        self.layer_exposure(frame, RIG)
    }
    fn indexed_layer(&self, layer: u64) -> (&[u32], &[u32]) {
        let index = self.layer_index.get_or_init(|| {
            let mut index = LayerIndex::default();
            for &tick in self.cels.keys() {
                if let Some(owners) = self.layer_starts.get(&tick) {
                    for &id in owners {
                        index.by_layer.entry(id).or_default().push(tick);
                    }
                } else {
                    index.global.push(tick);
                }
            }
            Arc::new(index)
        });
        (
            &index.global,
            index.by_layer.get(&layer).map_or(&[], Vec::as_slice),
        )
    }

    fn starts_layer(&self, tick: u32, layer: u64) -> bool {
        self.layer_starts
            .get(&tick)
            .is_none_or(|ids| ids.contains(&layer))
    }

    /// Boundaries absent from the per-layer map are the independent, whole-document
    /// drawings authored by older versions and by explicit whole-clip operations.
    pub fn layer_exposure(&self, frame: u32, layer: u64) -> (u32, u32) {
        if self.layer_starts.is_empty() {
            return self.exposure(frame);
        }
        let (global, local) = self.indexed_layer(layer);
        let (g, gn) = bounds(global, frame);
        let (l, ln) = bounds(local, frame);
        let start = g.max(l).unwrap_or(0);
        let end = gn.unwrap_or(self.frames).min(ln.unwrap_or(self.frames));
        (start, end - start)
    }

    pub fn layer_drawing_starts(
        &self,
        layer: u64,
        visible: RangeInclusive<u32>,
    ) -> impl Iterator<Item = u32> + '_ {
        let first = *visible.start();
        let last = first.max(*visible.end());
        let start = self.layer_exposure(first, layer).0;
        let legacy = self.layer_starts.is_empty();
        let mut all = legacy.then(|| {
            self.cels.range((
                std::ops::Bound::Excluded(first),
                std::ops::Bound::Included(last),
            ))
        });
        let (g, l) = if legacy {
            (&[][..], &[][..])
        } else {
            self.indexed_layer(layer)
        };
        let mut global = visible_after(g, first, last).iter().copied().peekable();
        let mut local = visible_after(l, first, last).iter().copied().peekable();
        std::iter::once(start)
            .take(usize::from(!visible.is_empty()))
            .chain(std::iter::from_fn(move || {
                if let Some(ref mut all) = all {
                    return all.next().map(|(&t, _)| t);
                }
                match (global.peek(), local.peek()) {
                    (Some(a), Some(b)) if a < b => global.next(),
                    (_, Some(_)) => local.next(),
                    _ => global.next(),
                }
            }))
    }

    pub fn layer_frame_is_blank(&self, frame: u32, layer: u64) -> bool {
        !self
            .drawing_at(frame)
            .objects
            .iter()
            .any(|o| o.layer == layer && o.clip_instance.is_none())
    }

    pub(super) fn validate_layer_starts(&self) -> Result<(), String> {
        for (&t, owners) in &self.layer_starts {
            let Some(doc) = self.cels.get(&t) else {
                return Err("Missing composite drawing for layer exposure.".into());
            };
            if t == 0
                || owners
                    .iter()
                    .any(|id| *id != RIG && !doc.layers.iter().any(|l| l.id == *id))
            {
                return Err("Invalid layer drawing boundary.".into());
            }
            let Some((_, previous)) = self.cels.range(..t).next_back() else {
                return Err("Missing initial layer drawing.".into());
            };
            if Arc::ptr_eq(previous, doc)
                || previous
                    .objects
                    .iter()
                    .filter(|o| !owners.contains(&o.layer))
                    .eq(doc.objects.iter().filter(|o| !owners.contains(&o.layer)))
            {
                continue;
            }
            for layer in &doc.layers {
                if !owners.contains(&layer.id)
                    && previous.layers.iter().any(|l| l.id == layer.id)
                    && !doc
                        .objects
                        .iter()
                        .filter(|o| o.layer == layer.id)
                        .eq(previous.objects.iter().filter(|o| o.layer == layer.id))
                {
                    return Err(format!("Layer '{}' has different artwork within the same held drawing at frame {}.", layer.name, t + 1));
                }
            }
        }
        Ok(())
    }

    pub fn duplicate_layer_frame(&mut self, frame: u32, layer: u64) -> Result<u32, String> {
        self.new_layer_frame(frame, layer, false)
    }

    pub(super) fn new_layer_frame(
        &mut self,
        frame: u32,
        layer: u64,
        blank: bool,
    ) -> Result<u32, String> {
        if frame >= self.frames {
            return Err("Invalid playhead.".into());
        }
        self.check_layer_editable(frame, layer)?;
        let (start, _) = self.layer_exposure(frame, layer);
        let at = if start == frame { frame + 1 } else { frame };
        if at < self.frames {
            self.check_layer_editable(at, layer)?;
            if self.cels.contains_key(&at) && self.starts_layer(at, layer) {
                if blank && self.layer_frame_is_blank(at, layer) {
                    return Ok(at);
                }
                return Err(format!(
                    "Frame {} already starts a drawing on this layer.",
                    at + 1
                ));
            }
        }
        if at == self.frames {
            self.set_length(at + 1)?;
        }
        self.split_layer(at, layer);
        if blank {
            let (_, duration) = self.layer_exposure(at, layer);
            for (_, doc) in self.cels.range_mut(at..at + duration) {
                if doc
                    .objects
                    .iter()
                    .any(|o| o.layer == layer && o.clip_instance.is_none())
                {
                    Arc::make_mut(doc)
                        .objects
                        .retain(|o| o.layer != layer || o.clip_instance.is_some());
                }
            }
        }
        Ok(at)
    }

    fn check_layer_editable(&self, at: u32, layer: u64) -> Result<(), String> {
        if !self
            .drawing_at(at)
            .layers
            .iter()
            .any(|l| l.id == layer && l.visible && !l.locked)
        {
            return Err("Choose a visible, unlocked drawing layer.".into());
        }
        Ok(())
    }

    fn split_layer(&mut self, at: u32, layer: u64) {
        self.layer_index.take();
        if !self.cels.contains_key(&at) {
            let doc = self.cels.range(..at).next_back().unwrap().1.clone();
            self.cels.insert(at, doc);
            self.layer_starts.insert(at, BTreeSet::from([layer]));
        } else if let Some(owners) = self.layer_starts.get_mut(&at) {
            owners.insert(layer);
        }
    }

    /// Slide the dividing line between two drawings on one layer. Other lanes,
    /// parent time, events and keys keep their original frame numbers.
    pub fn move_layer_boundary(&mut self, from: u32, to: u32, layer: u64) -> Result<(), String> {
        if from == 0
            || from >= self.frames
            || !self.cels.contains_key(&from)
            || !self.starts_layer(from, layer)
        {
            return Err("Choose a drawing boundary after frame one.".into());
        }
        if from == to {
            return Ok(());
        }
        let previous = self.layer_exposure(from - 1, layer).0;
        let (_, length) = self.layer_exposure(from, layer);
        if to <= previous || to >= from + length {
            return Err("A drawing must remain exposed for at least one frame.".into());
        }
        self.check_layer_editable(previous, layer)?;
        self.check_layer_editable(from, layer)?;
        let mut staged = self.clone();
        staged.split_layer(to, layer);
        let source = self.cels[&if to < from { from } else { previous }].clone();
        for (_, target) in staged.cels.range_mut(from.min(to)..from.max(to)) {
            if !target.layers.iter().any(|l| l.id == layer) {
                return Err("This layer does not exist throughout the requested range.".into());
            }
            if target.skeleton != source.skeleton
                && source
                    .objects
                    .iter()
                    .any(|o| o.layer == layer && (o.rigid_bone.is_some() || o.skin.is_some()))
            {
                return Err("These drawings use different rigs for this layer.".into());
            }
            copy_layer(target, &source, layer);
            fix_cursor(target);
            check_object_ids(target)?;
        }
        // Detach the owner even from a legacy whole-drawing boundary. A boundary can
        // still carry a separate rig, guides or layer configuration when no rows start.
        let ids = self.cels[&from]
            .layers
            .iter()
            .map(|l| l.id)
            .chain([RIG])
            .collect();
        let owners = staged.layer_starts.entry(from).or_insert(ids);
        owners.remove(&layer);
        if owners.is_empty() && staged.drawing_at(from) == staged.drawing_at(from - 1) {
            staged.layer_starts.remove(&from);
            staged.cels.remove(&from);
        }
        if !self.layer_starts.contains_key(&from) {
            // Sliding one old whole-drawing boundary joins two formerly independent
            // ID spaces. Reserve new IDs above both sets before another shape is drawn.
            let cursor = staged.next_shared_object_id()?;
            let (start, end) = bounds(staged.indexed_layer(RIG).0, from);
            let mut shared = HashMap::new();
            for (_, doc) in staged
                .cels
                .range_mut(start.unwrap_or(0)..end.unwrap_or(staged.frames))
            {
                if doc.next_id < cursor {
                    let ptr = Arc::as_ptr(doc);
                    if let Some(existing) = shared.get(&ptr) {
                        *doc = Arc::clone(existing);
                    } else {
                        Arc::make_mut(doc).next_id = cursor;
                        shared.insert(ptr, Arc::clone(doc));
                    }
                }
            }
        }
        *self = staged;
        Ok(())
    }

    pub fn set_layer_exposure(
        &mut self,
        frame: u32,
        layer: u64,
        duration: u32,
    ) -> Result<(), String> {
        if frame >= self.frames || !(1..=crate::MAX_FRAMES).contains(&duration) {
            return Err("Exposure duration or playhead is out of range.".into());
        }
        let (start, old) = self.layer_exposure(frame, layer);
        if old == duration {
            return Ok(());
        }
        self.check_layer_editable(frame, layer)?;
        let end = start + duration;
        if start + old < self.frames {
            self.move_layer_boundary(start + old, end, layer)
        } else if end > self.frames {
            self.set_length(end)
        } else {
            self.new_layer_frame(end, layer, true).map(|_| ())
        }
    }

    /// Commit edits made through a composite drawing back into each changed layer's
    /// held range. Work scales with authored boundaries; playback remains a direct
    /// lookup of a ready-to-render drawing and never assembles a scene per tick.
    pub fn sync_layer_edits(&mut self, before: &Clip, frame: u32) -> Result<(), String> {
        if frame >= self.frames
            || self.session_id != before.session_id
            || self.frames != before.frames
            || self.fps != before.fps
            || self.cels.len() != before.cels.len()
        {
            return Ok(());
        }
        let (at, _) = self.exposure(frame);
        let Some(old) = before.cels.get(&at) else {
            return Ok(());
        };
        let new = self.cels[&at].clone();
        if Arc::ptr_eq(old, &new) || self.layer_starts != before.layer_starts {
            return Ok(());
        }
        if old.layers != new.layers && self.cels.len() > 1 {
            let mut staged = self.clone();
            staged.sync_structure(before, at, old, &new)?;
            staged.sync_held_art(at, old)?;
            *self = staged;
            Ok(())
        } else {
            self.sync_held_art(at, old)
        }
    }

    fn sync_held_art(&mut self, at: u32, old: &Arc<Document>) -> Result<(), String> {
        if self.layer_starts.is_empty() {
            return Ok(());
        }
        let new = self.cels[&at].clone();
        let (family_start, n) = self.rig_exposure(at);
        let family_end = family_start + n;
        let rig = old.skeleton != new.skeleton;
        let guides = old.guides_x != new.guides_x || old.guides_y != new.guides_y;
        let allocated = old.next_id != new.next_id;
        let mut changed = BTreeMap::new();
        for layer in &new.layers {
            let id = layer.id;
            if !old
                .objects
                .iter()
                .filter(|o| o.layer == id)
                .eq(new.objects.iter().filter(|o| o.layer == id))
            {
                let (s, n) = self.layer_exposure(at, id);
                changed.insert(id, s..s + n);
            }
        }
        if changed.is_empty() && !rig && !guides && !allocated {
            return Ok(());
        }

        // A live placement can outlast ordinary artwork on its own layer (F7).
        // Preserve edits to that same placement across its blank/copy boundaries.
        let mut placements = Vec::new();
        for old_obj in old.objects.iter().filter(|o| o.clip_instance.is_some()) {
            let replacement = new.objects.iter().find(|o| o.id == old_obj.id);
            if replacement != Some(old_obj) {
                placements.push((old_obj, replacement));
            }
        }
        let mut range = at..at + 1;
        if rig || guides || !placements.is_empty() {
            range = family_start..family_end;
        }
        for affected in changed.values() {
            range.start = range.start.min(affected.start);
            range.end = range.end.max(affected.end);
        }
        if allocated {
            // Cursors are shared only as far as a layer could hold. A historical
            // boundary that still starts *every* layer keeps its own ID space.
            let (start, end) = bounds(self.indexed_layer(RIG).0, at);
            range = start.unwrap_or(0)..end.unwrap_or(self.frames);
        }
        if self.cels.range(range.clone()).nth(1).is_none() {
            return Ok(());
        }
        let boundaries: BTreeSet<_> = changed
            .values()
            .flat_map(|r| [r.start, r.end])
            .chain([family_start, family_end])
            .collect();
        let mut replacements = HashMap::new();
        let mut cels = self.cels.clone();
        for (&tick, doc) in cels.range_mut(range) {
            if boundaries.contains(&tick) {
                // Within one span, shared composite drawings receive the same edit.
                replacements.clear();
            }
            if tick == at {
                continue;
            }
            let ptr = Arc::as_ptr(doc);
            if let Some(ready) = replacements.get(&ptr) {
                *doc = Arc::clone(ready);
                continue;
            }
            let in_family = (family_start..family_end).contains(&tick);
            if in_family
                && old.layers == new.layers
                && Arc::ptr_eq(doc, old)
                && changed.values().all(|r| r.contains(&tick))
            {
                *doc = Arc::clone(&new);
                replacements.insert(ptr, Arc::clone(doc));
                continue;
            }
            let update_rig = in_family && rig && doc.skeleton == old.skeleton;
            if update_rig {
                Arc::make_mut(doc).skeleton.clone_from(&new.skeleton);
            }
            if in_family && guides && doc.guides_x == old.guides_x && doc.guides_y == old.guides_y {
                let doc = Arc::make_mut(doc);
                doc.guides_x.clone_from(&new.guides_x);
                doc.guides_y.clone_from(&new.guides_y);
            }
            if allocated && doc.next_id < new.next_id {
                Arc::make_mut(doc).next_id = new.next_id;
            }
            for (&layer, range) in &changed {
                if range.contains(&tick) && doc.layers.iter().any(|l| l.id == layer) {
                    if doc.skeleton != new.skeleton
                        && new.objects.iter().any(|o| {
                            o.layer == layer && (o.rigid_bone.is_some() || o.skin.is_some())
                        })
                    {
                        return Err("This held layer crosses drawings with different rigs. Start a layer drawing at the rig change before binding its artwork.".into());
                    }
                    copy_layer(doc, &new, layer);
                }
            }
            for &(old_obj, replacement) in &placements {
                if !in_family {
                    break;
                }
                if changed
                    .get(&old_obj.layer)
                    .is_some_and(|r| r.contains(&tick))
                {
                    continue;
                }
                if let Some(index) = doc
                    .objects
                    .iter()
                    .position(|o| o.id == old_obj.id && o == old_obj)
                {
                    if let Some(o) = replacement {
                        if doc.layers.iter().any(|l| l.id == o.layer) {
                            Arc::make_mut(doc).objects[index].clone_from(o);
                        }
                    } else {
                        Arc::make_mut(doc).objects.remove(index);
                    }
                }
            }
            if update_rig {
                Arc::make_mut(doc).refresh_rig()?;
            }
            if !Arc::ptr_eq(doc, &self.cels[&tick]) {
                fix_cursor(doc);
                check_object_ids(doc)?;
            }
            replacements.insert(ptr, Arc::clone(doc));
        }
        self.cels = cels;
        Ok(())
    }
}

fn copy_layer(target: &mut Arc<Document>, source: &Document, layer: u64) {
    let objects = || source.objects.iter().filter(|o| o.layer == layer);
    if objects().eq(target.objects.iter().filter(|o| o.layer == layer)) {
        return;
    }
    let target = Arc::make_mut(target);
    let mut incoming = objects();
    target.objects.retain_mut(|o| {
        if o.layer != layer {
            return true;
        }
        if let Some(new) = incoming.next() {
            o.clone_from(new);
            true
        } else {
            false
        }
    });
    target.objects.extend(incoming.cloned());
}

fn check_object_ids(doc: &Document) -> Result<(), String> {
    let mut seen = HashSet::with_capacity(doc.objects.len());
    if doc
        .objects
        .iter()
        .any(|o| o.id == doc.next_id || !seen.insert(o.id))
    {
        return Err("These independently authored drawings use conflicting object IDs.".into());
    }
    Ok(())
}

fn fix_cursor(doc: &mut Arc<Document>) {
    if doc.objects.iter().any(|o| o.id == doc.next_id) {
        let mut cursor = doc.next_id;
        while doc.objects.iter().any(|o| o.id == cursor) {
            cursor = cursor.wrapping_add(1).max(1);
        }
        Arc::make_mut(doc).next_id = cursor;
    }
}
