use super::{Clip, Document, RIG};
use crate::Layer;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

impl Clip {
    /// Reserve a cursor for an operation that creates a shared layer and its first
    /// object together, before the layer has a chance to unify legacy ID cursors.
    pub fn next_shared_object_id(&self) -> Result<u64, String> {
        let mut cursor = 1;
        let mut seen = HashSet::new();
        for doc in self.cels.values() {
            if !seen.insert(Arc::as_ptr(doc)) {
                continue;
            }
            cursor = cursor.max(doc.next_id);
            for object in &doc.objects {
                cursor = cursor.max(
                    object
                        .id
                        .checked_add(1)
                        .ok_or("Cannot allocate another shared object ID.")?,
                );
            }
        }
        Ok(cursor)
    }

    /// A new layer can span historical drawings with independent layer stacks.
    pub fn next_layer_id(&self) -> u64 {
        let used: HashSet<_> = self
            .cels
            .values()
            .flat_map(|doc| doc.layers.iter().map(|l| l.id))
            .collect();
        let mut next = used
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .wrapping_add(1)
            .max(1);
        while used.contains(&next) {
            next = next.wrapping_add(1).max(1);
        }
        next
    }

    pub(super) fn sync_structure(
        &mut self,
        before: &Clip,
        at: u32,
        old: &Document,
        new: &Document,
    ) -> Result<(), String> {
        let added: Vec<_> = new
            .layers
            .iter()
            .filter(|l| !old.layers.iter().any(|o| o.id == l.id))
            .collect();
        let mut reused = HashMap::new();
        let cursor = if added.is_empty() {
            0
        } else {
            self.next_shared_object_id()?
        };
        for (&tick, doc) in &mut self.cels {
            let source = before
                .cels
                .get(&tick)
                .ok_or("Drawing boundaries changed during a layer edit.")?;
            if !added.is_empty() && tick != 0 && !self.layer_starts.contains_key(&tick) {
                self.layer_starts.insert(
                    tick,
                    source.layers.iter().map(|l| l.id).chain([RIG]).collect(),
                );
            }
            if tick != at {
                let ptr = Arc::as_ptr(doc);
                if let Some(ready) = reused.get(&ptr) {
                    *doc = Arc::clone(ready);
                } else {
                    let layers = merge_layers(old, new, &doc.layers, &source.layers, &added)?;
                    if layers != doc.layers {
                        let target = Arc::make_mut(doc);
                        target
                            .objects
                            .retain(|o| layers.iter().any(|l| l.id == o.layer));
                        target.layers = layers;
                    }
                    if cursor > doc.next_id {
                        Arc::make_mut(doc).next_id = cursor;
                    }
                    reused.insert(ptr, Arc::clone(doc));
                }
            } else if cursor > doc.next_id {
                Arc::make_mut(doc).next_id = cursor;
            }
            if let Some(owners) = self.layer_starts.get_mut(&tick) {
                owners.retain(|id| *id == RIG || doc.layers.iter().any(|l| l.id == *id));
            }
        }
        self.layer_index.take();
        Ok(())
    }
}

fn merge_layers(
    old: &Document,
    new: &Document,
    layers: &[Layer],
    before_layers: &[Layer],
    added: &[&Layer],
) -> Result<Vec<Layer>, String> {
    if layers == old.layers {
        return Ok(new.layers.clone());
    }
    let mut out = layers.to_vec();
    // Legacy drawings can reuse a local numeric ID for an unrelated named layer.
    let compatible: BTreeSet<_> = old
        .layers
        .iter()
        .filter(|l| layers.iter().any(|o| o.id == l.id && o.name == l.name))
        .map(|l| l.id)
        .collect();
    out.retain_mut(|target| {
        if !compatible.contains(&target.id) {
            return true;
        }
        let Some(layer) = new.layers.iter().find(|l| l.id == target.id) else {
            return false;
        };
        let previous = old.layers.iter().find(|l| l.id == target.id).unwrap();
        if previous.name != layer.name {
            target.name.clone_from(&layer.name);
        }
        if previous.visible != layer.visible {
            target.visible = layer.visible;
        }
        if previous.locked != layer.locked {
            target.locked = layer.locked;
        }
        if previous.blend_mode != layer.blend_mode {
            target.blend_mode = layer.blend_mode;
        }
        true
    });
    for &layer in added {
        if let Some(already) = out.iter().find(|l| l.id == layer.id) {
            if already == layer && !before_layers.iter().any(|l| l.id == layer.id) {
                continue;
            }
            return Err("This layer ID already belongs to another drawing in the clip.".into());
        }
        let at = new.layers.iter().position(|l| l.id == layer.id).unwrap();
        let next = new.layers[at + 1..]
            .iter()
            .find_map(|l| out.iter().position(|o| o.id == l.id));
        out.insert(next.unwrap_or(out.len()), layer.clone());
    }
    if out.is_empty() {
        return Err("Removing this layer would leave another drawing without any layers.".into());
    }
    let before_order: Vec<_> = old
        .layers
        .iter()
        .filter(|l| compatible.contains(&l.id) && new.layers.iter().any(|n| n.id == l.id))
        .map(|l| l.id)
        .collect();
    let after_order: Vec<_> = new
        .layers
        .iter()
        .filter(|l| compatible.contains(&l.id))
        .map(|l| l.id)
        .collect();
    if before_order != after_order {
        let reordered: Vec<_> = after_order
            .iter()
            .filter_map(|id| out.iter().find(|l| l.id == *id).cloned())
            .collect();
        for (target, layer) in out
            .iter_mut()
            .filter(|l| compatible.contains(&l.id))
            .zip(reordered)
        {
            *target = layer;
        }
    }
    Ok(out)
}
