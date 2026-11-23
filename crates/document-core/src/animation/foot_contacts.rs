use super::Clip;
use crate::{Bone, Skeleton};
use std::collections::HashMap;
use std::sync::Arc;

impl Clip {
    /// An effects drawing can split an exposure while keeping the same character
    /// rig underneath it. Carry the contact forward through those drawings.
    pub fn set_foot_contact(&mut self, tip: u64, frame: u32, planted: bool) -> Result<(), String> {
        if frame >= self.frames {
            return Err("Choose a frame inside the clip.".into());
        }
        let (start, _) = self.exposure(frame);
        let original = &self.cels[&start];
        let mut edited = original.as_ref().clone();
        if planted {
            edited.pin_foot(tip, frame)?;
        } else {
            edited.release_foot_pin(tip, frame)?;
        }
        let edited = Arc::new(edited);
        let bones: HashMap<_, _> = original.skeleton.bones.iter().map(|b| (b.id, b)).collect();
        let mut shared = HashMap::from([(Arc::as_ptr(original), Arc::clone(&edited))]);
        let mut replacements = vec![(start, Arc::clone(&edited))];
        for (&tick, drawing) in self.cels.range(start + 1..) {
            if let Some(existing) = shared.get(&Arc::as_ptr(drawing)) {
                replacements.push((tick, Arc::clone(existing)));
                continue;
            }
            // Independent drawings can reuse bone IDs. A different rig or an
            // independently authored contact for this foot is a boundary.
            if drawing.skeleton.foot_pins.get(&tip) != original.skeleton.foot_pins.get(&tip)
                || !same_rig(&bones, &drawing.skeleton)
            {
                break;
            }
            let mut copy = drawing.as_ref().clone();
            if copy.skeleton == original.skeleton {
                // Preserve the keys that ease out of contact across held rig poses.
                copy.skeleton = edited.skeleton.clone();
            } else if let Some(track) = edited.skeleton.foot_pins.get(&tip) {
                copy.skeleton.foot_pins.insert(tip, track.clone());
            } else {
                copy.skeleton.foot_pins.remove(&tip);
            }
            copy.refresh_rig()
                .map_err(|e| format!("Could not update the foot pin on frame {}. {e}", tick + 1))?;
            let copy = Arc::new(copy);
            shared.insert(Arc::as_ptr(drawing), Arc::clone(&copy));
            replacements.push((tick, copy));
        }
        self.cels.extend(replacements);
        Ok(())
    }
}

fn same_rig(source: &HashMap<u64, &Bone>, next: &Skeleton) -> bool {
    source.len() == next.bones.len()
        && next.bones.iter().all(|b| {
            source.get(&b.id).is_some_and(|old| {
                old.name == b.name
                    && old.parent == b.parent
                    && old.head == b.head
                    && old.tail == b.tail
            })
        })
}

#[cfg(test)]
mod tests {
    use crate::{AnimationProject, Document};

    #[test]
    fn planting_and_releasing_carry_through_effects_drawings_and_stop_at_a_different_rig() {
        let mut doc = Document {
            frames: 12,
            ..Document::default()
        };
        let thigh = doc.add_bone("Thigh", None, [30., 32.], [45., 50.]).unwrap();
        let shin = doc
            .add_bone("Shin", Some(thigh), [45., 50.], [32., 74.])
            .unwrap();
        let mut project = AnimationProject::from_document(doc);
        let clip = &mut project.clips[0];
        clip.blank_layer_frame(3, 1).unwrap();
        clip.blank_layer_frame(6, 1).unwrap();
        clip.blank_layer_frame(9, 1).unwrap();
        clip.drawing_at_mut(3)
            .add_rectangle([5., 5.], [8., 7.], [255; 4]);
        clip.drawing_at_mut(6)
            .set_bone_rotation_key(thigh, 6, 22.)
            .unwrap();
        clip.drawing_at_mut(9).skeleton.bones[0].name = "Unrelated limb".into();
        clip.set_foot_contact(shin, 1, true).unwrap();
        let target = clip.drawing_at(1).foot_pin_at(shin, 1).unwrap();
        for t in 1..9 {
            let scene = clip.drawing_at(t);
            assert_eq!(scene.foot_pin_at(shin, t), Some(target));
            let tip = scene.bone_segment_at(shin, true, t).unwrap().1;
            assert!((tip[0] - target[0]).hypot(tip[1] - target[1]) < 0.001);
        }
        assert_eq!(clip.drawing_at(9).foot_pin_at(shin, 9), None);
        clip.set_foot_contact(shin, 4, false).unwrap();
        assert_eq!(clip.drawing_at(3).foot_pin_at(shin, 3), Some(target));
        for t in 4..12 {
            assert_eq!(clip.drawing_at(t).foot_pin_at(shin, t), None);
        }
        assert!(clip.drawing_at(6).skeleton.bones[0]
            .rotation_keys
            .contains_key(&6));
        assert_eq!(clip.drawing_at(3).objects.len(), 1);
        clip.validate().unwrap();
    }
}
