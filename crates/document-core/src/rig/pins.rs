//! Two-bone contacts hold a target in the drawing's artboard coordinates.
//! Time is sparse: moving a parent after planting updates every held pose.
use super::{
    ik::{near_angle, Limb},
    Bone, RigidPose, Skeleton,
};
use crate::Document;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FootPin {
    pub target: [f32; 2],
    pub bend_negative: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sole: Option<PlantedSole>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantedSole {
    pub bone: u64,
    pub world_angle: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PinJoint {
    Limb {
        limb: Limb,
        lower: bool,
        target: [f32; 2],
        negative: bool,
    },
    Sole(f32),
}

impl PinJoint {
    pub(super) fn rotation(self, parent: RigidPose, authored: f32) -> f32 {
        match self {
            Self::Limb {
                limb,
                lower,
                target,
                negative,
            } => {
                let target = parent.inverse().point(target.map(f64::from));
                // For the lower joint only the distance to the shoulder is
                // needed. The parent's rotation leaves that distance unchanged.
                limb.angles(target, negative, authored, authored)[usize::from(lower)]
            }
            Self::Sole(world_angle) => near_angle(f64::from(world_angle) - parent.angle, authored),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ContactSpan {
    pub(super) end: Option<u32>,
    pub(super) constraint: PinJoint,
}

const OVERLAP: &str = "These foot pins would control the same joint at the same time.";

impl Skeleton {
    fn pin_chain(&self, tip: u64) -> Result<(&Bone, &Bone, Limb), String> {
        let lower = self
            .bones
            .iter()
            .find(|b| b.id == tip)
            .ok_or("The foot pin references a missing bone.")?;
        let upper = lower
            .parent
            .and_then(|id| self.bones.iter().find(|b| b.id == id))
            .ok_or("Select a child bone to plant the foot.")?;
        Ok((upper, lower, Limb::new(upper, lower)?))
    }

    fn constrained_joints(
        &self,
        tip: u64,
        pin: FootPin,
    ) -> Result<impl Iterator<Item = (u64, PinJoint)>, String> {
        let (upper, lower, limb) = self.pin_chain(tip)?;
        if !pin.target.iter().all(|v| v.is_finite()) {
            return Err("A planted foot needs a finite target.".into());
        }
        let joint = |lower| PinJoint::Limb {
            limb,
            lower,
            target: pin.target,
            negative: pin.bend_negative,
        };
        let sole = if let Some(sole) = pin.sole {
            if !sole.world_angle.is_finite()
                || !self.bones.iter().any(|bone| {
                    bone.id == sole.bone && bone.parent == Some(tip) && bone.head == lower.tail
                })
            {
                return Err(
                    "The planted sole must have a finite angle and attach at the ankle.".into(),
                );
            }
            Some((sole.bone, PinJoint::Sole(sole.world_angle)))
        } else {
            None
        };
        Ok([(upper.id, joint(false)), (lower.id, joint(true))]
            .into_iter()
            .chain(sole))
    }

    pub(super) fn active_constraints(&self, frame: u32) -> Result<BTreeMap<u64, PinJoint>, String> {
        let mut result = BTreeMap::new();
        for (&tip, track) in &self.foot_pins {
            if let Some((_, Some(pin))) = track.range(..=frame).next_back() {
                for (id, joint) in self.constrained_joints(tip, *pin)? {
                    if result.insert(id, joint).is_some() {
                        return Err(OVERLAP.into());
                    }
                }
            }
        }
        Ok(result)
    }

    pub(super) fn contact_tracks(
        &self,
        frames: u32,
    ) -> Result<BTreeMap<u64, BTreeMap<u32, ContactSpan>>, String> {
        let mut joints: BTreeMap<u64, BTreeMap<u32, ContactSpan>> = BTreeMap::new();
        for (&tip, track) in &self.foot_pins {
            self.pin_chain(tip)?;
            if track.is_empty() || track.last_key_value().is_some_and(|(&f, _)| f >= frames) {
                return Err("Foot pin events need a frame inside the drawing.".into());
            }
            let mut events = track.iter().peekable();
            while let Some((&start, pin)) = events.next() {
                let Some(pin) = pin else {
                    continue;
                };
                let end = events.peek().map(|(&tick, _)| tick);
                for (id, constraint) in self.constrained_joints(tip, *pin)? {
                    let spans = joints.entry(id).or_default();
                    let overlaps_prior = spans
                        .range(..=start)
                        .next_back()
                        .is_some_and(|(_, prior)| prior.end.is_none_or(|stop| stop > start));
                    let overlaps_next = spans
                        .range(start..)
                        .next()
                        .is_some_and(|(&next, _)| end.is_none_or(|stop| next < stop));
                    if overlaps_prior || overlaps_next {
                        return Err(OVERLAP.into());
                    }
                    spans.insert(start, ContactSpan { end, constraint });
                }
            }
        }
        Ok(joints)
    }
}

impl Document {
    /// Artboard target of the tip's active contact, if there is one on this tick.
    pub fn foot_pin_at(&self, tip: u64, frame: u32) -> Option<[f32; 2]> {
        self.skeleton
            .foot_pins
            .get(&tip)?
            .range(..=frame)
            .next_back()?
            .1
            .map(|p| p.target)
    }

    /// Capture the current ankle position. A single sole attached at the tip
    /// also retains its artboard angle through this contact.
    pub fn pin_foot(&mut self, tip: u64, frame: u32) -> Result<(), String> {
        if frame >= self.frames {
            return Err("Choose a frame inside the drawing.".into());
        }
        if self.foot_pin_at(tip, frame).is_some() {
            return Err("This foot is already pinned on this frame.".into());
        }
        let (_, lower, limb) = self.skeleton.pin_chain(tip)?;
        let poses = self.skeleton.poses_at(frame)?;
        let target = poses[&tip]
            .point(lower.tail.map(f64::from))
            .map(|v| v as f32);
        let mut attached = self
            .skeleton
            .bones
            .iter()
            .filter(|b| b.parent == Some(tip) && b.head == lower.tail);
        let sole = attached
            .next()
            .filter(|_| attached.next().is_none())
            .map(|bone| PlantedSole {
                bone: bone.id,
                world_angle: poses[&bone.id].angle as f32,
            });
        let pin = FootPin {
            target,
            bend_negative: limb.bend_negative(lower.rotation_at(frame)),
            sole,
        };
        let mut skeleton = self.skeleton.clone();
        skeleton
            .foot_pins
            .entry(tip)
            .or_default()
            .insert(frame, Some(pin));
        self.replace_rig(skeleton)
    }

    /// First free frame. Key the solved joints here so lifting the constraint
    /// doesn't snap the foot back to its previously authored pose.
    pub fn release_foot_pin(&mut self, tip: u64, frame: u32) -> Result<(), String> {
        if frame >= self.frames {
            return Err("Choose a frame inside the drawing.".into());
        }
        let Some(pin) = self
            .skeleton
            .foot_pins
            .get(&tip)
            .and_then(|track| track.range(..=frame).next_back())
            .and_then(|(_, pin)| *pin)
        else {
            return Err("This foot is not pinned on this frame.".into());
        };
        let rotations = self.skeleton.posed_rotations_at(frame)?;
        let joint_ids: Vec<_> = self
            .skeleton
            .constrained_joints(tip, pin)?
            .map(|(id, _)| id)
            .collect();
        let mut skeleton = self.skeleton.clone();
        let track = skeleton.foot_pins.get_mut(&tip).unwrap();
        track.insert(frame, None);
        // A pin and immediate release, or a release before an existing one,
        // can leave consecutive inactive markers. Keep the track sparse.
        let mut active = false;
        track.retain(|_, state| {
            let previous = active;
            active = state.is_some();
            active || previous
        });
        if track.is_empty() {
            skeleton.foot_pins.remove(&tip);
        }
        // A guard sample preserves interpolation throughout all earlier ticks,
        // including unpinned history and a fully folded contact's shoulder pose.
        for b in skeleton
            .bones
            .iter_mut()
            .filter(|b| joint_ids.contains(&b.id))
        {
            if (b.rotation_at(frame) - rotations[&b.id]).abs() <= f32::EPSILON {
                continue;
            }
            if frame > 0 {
                let guard = b.rotation_at(frame - 1);
                b.rotation_keys.entry(frame - 1).or_insert(guard);
            }
            b.rotation_keys.insert(frame, rotations[&b.id]);
        }
        self.replace_rig(skeleton)
    }

    pub(super) fn replace_rig(&mut self, skeleton: Skeleton) -> Result<(), String> {
        let original = std::mem::replace(&mut self.skeleton, skeleton);
        if let Err(error) = self.refresh_rig() {
            self.skeleton = original;
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg() -> (Document, [u64; 4]) {
        let mut doc = Document {
            frames: 24,
            ..Default::default()
        };
        let hip = doc.add_bone("Body", None, [0., 0.], [3., 4.]).unwrap();
        let thigh = doc
            .add_bone("Thigh", Some(hip), [3., 4.], [13., 4.])
            .unwrap();
        let shin = doc
            .add_bone("Shin", Some(thigh), [13., 4.], [23., 4.])
            .unwrap();
        let sole = doc
            .add_bone("Foot", Some(shin), [23., 4.], [25., 5.])
            .unwrap();
        for (id, a, b) in [
            (hip, 0., 17.),
            (thigh, -25., 5.),
            (shin, 86., 100.),
            (sole, -17., 25.),
        ] {
            doc.set_bone_rotation_key(id, 0, a).unwrap();
            doc.set_bone_rotation_key(id, 20, b).unwrap();
        }
        (doc, [hip, thigh, shin, sole])
    }
    fn near(a: [f32; 2], b: [f32; 2]) {
        assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 0.0001, "{a:?} != {b:?}");
    }

    #[test]
    fn pins_hold_both_ends_through_later_ancestor_edits_and_release_without_changing_history() {
        let (mut doc, [hip, _, shin, sole]) = leg();
        let foot = doc.add_rectangle([22., 4.], [4., 2.], [220; 4]);
        doc.bind_rigid(foot, Some(sole)).unwrap();
        let ankle = doc.bone_segment_at(shin, true, 4).unwrap().1;
        let toe = doc.bone_segment_at(sole, true, 4).unwrap().1;
        let ink = doc.objects[0].rig_point_at([24., 5.], 4);
        doc.pin_foot(shin, 4).unwrap();
        doc.set_bone_rotation_key(hip, 14, 24.).unwrap();
        let before: Vec<_> = (0..=14)
            .map(|f| doc.bone_segments_at(true, f).unwrap())
            .collect();
        for f in 4..=14 {
            near(doc.bone_segment_at(shin, true, f).unwrap().1, ankle);
            near(doc.bone_segment_at(sole, true, f).unwrap().1, toe);
            near(doc.objects[0].rig_point_at([24., 5.], f), ink);
        }
        doc.release_foot_pin(shin, 14).unwrap();
        assert_eq!(doc.foot_pin_at(shin, 13), Some(ankle));
        assert_eq!(doc.foot_pin_at(shin, 14), None);
        for (f, segments) in before.iter().enumerate() {
            for (old, new) in segments
                .iter()
                .zip(doc.bone_segments_at(true, f as u32).unwrap())
            {
                near(old.head, new.head);
                near(old.tail, new.tail);
            }
        }
        assert_ne!(doc.bone_segment_at(shin, true, 20).unwrap().1, ankle);
        doc.validate().unwrap();
    }

    #[test]
    fn another_independent_foot_can_plant_but_overlapping_chains_fail_atomically() {
        let (mut doc, [hip, _, shin, sole]) = leg();
        let left = doc
            .add_bone("Other thigh", Some(hip), [2., 5.], [0., 15.])
            .unwrap();
        let ankle = doc
            .add_bone("Other shin", Some(left), [0., 15.], [2., 24.])
            .unwrap();
        doc.pin_foot(shin, 6).unwrap();
        doc.pin_foot(ankle, 1).unwrap();
        let accepted = doc.clone();
        // Other foot is independent. A contact on the articulated sole would
        // also take over the already pinned shin a few frames in the future.
        assert!(doc.pin_foot(sole, 2).is_err());
        assert!(doc.pose_bone_ik(sole, 7, [12., 17.], false).is_err());
        assert_eq!(doc, accepted);
        doc.release_foot_pin(ankle, 1).unwrap();
        assert!(!doc.skeleton.foot_pins.contains_key(&ankle));
        assert!(doc.foot_pin_at(shin, 9).is_some());
    }

    #[test]
    fn branched_tip_holds_the_ankle_without_choosing_a_sole() {
        let (mut doc, [_, _, shin, _]) = leg();
        doc.add_bone("Toe B", Some(shin), [23., 4.], [25., 3.])
            .unwrap();
        doc.pin_foot(shin, 3).unwrap();
        let pin = doc.skeleton.foot_pins[&shin][&3].unwrap();
        assert_eq!(pin.sole, None);
        near(doc.bone_segment_at(shin, true, 12).unwrap().1, pin.target);
        let mut invalid = doc.clone();
        invalid
            .skeleton
            .foot_pins
            .get_mut(&shin)
            .unwrap()
            .get_mut(&3)
            .unwrap()
            .as_mut()
            .unwrap()
            .target[1] = f32::NAN;
        assert!(invalid.refresh_rig().is_err());
    }
}
