use super::{Bone, RigidPose};
use crate::Document;
use std::f64::consts::PI;

pub(super) struct Reach {
    pub(super) joints: [usize; 2],
    pub(super) angles: [f32; 2],
    parent_space: RigidPose,
}

impl Document {
    /// Key a two-bone chain so its tip reaches an artboard point at this tick.
    /// Out-of-reach targets stop at full extension or full flexion. The current
    /// elbow side is preserved unless the artist asks for the other solution.
    pub fn pose_bone_ik(
        &mut self,
        tip_id: u64,
        frame: u32,
        target: [f32; 2],
        flip_bend: bool,
    ) -> Result<(), String> {
        let reach = self.solve_bone_ik(tip_id, frame, target, flip_bend)?;

        // Stage the pair together. Validating an intermediate one-joint pose
        // would allow a failed solve to leave half a pose or reject a valid pair.
        let mut skeleton = self.skeleton.clone();
        for (index, angle) in reach.joints.into_iter().zip(reach.angles) {
            let bone = &mut skeleton.bones[index];
            if frame > 0 && bone.rotation_keys.is_empty() {
                bone.rotation_keys.insert(0, bone.rotation);
            }
            bone.rotation_keys.insert(frame, angle);
        }
        self.replace_rig(skeleton)
    }

    /// Artboard positions of the shoulder, elbow and tip after a prospective
    /// solve. Previewing changes neither the authored angles nor the pose cache.
    pub fn preview_bone_ik(
        &self,
        tip_id: u64,
        frame: u32,
        target: [f32; 2],
        flip_bend: bool,
    ) -> Result<[[f32; 2]; 3], String> {
        let reach = self.solve_bone_ik(tip_id, frame, target, flip_bend)?;
        let [upper, lower] = reach.joints.map(|index| &self.skeleton.bones[index]);
        let upper_pose = reach
            .parent_space
            .compose(RigidPose::about(upper.head, reach.angles[0]));
        let lower_pose = upper_pose.compose(RigidPose::about(lower.head, reach.angles[1]));
        let points = [
            (upper_pose, upper.head),
            (lower_pose, lower.head),
            (lower_pose, lower.tail),
        ]
        .map(|(pose, point)| pose.point(point.map(f64::from)).map(|x| x as f32));
        if points.iter().flatten().any(|v| !v.is_finite()) {
            return Err("The posed skeleton exceeds artboard coordinates.".into());
        }
        Ok(points)
    }

    pub(super) fn solve_bone_ik(
        &self,
        tip_id: u64,
        frame: u32,
        target: [f32; 2],
        flip_bend: bool,
    ) -> Result<Reach, String> {
        if frame >= self.frames || !target.into_iter().all(f32::is_finite) {
            return Err("Choose a finite IK target on an existing frame.".into());
        }
        let joints = &self.skeleton.bones;
        let index = |id| {
            joints
                .iter()
                .position(|b| b.id == id)
                .ok_or("Choose an existing bone.")
        };
        let lower = index(tip_id)?;
        let upper = index(
            joints[lower]
                .parent
                .ok_or("Select a child bone for two-bone IK.")?,
        )?;
        let pinned = self.skeleton.active_constraints(frame)?;
        if pinned.contains_key(&joints[upper].id) || pinned.contains_key(&tip_id) {
            return Err("Release the foot pin before dragging this limb.".into());
        }
        let poses = self.skeleton.poses_at(frame)?;
        let parent_space = joints[upper]
            .parent
            .map_or_else(RigidPose::default, |id| poses[&id]);
        let local_target = parent_space.inverse().point(target.map(f64::from));
        let limb = Limb::new(&joints[upper], &joints[lower])?;
        let upper_rotation = joints[upper].rotation_at(frame);
        let lower_rotation = joints[lower].rotation_at(frame);
        let negative = limb.bend_negative(lower_rotation) ^ flip_bend;
        let angles = limb.angles(local_target, negative, upper_rotation, lower_rotation);
        Ok(Reach {
            joints: [upper, lower],
            angles,
            parent_space,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Limb {
    shoulder: [f32; 2],
    a: f64,
    b: f64,
    upper_rest: f64,
    rest_bend: f64,
}

fn direction(start: [f32; 2], end: [f64; 2]) -> (f64, f64) {
    let x = end[0] - f64::from(start[0]);
    let y = end[1] - f64::from(start[1]);
    (x.hypot(y), y.atan2(x))
}

impl Limb {
    pub(super) fn new(upper: &Bone, lower: &Bone) -> Result<Self, String> {
        // Offset joints are legal. The first lever is the distance between the
        // actual pivots, even if the parent's visible tail ends somewhere else.
        let (a, upper_rest) = direction(upper.head, lower.head.map(f64::from));
        let (b, lower_rest) = direction(lower.head, lower.tail.map(f64::from));
        if a == 0. || b == 0. {
            return Err(
                "Two-bone IK needs distinct shoulder, elbow, and tip positions along the chain."
                    .into(),
            );
        }
        Ok(Self {
            shoulder: upper.head,
            a,
            b,
            upper_rest,
            rest_bend: lower_rest - upper_rest,
        })
    }

    pub(super) fn bend_negative(self, lower_rotation: f32) -> bool {
        (self.rest_bend + (f64::from(lower_rotation) % 360.).to_radians()).sin() < -1e-12
    }

    pub(super) fn angles(
        self,
        target: [f64; 2],
        negative: bool,
        upper_rotation: f32,
        lower_rotation: f32,
    ) -> [f32; 2] {
        let Self {
            a,
            b,
            shoulder,
            upper_rest,
            rest_bend,
        } = self;
        let (distance, heading) = direction(shoulder, target);
        let magnitude = if distance >= a + b {
            0.
        } else if distance <= (a - b).abs() {
            PI
        } else {
            ((distance * distance - a * a - b * b) / (2. * a * b))
                .clamp(-1., 1.)
                .acos()
        };
        let bend = if negative { -magnitude } else { magnitude };
        let upper_angle = if distance == 0. {
            // The target lies on the shoulder. There is no preferred direction:
            // retain the shoulder's current angle while the elbow folds.
            f64::from(upper_rotation)
        } else {
            let (sin, cos) = bend.sin_cos();
            (heading - (b * sin).atan2(a + b * cos) - upper_rest).to_degrees()
        };
        // Keep the closest authored turn unless its magnitude makes the solved
        // fractional angle unrepresentable in the file's f32 channel.
        [
            near_angle(upper_angle, upper_rotation),
            near_angle((bend - rest_bend).to_degrees(), lower_rotation),
        ]
    }
}

pub(super) fn near_angle(degrees: f64, current: f32) -> f32 {
    let current = f64::from(current);
    let wrap = |v: f64| (v + 180.).rem_euclid(360.) - 180.;
    let desired = wrap(degrees % 360.);
    let closest = (current + wrap(desired - current % 360.)) as f32;
    if wrap(f64::from(closest) % 360. - desired).abs() > 0.001 {
        desired as f32
    } else {
        closest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: [f32; 2], expected: [f32; 2]) {
        assert!(
            (actual[0] - expected[0]).hypot(actual[1] - expected[1]) < 1e-4,
            "{actual:?} vs {expected:?}"
        );
    }

    fn arm(lower_length: f32) -> (Document, u64, u64) {
        let mut doc = Document::default();
        let upper = doc.add_bone("Arm", None, [10., 10.], [20., 10.]).unwrap();
        let lower = doc
            .add_bone("Hand", Some(upper), [20., 10.], [20. + lower_length, 10.])
            .unwrap();
        (doc, upper, lower)
    }

    #[test]
    fn reaches_with_an_animated_ancestor_and_an_offset_joint() {
        let mut doc = Document::default();
        let spine = doc.add_bone("Spine", None, [10., 10.], [20., 10.]).unwrap();
        let arm = doc
            .add_bone("Arm", Some(spine), [20., 10.], [30., 10.])
            .unwrap();
        let hand = doc
            .add_bone("Hand", Some(arm), [28., 16.], [28., 21.])
            .unwrap();
        doc.set_bone_rotation_key(spine, 0, 0.).unwrap();
        doc.set_bone_rotation_key(spine, 4, 120.).unwrap();
        let frame = 3;
        let start = doc.bone_segment_at(hand, true, 0).unwrap();
        let ancestor = doc.skeleton.bones[0].clone();
        let before = doc.clone();
        let predicted = doc.preview_bone_ik(hand, frame, [16., 24.], false).unwrap();
        assert_eq!(doc, before);
        doc.pose_bone_ik(hand, frame, [16., 24.], false).unwrap();
        let (shoulder, _) = doc.bone_segment_at(arm, true, frame).unwrap();
        let (elbow, tip) = doc.bone_segment_at(hand, true, frame).unwrap();
        assert_eq!(predicted, [shoulder, elbow, tip]);
        close(
            doc.bone_segment_at(hand, true, frame).unwrap().1,
            [16., 24.],
        );
        assert_eq!(doc.skeleton.bones[0], ancestor);
        assert_eq!(doc.bone_segment_at(hand, true, 0).unwrap(), start);
        for joint in &doc.skeleton.bones[1..] {
            assert_eq!(joint.rotation, 0.);
            assert_eq!(joint.rotation_keys[&0], 0.);
            assert!(joint.rotation_keys.contains_key(&frame));
        }
        doc.validate().unwrap();
    }

    #[test]
    fn mirrored_solutions_reach_the_same_tip_and_retain_the_elbow_side() {
        let (mut doc, upper, hand) = arm(10.);
        let target = [20., 20.];
        doc.pose_bone_ik(hand, 0, target, false).unwrap();
        let first = doc.bone_segment_at(upper, true, 0).unwrap().1;
        close(first, [20., 10.]);
        close(doc.bone_segment_at(hand, true, 0).unwrap().1, target);
        doc.pose_bone_ik(hand, 0, target, true).unwrap();
        close(doc.bone_segment_at(upper, true, 0).unwrap().1, [10., 20.]);
        close(doc.bone_segment_at(hand, true, 0).unwrap().1, target);
        doc.pose_bone_ik(hand, 0, target, false).unwrap();
        close(doc.bone_segment_at(upper, true, 0).unwrap().1, [10., 20.]);
    }

    #[test]
    fn ik_keeps_authored_turns_while_the_channel_can_represent_the_solution() {
        let (mut doc, upper, hand) = arm(10.);
        doc.set_bone_rotation(upper, 359.).unwrap();
        doc.pose_bone_ik(hand, 0, [20., 20.], false).unwrap();
        assert_eq!(doc.skeleton.bones[0].rotation_at(0), 360.);
        doc.set_bone_rotation_key(upper, 1, f32::MAX).unwrap();
        doc.pose_bone_ik(hand, 1, [22., 16.], false).unwrap();
        close(doc.bone_segment_at(hand, true, 1).unwrap().1, [22., 16.]);
    }

    #[test]
    fn clamps_both_reach_limits_and_handles_folded_singularities() {
        for length in [6., 10., 16.] {
            let (mut doc, upper, hand) = arm(length);
            doc.pose_bone_ik(hand, 0, [10., 100.], false).unwrap();
            close(
                doc.bone_segment_at(hand, true, 0).unwrap().1,
                [10., 20. + length],
            );
            doc.pose_bone_ik(hand, 0, [10., 11.], false).unwrap();
            close(
                doc.bone_segment_at(hand, true, 0).unwrap().1,
                [10., 10. + (10. - length).abs().max(1.)],
            );
            let elbow = doc.bone_segment_at(upper, true, 0).unwrap().1;
            doc.pose_bone_ik(hand, 0, [10., 10.], false).unwrap();
            close(doc.bone_segment_at(upper, true, 0).unwrap().1, elbow);
            let tip = doc.bone_segment_at(hand, true, 0).unwrap().1;
            assert!(((tip[0] - 10.).hypot(tip[1] - 10.) - (10. - length).abs()).abs() < 1e-4);
        }
    }

    #[test]
    fn invalid_targets_and_failed_attachment_validation_leave_both_tracks_intact() {
        let (mut doc, upper, hand) = arm(6.);
        let stacked = doc
            .add_bone("Stacked", Some(upper), [10., 10.], [10., 11.])
            .unwrap();
        let before = doc.clone();
        for (bone, frame, target) in [
            (999, 1, [1.; 2]),
            (upper, 1, [1.; 2]),
            (stacked, 1, [1.; 2]),
            (hand, doc.frames, [1.; 2]),
            (hand, 1, [f32::NAN, 0.]),
        ] {
            assert!(doc.pose_bone_ik(bone, frame, target, false).is_err());
            assert_eq!(doc, before);
        }
        let art = doc.add_rectangle([10., 10.], [2.; 2], [255; 4]);
        doc.bind_rigid(art, Some(hand)).unwrap();
        doc.objects[0].rigid_bone = Some(999);
        let before = doc.clone();
        assert!(doc.pose_bone_ik(hand, 2, [18., 15.], false).is_err());
        assert_eq!(doc, before);
    }
}
