use super::{layer_exposures::RIG, retime, update_shared_drawings, Clip, Edit};
use std::collections::{btree_map::Entry, BTreeMap, HashSet};
use std::sync::Arc;

/// Original half-open interval and the tick at which its two sides meet after removal.
pub(super) struct Cut {
    pub start: u32,
    pub end: u32,
    pub dest: u32,
}

pub(super) struct FrameRemoval {
    pub cuts: Vec<Cut>,
    remaining: u32,
}

impl FrameRemoval {
    fn new(frames: u32, ranges: &[(u32, u32)]) -> Result<Self, String> {
        let mut cuts = Vec::<Cut>::with_capacity(ranges.len());
        let mut removed = 0;
        for &(start, last) in ranges {
            if last >= frames || start > last || cuts.last().is_some_and(|cut| start < cut.end) {
                return Err("Choose frame ranges in order without overlap.".into());
            }
            let end = last + 1;
            if let Some(prior) = cuts.last_mut().filter(|cut| cut.end == start) {
                prior.end = end;
            } else {
                cuts.push(Cut {
                    start,
                    end,
                    dest: start - removed,
                });
            }
            removed += end - start;
        }
        if removed == 0 {
            return Err("Select at least one frame.".into());
        }
        if removed == frames {
            return Err("Keep at least one frame in the clip.".into());
        }
        Ok(Self {
            cuts,
            remaining: frames - removed,
        })
    }

    /// Map a tick to its new position. Boundaries inside a cut join the next surviving tick.
    pub(super) fn map(&self, tick: u32) -> (u32, bool) {
        let before = self.cuts.partition_point(|cut| cut.start <= tick);
        match self.cuts[..before].last() {
            None => (tick, true),
            Some(cut) => (cut.dest + tick.saturating_sub(cut.end), tick >= cut.end),
        }
    }
}

impl Clip {
    /// Delete sorted, disjoint inclusive ranges in the original timeline as one edit.
    pub fn delete_frame_ranges(&mut self, ranges: &[(u32, u32)]) -> Result<u32, String> {
        let removal = FrameRemoval::new(self.frames, ranges)?;
        let mut cels = BTreeMap::new();
        let mut starts = BTreeMap::new();
        for (&old, doc) in &self.cels {
            let (dest, _) = removal.map(old);
            if dest == removal.remaining {
                break;
            }
            let owners = self.layer_starts.get(&old);
            // Every boundary cut away before this tick contributes its layers. The last
            // drawing supplies the surviving pose, and a whole-drawing cut covers all layers.
            match cels.entry(dest) {
                Entry::Vacant(entry) => {
                    entry.insert(Arc::clone(doc));
                    if let Some(owners) = owners {
                        starts.insert(dest, owners.clone());
                    }
                }
                Entry::Occupied(mut entry) => {
                    entry.insert(Arc::clone(doc));
                    if let Some(owners) = owners {
                        if let Some(joined) = starts.get_mut(&dest) {
                            joined.extend(owners);
                        }
                    } else {
                        starts.remove(&dest);
                    }
                }
            }
        }

        // A legacy drawing can have a different layer stack. A layer that exists
        // only in the removed interval cannot own a boundary in the surviving cel.
        for cut in &removal.cuts {
            if let Some(owners) = starts.get_mut(&cut.dest) {
                let layers: HashSet<_> = cels[&cut.dest].layers.iter().map(|l| l.id).collect();
                owners.retain(|id| *id == RIG || layers.contains(id));
            }
        }

        // Work off the original poses once, even when many separated frames are removed
        // or several cels share a drawing. Publish only after all tracks validate.
        update_shared_drawings(cels.values_mut(), |doc| {
            retime(doc, Edit::Remove(&removal), removal.remaining)
        })?;
        self.events = std::mem::take(&mut self.events)
            .into_iter()
            .filter_map(|(tick, names)| {
                let (dest, kept) = removal.map(tick);
                kept.then_some((dest, names))
            })
            .collect();
        self.layer_index.take();
        self.layer_starts = starts;
        self.cels = cels;
        self.frames = removal.remaining;
        Ok(removal.cuts[0].dest.min(self.frames - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnimationProject, Easing, Layer, Property};

    #[test]
    fn removing_disjoint_frames_preserves_eased_poses_and_merged_layer_boundaries() {
        let mut original = AnimationProject::default().clips.remove(0);
        original.set_length(12).unwrap();
        let doc = original.drawing_at_mut(0);
        doc.layers.push(Layer {
            id: 2,
            name: "Effects".into(),
            ..Default::default()
        });
        doc.add_rectangle([0.; 2], [8.; 2], [255; 4]);
        let obj = &mut doc.objects[0];
        obj.keys.extend([(0, [0.; 2]), (11, [80., 40.])]);
        Property::Position.set_easing(obj, 0, Easing::EaseInOut);
        for (tick, layer) in [(2, 1), (3, 2), (8, 2)] {
            original.duplicate_layer_frame(tick, layer).unwrap();
        }
        original.add_event(4, "kept").unwrap();
        original.add_event(6, "discarded").unwrap();
        let mut edited = original.clone();
        let kept = [1, 4, 5, 7, 8, 10];
        edited
            .delete_frame_ranges(&[(0, 0), (2, 3), (6, 6), (9, 9), (11, 11)])
            .unwrap();
        assert_eq!(edited.frames as usize, kept.len());
        assert_eq!(edited.events, [(1, vec!["kept".to_owned()])].into());
        for (new, old) in kept.into_iter().enumerate() {
            let new = new as u32;
            let expected = original.drawing_at(old).objects[0].position_at(old);
            let actual = edited.drawing_at(new).objects[0].position_at(new);
            for (a, b) in actual.into_iter().zip(expected) {
                assert!((a - b).abs() < 0.001, "frame {old}: {a} versus {b}");
            }
        }
        assert_eq!(edited.layer_starts[&1], [1, 2].into());
        assert_eq!(edited.layer_starts[&4], [2].into());
        edited.validate().unwrap();
    }

    #[test]
    fn invalid_frame_sets_are_atomic_and_shared_drawings_are_retimed_once() {
        let mut clip = AnimationProject::default().clips.remove(0);
        clip.set_length(1000).unwrap();
        let shared = Arc::clone(&clip.cels[&0]);
        for tick in 1..clip.frames {
            clip.cels.insert(tick, Arc::clone(&shared));
        }
        let original = clip.clone();
        for ranges in [
            &[][..],
            &[(1, 3), (3, 5)],
            &[(4, 6), (1, 2)],
            &[(999, 1000)],
            &[(0, 998), (999, 999)],
        ] {
            assert!(clip.delete_frame_ranges(ranges).is_err());
            assert_eq!(clip, original);
        }
        let alternating: Vec<_> = (0..1000).step_by(2).map(|t| (t, t)).collect();
        clip.delete_frame_ranges(&alternating).unwrap();
        assert_eq!(clip.cels.len(), 500);
        assert!(clip
            .cels
            .values()
            .all(|drawing| Arc::ptr_eq(drawing, &clip.cels[&0])));
        assert_eq!(clip.cels[&0].frames, 500);
        assert_eq!(shared.frames, 1000);
        clip.validate().unwrap();
    }

    #[test]
    fn removed_drawings_cannot_leave_a_boundary_for_a_missing_layer() {
        let mut clip = AnimationProject::default().clips.remove(0);
        let mut extra_layer = (*clip.cels[&0]).clone();
        extra_layer.layers.push(Layer {
            id: 2,
            name: "Effects".into(),
            ..Default::default()
        });
        clip.cels.insert(2, Arc::new(extra_layer));
        clip.cels.insert(3, Arc::clone(&clip.cels[&0]));
        clip.layer_starts.insert(2, [RIG, 1, 2].into());
        clip.layer_starts.insert(3, [RIG, 1].into());
        clip.validate().unwrap();
        clip.delete_frame_ranges(&[(0, 0), (2, 2)]).unwrap();
        assert_eq!(clip.layer_starts[&1], [RIG, 1].into());
        clip.validate().unwrap();
    }
}
