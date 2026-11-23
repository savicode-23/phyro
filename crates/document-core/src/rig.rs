//! Drawing-local skeletons. Bone endpoints are authored in artboard pixels.
use crate::{Document, Interpolation, VectorObject};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

mod ik;
mod ik_preview;
mod pins;
mod skin;
pub use pins::{FootPin, PlantedSole};
pub use skin::SkinBinding;

pub const MAX_BONES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneSegment {
    pub id: u64,
    pub head: [f32; 2],
    pub tail: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bone {
    pub id: u64,
    pub name: String,
    pub parent: Option<u64>,
    pub head: [f32; 2],
    pub tail: [f32; 2],
    /// Clockwise rotation relative to the rest orientation and the parent's pose.
    pub rotation: f32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rotation_keys: BTreeMap<u32, f32>,
    #[serde(default, skip_serializing_if = "crate::is_linear")]
    pub interpolation: Interpolation,
}

impl Bone {
    pub fn rotation_at(&self, frame: u32) -> f32 {
        match (
            self.rotation_keys.range(..=frame).next_back(),
            self.rotation_keys.range(frame..).next(),
        ) {
            (Some((&a, &from)), Some((&b, &to)))
                if a < b && self.interpolation == Interpolation::Linear =>
            {
                crate::properties::lerp(from, to, f64::from(frame - a) / f64::from(b - a))
            }
            (Some((_, &value)), _) | (None, Some((_, &value))) => value,
            _ => self.rotation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
    pub next_id: u64,
    /// Per-tip contact changes in drawing time. A release takes effect on its own tick.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub foot_pins: BTreeMap<u64, BTreeMap<u32, Option<FootPin>>>,
}

impl Default for Skeleton {
    fn default() -> Self {
        Self {
            bones: Vec::new(),
            next_id: 1,
            foot_pins: BTreeMap::new(),
        }
    }
}

/// One rigid transformation in artboard space. Keeping the bind offset separate
/// lets an animator rebind artwork in any pose without altering its existing tracks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RigidPose {
    angle: f64,
    translation: [f64; 2],
}

impl RigidPose {
    pub(crate) fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    fn rotate(angle: f64, [x, y]: [f64; 2]) -> [f64; 2] {
        if angle == 0. {
            return [x, y];
        }
        let (s, c) = angle.to_radians().sin_cos();
        [c * x - s * y, s * x + c * y]
    }

    fn point(self, point: [f64; 2]) -> [f64; 2] {
        let rotated = Self::rotate(self.angle, point);
        std::array::from_fn(|i| rotated[i] + self.translation[i])
    }

    fn about(pivot: [f32; 2], degrees: f32) -> Self {
        let angle = f64::from(degrees) % 360.;
        let pivot = pivot.map(f64::from);
        let rot = Self::rotate(angle, pivot);
        Self {
            angle,
            translation: std::array::from_fn(|i| pivot[i] - rot[i]),
        }
    }

    fn compose(self, local: Self) -> Self {
        if self.is_identity() {
            return local;
        }
        if local.is_identity() {
            return self;
        }
        Self {
            angle: (self.angle + local.angle) % 360.,
            translation: self.point(local.translation),
        }
    }

    fn inverse(self) -> Self {
        Self {
            angle: -self.angle,
            translation: Self::rotate(-self.angle, self.translation.map(|v| -v)),
        }
    }

    pub(crate) fn box_origin(self, top_left: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        if self.is_identity() {
            return top_left;
        }
        let half = size.map(|v| f64::from(v) * 0.5);
        let center = std::array::from_fn(|i| f64::from(top_left[i]) + half[i]);
        let posed = self.point(center);
        std::array::from_fn(|i| (posed[i] - half[i]) as f32)
    }
}

/// Derived per attached joint. Siblings and objects share animated ancestors; static
/// chains collapse to the original constant transform. Nothing here is serialized.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PoseTrack {
    Static(RigidPose),
    Animated(Arc<AnimatedJoint>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnimatedJoint {
    parent: PoseTrack,
    bone: Bone,
    contacts: BTreeMap<u32, pins::ContactSpan>,
}

impl Default for PoseTrack {
    fn default() -> Self {
        Self::Static(RigidPose::default())
    }
}

impl PoseTrack {
    fn child(&self, bone: &Bone, contacts: BTreeMap<u32, pins::ContactSpan>) -> Self {
        if let Self::Static(parent) = self {
            if bone.rotation_keys.is_empty() && contacts.is_empty() {
                return Self::Static(parent.compose(RigidPose::about(bone.head, bone.rotation)));
            }
        }
        Self::Animated(Arc::new(AnimatedJoint {
            parent: self.clone(),
            bone: bone.clone(),
            contacts,
        }))
    }

    fn at(&self, frame: u32) -> RigidPose {
        match self {
            Self::Static(pose) => *pose,
            Self::Animated(joint) => {
                let parent = joint.parent.at(frame);
                let authored = joint.bone.rotation_at(frame);
                let angle = joint
                    .contacts
                    .range(..=frame)
                    .next_back()
                    .map(|(_, span)| span)
                    .filter(|span| span.end.is_none_or(|end| frame < end))
                    .map_or(authored, |span| span.constraint.rotation(parent, authored));
                parent.compose(RigidPose::about(joint.bone.head, angle))
            }
        }
    }
}

impl Skeleton {
    pub(crate) fn is_default(&self) -> bool {
        self.bones.is_empty() && self.next_id == 1 && self.foot_pins.is_empty()
    }

    fn resolve_poses<T: Clone + Default>(
        &self,
        mut child: impl FnMut(&T, &Bone) -> Result<T, String>,
    ) -> Result<BTreeMap<u64, T>, String> {
        if self.bones.len() > MAX_BONES {
            return Err(format!("A drawing supports up to {MAX_BONES} bones."));
        }
        let mut bones = BTreeMap::new();
        let mut names = HashSet::new();
        for bone in &self.bones {
            if bone.id == 0 || bones.insert(bone.id, bone).is_some() {
                return Err("Bone IDs must be unique and nonzero.".into());
            }
            if bone.name.is_empty()
                || bone.name.trim() != bone.name
                || bone.name.chars().count() > 80
                || bone.name.chars().any(char::is_control)
                || !names.insert(bone.name.as_str())
            {
                return Err("Give each bone a unique name of 1–80 printable characters.".into());
            }
            if !bone
                .head
                .into_iter()
                .chain(bone.tail)
                .chain([bone.rotation])
                .all(f32::is_finite)
                || bone.head == bone.tail
            {
                return Err("Give the bone distinct, finite endpoints and a finite angle.".into());
            }
        }
        if self.next_id == 0 || bones.contains_key(&self.next_id) {
            return Err("Invalid next bone ID.".into());
        }
        let mut poses = BTreeMap::new();
        for start in &self.bones {
            let mut chain = Vec::new();
            let mut id = Some(start.id);
            while let Some(next) = id {
                if poses.contains_key(&next) {
                    break;
                }
                let bone = bones
                    .get(&next)
                    .ok_or("The skeleton references a missing parent.")?;
                if chain.len() >= self.bones.len() {
                    return Err("The bone hierarchy contains a cycle.".into());
                }
                chain.push(*bone);
                id = bone.parent;
            }
            let mut pose: T = id.map_or_else(T::default, |id| T::clone(&poses[&id]));
            for bone in chain.into_iter().rev() {
                pose = child(&pose, bone)?;
                poses.insert(bone.id, pose.clone());
            }
        }
        Ok(poses)
    }

    fn poses_at(&self, frame: u32) -> Result<BTreeMap<u64, RigidPose>, String> {
        self.sample_poses(frame, |_, _| {})
    }

    pub fn posed_rotations_at(&self, frame: u32) -> Result<BTreeMap<u64, f32>, String> {
        let mut angles = BTreeMap::new();
        self.sample_poses(frame, |id, angle| {
            angles.insert(id, angle);
        })?;
        Ok(angles)
    }

    fn sample_poses(
        &self,
        frame: u32,
        mut visit: impl FnMut(u64, f32),
    ) -> Result<BTreeMap<u64, RigidPose>, String> {
        let contacts = self.active_constraints(frame)?;
        self.resolve_poses(|parent: &RigidPose, bone| {
            let authored = bone.rotation_at(frame);
            let rotation = contacts
                .get(&bone.id)
                .map_or(authored, |joint| joint.rotation(*parent, authored));
            let pose = parent.compose(RigidPose::about(bone.head, rotation));
            visit(bone.id, rotation);
            if [bone.head, bone.tail]
                .into_iter()
                .flat_map(|p| pose.point(p.map(f64::from)))
                .any(|v| !(v as f32).is_finite())
            {
                return Err("The posed skeleton exceeds artboard coordinates.".into());
            }
            Ok(pose)
        })
    }
}

impl VectorObject {
    fn effective_rig(&self, frame: u32) -> RigidPose {
        self.rig_pose.at(frame).compose(self.rig_offset)
    }

    pub fn has_animated_rig(&self) -> bool {
        matches!(self.rig_pose, PoseTrack::Animated(_))
            || self.skin.as_ref().is_some_and(|skin| skin.is_animated())
    }

    pub(crate) fn rigged_origin(&self, origin: [f32; 2], frame: u32) -> [f32; 2] {
        self.effective_rig(frame).box_origin(origin, self.size)
    }

    pub(crate) fn unrigged_origin(&self, origin: [f32; 2], frame: u32) -> [f32; 2] {
        self.effective_rig(frame)
            .inverse()
            .box_origin(origin, self.size)
    }

    pub fn rig_angle(&self) -> f32 {
        self.rig_angle_at(0)
    }

    pub fn rig_angle_at(&self, frame: u32) -> f32 {
        self.effective_rig(frame).angle as f32
    }

    pub fn rig_point(&self, point: [f32; 2]) -> [f32; 2] {
        self.rig_point_at(point, 0)
    }

    pub fn rig_point_at(&self, point: [f32; 2], frame: u32) -> [f32; 2] {
        self.effective_rig(frame)
            .point(point.map(f64::from))
            .map(|v| v as f32)
    }

    pub fn unrig_point(&self, point: [f32; 2]) -> [f32; 2] {
        self.unrig_point_at(point, 0)
    }

    pub fn unrig_point_at(&self, point: [f32; 2], frame: u32) -> [f32; 2] {
        self.effective_rig(frame)
            .inverse()
            .point(point.map(f64::from))
            .map(|v| v as f32)
    }

    pub fn release_rig(&mut self) {
        self.release_rig_at(0);
    }

    /// Freeze the current rig transform and retain the artwork's own animation tracks.
    pub fn release_rig_at(&mut self, frame: u32) {
        self.bake_skin_at(frame);
        self.rig_offset = self.effective_rig(frame);
        self.rigid_bone = None;
        self.rig_pose = PoseTrack::default();
    }
}

impl Document {
    pub fn uses_rig(&self) -> bool {
        !self.skeleton.is_default()
            || self
                .objects
                .iter()
                .any(|o| o.rigid_bone.is_some() || o.skin.is_some() || !o.rig_offset.is_identity())
    }

    pub fn add_bone(
        &mut self,
        name: &str,
        parent: Option<u64>,
        head: [f32; 2],
        tail: [f32; 2],
    ) -> Result<u64, String> {
        let mut skeleton = self.skeleton.clone();
        let id = skeleton.next_id;
        skeleton.bones.push(Bone {
            id,
            name: name.trim().to_owned(),
            parent,
            head,
            tail,
            rotation: 0.,
            rotation_keys: BTreeMap::new(),
            interpolation: Interpolation::Linear,
        });
        loop {
            skeleton.next_id = skeleton.next_id.wrapping_add(1).max(1);
            if skeleton.bones.iter().all(|b| b.id != skeleton.next_id) {
                break;
            }
        }
        skeleton.poses_at(0)?;
        self.skeleton = skeleton;
        Ok(id)
    }

    pub fn bone_segment(&self, id: u64, posed: bool) -> Result<([f32; 2], [f32; 2]), String> {
        self.bone_segment_at(id, posed, 0)
    }

    pub fn bone_segment_at(
        &self,
        id: u64,
        posed: bool,
        frame: u32,
    ) -> Result<([f32; 2], [f32; 2]), String> {
        let bone = self
            .skeleton
            .bones
            .iter()
            .find(|b| b.id == id)
            .ok_or("Choose an existing bone.")?;
        if !posed {
            return Ok((bone.head, bone.tail));
        }
        let pose = self.skeleton.poses_at(frame)?[&id];
        let point = |p: [f32; 2]| pose.point(p.map(f64::from)).map(|v| v as f32);
        Ok((point(bone.head), point(bone.tail)))
    }

    fn bone_pose_at(&self, id: u64, frame: u32) -> Result<RigidPose, String> {
        self.skeleton
            .poses_at(frame)?
            .get(&id)
            .copied()
            .ok_or_else(|| "Choose an existing bone.".into())
    }

    /// Map a rest-space child endpoint to the displayed pose of its parent.
    pub fn bone_posed_point(&self, id: u64, rest: [f32; 2]) -> Result<[f32; 2], String> {
        self.bone_posed_point_at(id, rest, 0)
    }

    pub fn bone_posed_point_at(
        &self,
        id: u64,
        rest: [f32; 2],
        frame: u32,
    ) -> Result<[f32; 2], String> {
        Ok(self
            .bone_pose_at(id, frame)?
            .point(rest.map(f64::from))
            .map(|v| v as f32))
    }

    /// Recover the authored endpoint from an artboard click on the posed drawing.
    pub fn bone_rest_point(&self, id: u64, posed: [f32; 2]) -> Result<[f32; 2], String> {
        self.bone_rest_point_at(id, posed, 0)
    }

    pub fn bone_rest_point_at(
        &self,
        id: u64,
        posed: [f32; 2],
        frame: u32,
    ) -> Result<[f32; 2], String> {
        Ok(self
            .bone_pose_at(id, frame)?
            .inverse()
            .point(posed.map(f64::from))
            .map(|v| v as f32))
    }

    pub fn bone_segments(&self, posed: bool) -> Result<Vec<BoneSegment>, String> {
        self.bone_segments_at(posed, 0)
    }

    pub fn bone_segments_at(&self, posed: bool, frame: u32) -> Result<Vec<BoneSegment>, String> {
        let poses = if posed {
            self.skeleton.poses_at(frame)?
        } else {
            BTreeMap::new()
        };
        Ok(self
            .skeleton
            .bones
            .iter()
            .map(|bone| {
                let pose = poses.get(&bone.id).copied().unwrap_or_default();
                let point = |p: [f32; 2]| pose.point(p.map(f64::from)).map(|v| v as f32);
                BoneSegment {
                    id: bone.id,
                    head: point(bone.head),
                    tail: point(bone.tail),
                }
            })
            .collect())
    }

    pub fn set_bone_rotation(&mut self, id: u64, degrees: f32) -> Result<(), String> {
        self.edit_bone(id, |bone| {
            bone.rotation = degrees;
            Ok(())
        })
    }

    pub fn set_bone_rotation_key(
        &mut self,
        id: u64,
        frame: u32,
        degrees: f32,
    ) -> Result<(), String> {
        self.edit_bone(id, |bone| {
            bone.rotation_keys.insert(frame, degrees);
            Ok(())
        })
    }

    pub fn remove_bone_rotation_key(&mut self, id: u64, frame: u32) -> Result<(), String> {
        self.edit_bone(id, |bone| {
            bone.rotation_keys
                .remove(&frame)
                .ok_or("There is no rotation key at this frame.")?;
            Ok(())
        })
    }

    pub fn move_bone_rotation_key(&mut self, id: u64, from: u32, to: u32) -> Result<(), String> {
        self.edit_bone(id, |bone| {
            let value = *bone
                .rotation_keys
                .get(&from)
                .ok_or("The source rotation key no longer exists.")?;
            if from != to {
                if bone.rotation_keys.contains_key(&to) {
                    return Err("There is already a rotation key at this frame.".into());
                }
                bone.rotation_keys.remove(&from);
                bone.rotation_keys.insert(to, value);
            }
            Ok(())
        })
    }

    pub fn set_bone_interpolation(
        &mut self,
        id: u64,
        interpolation: Interpolation,
    ) -> Result<(), String> {
        self.edit_bone(id, |bone| {
            bone.interpolation = interpolation;
            Ok(())
        })
    }

    fn edit_bone(
        &mut self,
        id: u64,
        edit: impl FnOnce(&mut Bone) -> Result<(), String>,
    ) -> Result<(), String> {
        let index = self
            .skeleton
            .bones
            .iter()
            .position(|b| b.id == id)
            .ok_or("Choose an existing bone.")?;
        let bone = &mut self.skeleton.bones[index];
        let original = bone.clone();
        if let Err(err) = edit(bone).and_then(|()| self.refresh_rig()) {
            self.skeleton.bones[index] = original;
            return Err(err);
        }
        Ok(())
    }

    pub fn bind_rigid(&mut self, object_id: u64, bone_id: Option<u64>) -> Result<(), String> {
        self.bind_rigid_at(object_id, bone_id, 0)
    }

    pub fn bind_rigid_at(
        &mut self,
        object_id: u64,
        bone_id: Option<u64>,
        frame: u32,
    ) -> Result<(), String> {
        if frame >= self.frames {
            return Err("Choose a frame inside the clip.".into());
        }
        let tracks = self.checked_rig()?;
        let track = match bone_id {
            Some(id) => tracks.get(&id).ok_or("Choose an existing bone.")?.clone(),
            None => PoseTrack::default(),
        };
        let index = self.editable_object_index(object_id)?;
        let object = &mut self.objects[index];
        if object.rigid_bone != bone_id || object.skin.is_some() {
            let old = object.clone();
            object.bake_skin_at(frame);
            let offset = track
                .at(frame)
                .inverse()
                .compose(object.effective_rig(frame));
            object.rigid_bone = bone_id;
            object.rig_pose = track;
            object.rig_offset = offset;
            if let Err(err) = self.checked_rig() {
                self.objects[index] = old;
                return Err(err);
            }
        }
        Ok(())
    }

    fn checked_rig(&self) -> Result<BTreeMap<u64, PoseTrack>, String> {
        if !(1..=crate::MAX_FRAMES).contains(&self.frames) {
            return Err("The skeleton needs a valid clip duration.".into());
        }
        let mut contacts = self.skeleton.contact_tracks(self.frames)?;
        let tracks = self.skeleton.resolve_poses(|parent: &PoseTrack, bone| {
            if bone
                .rotation_keys
                .last_key_value()
                .is_some_and(|(&f, _)| f >= self.frames)
                || bone.rotation_keys.values().any(|v| !v.is_finite())
            {
                return Err(
                    "Bone rotation keys require a finite angle and a frame inside the clip.".into(),
                );
            }
            Ok(parent.child(bone, contacts.remove(&bone.id).unwrap_or_default()))
        })?;
        let poses = self.skeleton.poses_at(0)?;
        // A rigid transform preserves Euclidean length. For ordinary coordinates even
        // a full hierarchy of translated pivots stays far inside f32 range. Only
        // extreme imported geometry needs its individual animated frames checked.
        let large = |x: f64| x.abs() > f64::from(f32::MAX) / (16. * (MAX_BONES + 1) as f64);
        let large_skeleton = self
            .skeleton
            .bones
            .iter()
            .flat_map(|b| b.head.into_iter().chain(b.tail))
            .any(|x| large(f64::from(x)));
        if large_skeleton
            && (!self.skeleton.foot_pins.is_empty()
                || self
                    .skeleton
                    .bones
                    .iter()
                    .any(|b| !b.rotation_keys.is_empty()))
        {
            for frame in 1..self.frames {
                self.skeleton.poses_at(frame)?;
            }
        }
        for o in &self.objects {
            if let Some(skin) = &o.skin {
                let parent = tracks
                    .get(&skin.parent)
                    .ok_or("The skin references a missing parent bone.")?;
                let child = tracks
                    .get(&skin.child)
                    .ok_or("The skin references a missing child bone.")?;
                skin.validate(o, &self.skeleton, [parent, child], self.frames)?;
            }
            if o.rigid_bone.is_some_and(|id| !poses.contains_key(&id)) {
                return Err("An artwork attachment references a missing bone.".into());
            }
            if !(-360.0..=360.0).contains(&o.rig_offset.angle)
                || !o.rig_offset.translation.into_iter().all(f64::is_finite)
            {
                return Err("Invalid artwork attachment offset.".into());
            }
            if o.rigid_bone.is_some() || !o.rig_offset.is_identity() {
                let pose = o
                    .rigid_bone
                    .map_or(RigidPose::default(), |id| poses[&id])
                    .compose(o.rig_offset);
                if std::iter::once(&o.position)
                    .chain(o.keys.values())
                    .any(|&p| !o.valid_bounds(pose.box_origin(p, o.size)))
                {
                    return Err("The posed artwork exceeds artboard coordinates.".into());
                }
                if let Some(track @ PoseTrack::Animated(_)) =
                    o.rigid_bone.and_then(|id| tracks.get(&id))
                {
                    let large_art = o
                        .position
                        .into_iter()
                        .chain(o.size)
                        .chain(o.keys.values().flatten().copied())
                        .any(|x| large(f64::from(x)))
                        || o.rig_offset.translation.into_iter().any(large);
                    if large_skeleton || large_art {
                        for frame in 0..self.frames {
                            let position = o.sample_map(
                                crate::Property::Position,
                                &o.keys,
                                o.position,
                                frame,
                                crate::properties::pair,
                            );
                            let offset = o.motion_offset_at(frame);
                            let origin = std::array::from_fn(|i| position[i] + offset[i]);
                            let pose = track.at(frame).compose(o.rig_offset);
                            if !o.valid_bounds(pose.box_origin(origin, o.size)) {
                                return Err(
                                    "The animated artwork exceeds artboard coordinates.".into()
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(tracks)
    }

    pub(crate) fn rig_is_current(&self) -> Result<bool, String> {
        let poses = self.checked_rig()?;
        Ok(self.objects.iter().all(|o| {
            o.skin.as_ref().is_none_or(|skin| skin.is_current(&poses))
                && o.rigid_bone.map_or_else(
                    || o.rig_pose == PoseTrack::default(),
                    |id| o.rig_pose == poses[&id],
                )
        }))
    }

    /// Recompute derived poses after loading or editing a drawing. Existing tracks stay in bind space.
    pub fn refresh_rig(&mut self) -> Result<(), String> {
        let poses = self.checked_rig()?;
        for o in &mut self.objects {
            if let Some(skin) = &mut o.skin {
                skin.refresh(&poses);
            }
            o.rig_pose = o
                .rigid_bone
                .map_or_else(PoseTrack::default, |id| poses[&id].clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: [f32; 2], expected: [f32; 2]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < 1e-4, "{actual:?} vs {expected:?}");
        }
    }

    fn arm() -> (Document, u64, u64) {
        let mut doc = Document::default();
        let upper = doc
            .add_bone("Upper arm", None, [10., 10.], [20., 10.])
            .unwrap();
        let lower = doc
            .add_bone("Forearm", Some(upper), [20., 10.], [26., 10.])
            .unwrap();
        (doc, upper, lower)
    }

    #[test]
    fn parent_and_child_rotate_about_their_own_rest_heads() {
        let (mut doc, upper, lower) = arm();
        let free = doc
            .add_bone("Other root", None, [0., 0.], [0., 2.])
            .unwrap();
        doc.set_bone_rotation(upper, 90.).unwrap();
        doc.set_bone_rotation(lower, 90.).unwrap();
        close(doc.bone_segment(upper, true).unwrap().1, [10., 20.]);
        let (head, tail) = doc.bone_segment(lower, true).unwrap();
        close(head, [10., 20.]);
        close(tail, [4., 20.]);
        close(doc.bone_segment(lower, false).unwrap().1, [26., 10.]);
        close(doc.bone_segment(free, true).unwrap().1, [0., 2.]);
        // File order never changes parentage.
        doc.skeleton.bones.reverse();
        let posed = doc.bone_segments(true).unwrap();
        close(posed.iter().find(|b| b.id == lower).unwrap().tail, tail);
    }

    #[test]
    fn attaching_rebinding_and_detaching_in_pose_preserve_all_authored_frames() {
        let (mut doc, upper, lower) = arm();
        let id = doc.add_rectangle([21., 9.], [2., 2.], [255; 4]);
        doc.objects[0].keys.extend([(0, [21., 9.]), (4, [25., 9.])]);
        doc.set_bone_rotation(upper, 30.).unwrap();
        doc.bind_rigid(id, Some(upper)).unwrap();
        close(doc.objects[0].position_at(0), [21., 9.]);
        close(doc.objects[0].position_at(4), [25., 9.]);
        doc.set_bone_rotation(upper, 120.).unwrap();
        close(doc.objects[0].position_at(0), [9., 21.]);
        close(doc.objects[0].position_at(4), [9., 25.]);
        assert!((doc.objects[0].rotation_at(0) - 90.).abs() < 1e-4);
        doc.set_bone_rotation(lower, -30.).unwrap();
        doc.bind_rigid(id, Some(lower)).unwrap();
        close(doc.objects[0].position_at(4), [9., 25.]);
        doc.set_bone_rotation(lower, -60.).unwrap();
        let posed = doc.objects[0].position_at(4);
        assert_ne!(posed, [9., 25.]);
        let rotation = doc.objects[0].rotation_at(2);
        doc.bind_rigid(id, None).unwrap();
        doc.set_bone_rotation(upper, -80.).unwrap();
        close(doc.objects[0].position_at(4), posed);
        assert_eq!(doc.objects[0].rotation_at(2), rotation);
        assert_eq!(doc.objects[0].keys[&0], [21., 9.]);
        doc.validate().unwrap();
    }

    #[test]
    fn invalid_hierarchies_and_locked_artwork_fail_atomically() {
        let (mut doc, upper, lower) = arm();
        let id = doc.add_rectangle([1., 1.], [2., 2.], [255; 4]);
        let before = doc.clone();
        for (name, parent, end) in [
            ("Upper arm", None, [2., 2.]),
            ("", None, [2., 2.]),
            ("Missing parent", Some(102), [2., 2.]),
            ("Zero length", None, [0., 0.]),
            ("Infinite", None, [f32::INFINITY, 0.]),
        ] {
            assert!(doc.add_bone(name, parent, [0., 0.], end).is_err());
        }
        assert!(doc.set_bone_rotation(upper, f32::NAN).is_err());
        assert!(doc.bind_rigid(id, Some(111)).is_err());
        assert_eq!(doc, before);
        doc.layers[0].locked = true;
        assert!(doc.bind_rigid(id, Some(lower)).is_err());
        assert_eq!(doc.objects[0].rigid_bone, None);
        doc.skeleton.bones[0].parent = Some(lower);
        assert!(doc.refresh_rig().unwrap_err().contains("cycle"));
        assert!(doc.validate().is_err());
        doc.skeleton.bones[0].parent = None;
        doc.objects[0].rigid_bone = Some(777);
        assert!(doc.refresh_rig().unwrap_err().contains("missing bone"));
    }

    #[test]
    fn rigged_artboard_setters_preserve_the_animators_world_space_drag() {
        let (mut doc, upper, _) = arm();
        let id = doc.add_rectangle([19., 9.], [2., 2.], [255; 4]);
        doc.bind_rigid(id, Some(upper)).unwrap();
        doc.set_bone_rotation(upper, 90.).unwrap();
        let object = &mut doc.objects[0];
        object.set_position_at(0, [14., 22.]);
        object.set_rotation_at(0, 20.);
        close(object.position_at(0), [14., 22.]);
        close(object.position, [22., 4.]);
        assert_eq!(object.rotation_at(0), 20.);
        assert_eq!(object.rotation, -70.);
        close(
            object.world_to_local(object.local_to_world([0.1, 0.9], 0), 0),
            [0.1, 0.9],
        );
    }

    #[test]
    fn static_symbol_capture_bakes_pose_without_carrying_a_bone_reference() {
        let (mut doc, upper, _) = arm();
        let id = doc.add_rectangle([19., 9.], [2., 2.], [255; 4]);
        doc.bind_rigid(id, Some(upper)).unwrap();
        doc.set_bone_rotation(upper, 90.).unwrap();
        let (symbol, origin) = crate::SymbolDefinition::capture("Elbow", &doc, 0, &[id]).unwrap();
        assert!(!symbol.artwork.uses_rig());
        assert_eq!(symbol.artwork.objects[0].rotation_at(0), 90.);
        close(origin, [9., 19.]);
        symbol.validate().unwrap();
    }

    #[test]
    fn keyed_ancestors_drive_attached_geometry_and_hold_their_endpoints() {
        let (mut doc, upper, lower) = arm();
        let id = doc.add_rectangle([25., 9.], [2., 2.], [255; 4]);
        doc.bind_rigid(id, Some(lower)).unwrap();
        doc.set_bone_rotation_key(upper, 1, 0.).unwrap();
        doc.set_bone_rotation_key(upper, 3, 180.).unwrap();
        let wrist = &doc.objects[0];
        assert!(wrist.has_animated_rig());
        close(wrist.position_at(0), [25., 9.]);
        close(wrist.position_at(2), [9., 25.]);
        close(wrist.position_at(4), [-7., 9.]);
        assert_eq!(doc.hit_test([10., 26.], 2), Some(id));
        let rest = [26., 10.];
        let posed = doc.bone_posed_point_at(lower, rest, 2).unwrap();
        close(posed, doc.bone_segment_at(lower, true, 2).unwrap().1);
        close(doc.bone_rest_point_at(lower, posed, 2).unwrap(), rest);
        close(wrist.unrig_point_at(wrist.rig_point_at(rest, 2), 2), rest);
        let (symbol, origin) =
            crate::SymbolDefinition::capture("Raised wrist", &doc, 2, &[id]).unwrap();
        assert!(!symbol.artwork.uses_rig());
        assert_eq!(symbol.artwork.objects[0].rotation_at(0), 90.);
        close(origin, [9., 25.]);

        doc.set_bone_interpolation(upper, Interpolation::Hold)
            .unwrap();
        close(doc.objects[0].position_at(2), [25., 9.]);
        close(doc.objects[0].position_at(3), [-7., 9.]);
        doc.set_bone_rotation_key(lower, 2, -45.).unwrap();
        assert_eq!(doc.objects[0].rotation_at(2), -45.);
        doc.objects[0].set_rotation_at(2, -20.);
        assert_eq!(doc.objects[0].rotation, 25.);
        doc.objects[0].set_position_at(2, [15., 40.]);
        close(doc.objects[0].position_at(2), [15., 40.]);

        doc.bind_rigid_at(id, Some(upper), 2).unwrap();
        close(doc.objects[0].position_at(2), [15., 40.]);
        assert_eq!(doc.objects[0].rotation_at(2), -20.);
        doc.bind_rigid_at(id, None, 3).unwrap();
        assert!(!doc.objects[0].has_animated_rig());
        close(doc.objects[0].position_at(2), doc.objects[0].position_at(3));
        doc.validate().unwrap();
    }

    #[test]
    fn key_mutations_are_atomic_and_clearing_the_track_restores_the_static_pose() {
        let (mut doc, upper, _) = arm();
        doc.set_bone_rotation(upper, 15.).unwrap();
        doc.set_bone_rotation_key(upper, 0, -10.).unwrap();
        doc.set_bone_rotation_key(upper, 3, 80.).unwrap();
        let original = doc.clone();
        assert!(doc.move_bone_rotation_key(upper, 0, 3).is_err());
        assert!(doc.move_bone_rotation_key(upper, 0, doc.frames).is_err());
        assert!(doc.set_bone_rotation_key(upper, 2, f32::NAN).is_err());
        assert!(doc.set_bone_rotation_key(upper, doc.frames, 1.).is_err());
        assert!(doc.remove_bone_rotation_key(upper, 1).is_err());
        assert_eq!(doc, original);
        doc.move_bone_rotation_key(upper, 3, 4).unwrap();
        assert_eq!(doc.skeleton.bones[0].rotation_at(2), 35.);
        doc.remove_bone_rotation_key(upper, 0).unwrap();
        doc.remove_bone_rotation_key(upper, 4).unwrap();
        assert_eq!(doc.skeleton.bones[0].rotation_at(2), 15.);
        doc.skeleton.bones[0].rotation_keys.insert(6, 20.);
        assert!(doc.validate().is_err());
    }

    #[test]
    fn overflow_at_an_interpolated_frame_rejects_the_edit_atomically() {
        let mut doc = Document::default();
        let id = doc
            .add_bone("Long lever", None, [0.; 2], [f32::MAX; 2])
            .unwrap();
        doc.set_bone_rotation_key(id, 0, 0.).unwrap();
        let before = doc.clone();
        assert!(doc.set_bone_rotation_key(id, 4, 90.).is_err());
        assert_eq!(doc, before);

        let mut doc = Document::default();
        let id = doc.add_bone("Pivot", None, [0.; 2], [10., 0.]).unwrap();
        let object = doc.add_rectangle([f32::MAX; 2], [2.; 2], [255; 4]);
        doc.bind_rigid(object, Some(id)).unwrap();
        doc.set_bone_rotation_key(id, 0, 0.).unwrap();
        let before = doc.clone();
        assert!(doc.set_bone_rotation_key(id, 4, 90.).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn nested_clip_refreshes_its_animated_attachment_and_symbol_capture_keeps_one_pose() {
        let (mut doc, upper, lower) = arm();
        let id = doc.add_rectangle([25., 9.], [2.; 2], [255; 4]);
        doc.bind_rigid(id, Some(lower)).unwrap();
        doc.set_bone_rotation_key(upper, 0, 0.).unwrap();
        doc.set_bone_rotation_key(upper, 4, 180.).unwrap();
        let mut project = crate::AnimationProject::from_document(doc);
        assert!(project.convert_to_symbol(0, 2, &[id], "Frozen").is_err());
        let parent = project.add_clip("Composite").unwrap();
        project.clips[parent].set_length(5).unwrap();
        project
            .place_clip(project.clips[0].id, parent, 0, 1, [0.; 2])
            .unwrap();
        let nested = |p: &crate::AnimationProject| {
            let instance = &p.clips[parent].drawing_at(2).objects[0];
            let (child, frame) = instance.instance_at(2).unwrap();
            child.objects[0].position_at(frame)
        };
        close(nested(&project), [9., 25.]);
        project.clips[0]
            .drawing_at_mut(0)
            .set_bone_rotation_key(upper, 2, -90.)
            .unwrap();
        project.hydrate_composition().unwrap();
        close(nested(&project), [9., -7.]);
        project.validate().unwrap();
    }

    #[test]
    fn frame_edits_retime_bone_tracks_and_keep_duplicate_drawings_independent() {
        let (mut doc, upper, lower) = arm();
        let id = doc.add_rectangle([22., 9.], [2.; 2], [255; 4]);
        doc.bind_rigid(id, Some(lower)).unwrap();
        doc.set_bone_rotation_key(upper, 0, 0.).unwrap();
        doc.set_bone_rotation_key(upper, 4, 120.).unwrap();
        doc.set_bone_interpolation(lower, Interpolation::Hold)
            .unwrap();
        doc.set_bone_rotation_key(lower, 1, 0.).unwrap();
        doc.set_bone_rotation_key(lower, 3, -30.).unwrap();
        let mut project = crate::AnimationProject::from_document(doc);
        project.add_clip("Fresh rig").unwrap();
        project.validate().unwrap();
        project.duplicate_clip(0, "Variation").unwrap();
        let source = project.clips[0].clone();
        let position = |clip: &crate::Clip, f| clip.drawing_at(f).objects[0].position_at(f);
        let mut held = source.clone();
        held.cels.insert(3, held.cels[&0].clone());
        held.set_exposure(0, 5).unwrap();
        for (f, original) in [0, 1, 2, 2, 2, 3, 4].into_iter().enumerate() {
            close(position(&held, f as u32), position(&source, original));
        }
        held.set_exposure(0, 3).unwrap();
        for f in 0..5 {
            close(position(&held, f), position(&source, f));
        }
        held.validate().unwrap();
        for first in 0..5 {
            for last in first..5 {
                let mut clip = source.clone();
                assert!(clip.set_length(4).is_err());
                clip.duplicate_range(first, last).unwrap();
                let frames: Vec<_> = (0..=last).chain(first..=last).chain(last + 1..5).collect();
                for (f, &original) in frames.iter().enumerate() {
                    close(position(&clip, f as u32), position(&source, original));
                }
                clip.validate().unwrap();
                clip.delete_range(last + 1, last + 1 + last - first)
                    .unwrap();
                for f in 0..5 {
                    close(position(&clip, f), position(&source, f));
                }
                clip.validate().unwrap();
            }
        }
        let variation = &mut project.clips[1];
        variation
            .drawing_at_mut(0)
            .set_bone_rotation_key(upper, 2, -45.)
            .unwrap();
        assert_ne!(position(variation, 2), position(&source, 2));
        assert_eq!(project.clips[0], source);
    }
}
