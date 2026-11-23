use super::PoseTrack;
use crate::Document;
use std::collections::{BTreeMap, BTreeSet};

impl Document {
    /// Joint tips that can be posed or selected to release their own contact.
    /// Suppress other handles on joints already controlled by a planted foot.
    pub fn ik_handles_at(&self, frame: u32) -> Result<BTreeSet<u64>, String> {
        let pinned = self.skeleton.active_constraints(frame)?;
        let heads: BTreeMap<_, _> = self.skeleton.bones.iter().map(|b| (b.id, b.head)).collect();
        Ok(self
            .skeleton
            .bones
            .iter()
            .filter_map(|bone| {
                let parent = bone.parent?;
                let occupied = pinned.contains_key(&bone.id) || pinned.contains_key(&parent);
                (self.foot_pin_at(bone.id, frame).is_some()
                    || !occupied
                        && heads.get(&parent).is_some_and(|head| *head != bone.head)
                        && bone.head != bone.tail)
                    .then_some(bone.id)
            })
            .collect())
    }

    /// A transient drawing for painting one frame while an IK handle moves.
    /// Committing goes through `pose_bone_ik` to validate the full animation.
    pub fn preview_posed_artwork(
        &self,
        tip: u64,
        frame: u32,
        target: [f32; 2],
        flip: bool,
    ) -> Result<Document, String> {
        let reach = self.solve_bone_ik(tip, frame, target, flip)?;
        let mut scene = self.clone();
        for (index, angle) in reach.joints.into_iter().zip(reach.angles) {
            scene.skeleton.bones[index]
                .rotation_keys
                .insert(frame, angle);
        }
        let tracks: BTreeMap<_, _> = scene
            .skeleton
            .poses_at(frame)?
            .into_iter()
            .map(|(id, pose)| (id, PoseTrack::Static(pose)))
            .collect();
        for object in &mut scene.objects {
            if let Some(id) = object.rigid_bone {
                object.rig_pose = tracks.get(&id).ok_or("Missing bound bone.")?.clone();
            }
            if let Some(skin) = &mut object.skin {
                if !tracks.contains_key(&skin.parent) || !tracks.contains_key(&skin.child) {
                    return Err("Missing skin bone.".into());
                }
                skin.refresh(&tracks);
            }
        }
        Ok(scene)
    }
}
