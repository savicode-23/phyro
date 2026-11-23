//! Linear blend skinning of cubic path nodes. Each anchor and its two handles
//! share a weight so a smooth node keeps its tangent through a pose change.
use super::{PoseTrack, RigidPose, Skeleton};
use crate::{Document, Shape, VectorObject};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinBinding {
    pub parent: u64,
    pub child: u64,
    /// Child influence in each authored contour. The parent receives the remainder.
    pub weights: Vec<Vec<f32>>,
    bind_poses: [RigidPose; 2],
    #[serde(skip)]
    tracks: [PoseTrack; 2],
}

type Point = [f64; 2];

/// Affine map with basis vectors x and y and translation t. Intermediate
/// calculations stay in double precision, like the underlying bone poses.
#[derive(Clone, Copy)]
struct Affine {
    x: Point,
    y: Point,
    t: Point,
}

impl Affine {
    const IDENTITY: Self = Self {
        x: [1., 0.],
        y: [0., 1.],
        t: [0., 0.],
    };

    fn vector(self, [x, y]: Point) -> Point {
        std::array::from_fn(|i| self.x[i] * x + self.y[i] * y)
    }

    fn point(self, p: Point) -> Point {
        let v = self.vector(p);
        std::array::from_fn(|i| v[i] + self.t[i])
    }

    fn compose(self, inner: Self) -> Self {
        Self {
            x: self.vector(inner.x),
            y: self.vector(inner.y),
            t: self.point(inner.t),
        }
    }

    fn det(self) -> f64 {
        self.x[0] * self.y[1] - self.x[1] * self.y[0]
    }

    fn inverse(self) -> Self {
        let det = self.det();
        let mut inverse = Self {
            x: [self.y[1] / det, -self.x[1] / det],
            y: [-self.y[0] / det, self.x[0] / det],
            t: [0.; 2],
        };
        inverse.t = inverse.vector(self.t.map(|x| -x));
        inverse
    }

    fn from_pose(pose: RigidPose) -> Self {
        let (sin, cos) = pose.angle.to_radians().sin_cos();
        Self {
            x: [cos, sin],
            y: [-sin, cos],
            t: pose.translation,
        }
    }

    /// Normalized path space to artboard pixels, including animated transforms.
    fn from_object(o: &VectorObject, frame: u32) -> Self {
        let (origin, size) = o.signed_box_at(frame);
        let [w, h] = size.map(f64::from);
        let (s, c) = f64::from(o.rotation_at(frame).to_radians()).sin_cos();
        let shear = f64::from(o.skew_at(frame).to_radians().tan());
        let x = [w * c, w * s];
        let y = [h * (shear * c - s), h * (shear * s + c)];
        let t = std::array::from_fn(|i| {
            f64::from(origin[i]) + 0.5 * (f64::from(size[i]) - x[i] - y[i])
        });
        Self { x, y, t }
    }

    fn blend([a, b]: [Self; 2], weight: f32) -> Self {
        if weight == 0. {
            return a;
        }
        if weight == 1. {
            return b;
        }
        let w = f64::from(weight);
        let lerp = |a: Point, b: Point| std::array::from_fn(|i| (1. - w) * a[i] + w * b[i]);
        Self {
            x: lerp(a.x, b.x),
            y: lerp(a.y, b.y),
            t: lerp(a.t, b.t),
        }
    }
}

impl SkinBinding {
    pub(super) fn validate(
        &self,
        object: &VectorObject,
        skeleton: &Skeleton,
        tracks: [&PoseTrack; 2],
        frames: u32,
    ) -> Result<(), String> {
        if object.rigid_bone.is_some()
            || object.text.is_some()
            || object.is_instance()
            || object.shape.contours().is_empty()
        {
            return Err("Skinning requires an editable path with no rigid attachment.".into());
        }
        if !skeleton
            .bones
            .iter()
            .any(|b| b.id == self.child && b.parent == Some(self.parent))
        {
            return Err("A skin binding needs a child bone and its parent in this drawing.".into());
        }
        if self.weights.len() != object.shape.contours().len()
            || self
                .weights
                .iter()
                .zip(object.shape.contours())
                .any(|(weights, nodes)| {
                    weights.len() != nodes.len() || weights.iter().any(|w| !(0.0..=1.0).contains(w))
                })
        {
            return Err("Provide a child weight from 0 to 1 for each path node.".into());
        }
        if self.bind_poses.iter().any(|pose| {
            !(-360.0..=360.0).contains(&pose.angle)
                // A rigid map can translate by more than f32::MAX even when
                // both its rest pivot and its posed pivot are representable.
                || pose.translation.iter().any(|v| !v.is_finite() || v.abs() > 3. * f64::from(f32::MAX))
        }) {
            return Err("Invalid bone transform in skin bind pose.".into());
        }
        self.validate_coordinates(object, tracks, frames)
    }

    fn validate_coordinates(
        &self,
        object: &VectorObject,
        tracks: [&PoseTrack; 2],
        frames: u32,
    ) -> Result<(), String> {
        // Every interpolated node lies inside the coordinate limits of its authored keys.
        // Checking the two affine images of that box also bounds every weighted blend.
        let mut extent: Point = [0.; 2];
        for shape in std::iter::once(&object.shape).chain(object.animated.geometry_keys.values()) {
            for node in shape.contours().iter().flatten() {
                for p in [node.point, node.incoming, node.outgoing] {
                    for i in 0..2 {
                        extent[i] = extent[i].max(f64::from(p[i]).abs());
                    }
                }
            }
        }
        if let Some(motion) = &object.motion {
            motion.validate()?;
        }
        let animated = tracks.iter().any(|t| matches!(t, PoseTrack::Animated(_)))
            || object.last_keyframe().is_some();
        for frame in 0..if animated { frames } else { 1 } {
            let Some(poses) = self.relative_poses(object, frame, tracks.map(|t| t.at(frame)))
            else {
                continue;
            };
            let world = Affine::from_object(object, frame);
            let (_, size) = object.scaled_box_at(frame);
            let safe = |map: Affine, units: [f32; 2]| {
                (0..2).all(|i| {
                    let bound =
                        map.t[i].abs() + extent[0] * map.x[i].abs() + extent[1] * map.y[i].abs();
                    bound * f64::from(units[i]).max(1.) < f64::from(f32::MAX) / 2.
                })
            };
            let units = std::array::from_fn(|i| object.size[i].max(size[i]));
            if poses
                .iter()
                .all(|&p| safe(p, units) && safe(world.compose(p), [1.; 2]))
            {
                continue;
            }
            // Only extreme coordinates need the exact shape. An unused influence
            // or a conservative bound must not reject otherwise valid artwork.
            let mut shape = object.shape_at(frame).into_owned();
            self.deform(&mut shape, poses);
            if !shape.valid_nodes(object.size)
                || !shape.valid_nodes(size)
                || shape.contours().iter().flatten().any(|n| {
                    [n.point, n.incoming, n.outgoing].into_iter().any(|p| {
                        object
                            .local_to_world(p, frame)
                            .iter()
                            .any(|v| !v.is_finite())
                    })
                })
            {
                return Err("The skinned path exceeds the editable coordinate range.".into());
            }
        }
        Ok(())
    }

    pub(super) fn is_current(&self, tracks: &BTreeMap<u64, PoseTrack>) -> bool {
        self.tracks[0] == tracks[&self.parent] && self.tracks[1] == tracks[&self.child]
    }

    pub(super) fn refresh(&mut self, tracks: &BTreeMap<u64, PoseTrack>) {
        self.tracks = [self.parent, self.child].map(|id| tracks[&id].clone());
    }

    pub(super) fn is_animated(&self) -> bool {
        self.tracks
            .iter()
            .any(|t| matches!(t, PoseTrack::Animated(_)))
    }

    /// Transform each bone's motion since binding into the current object's
    /// local frame once, before visiting any path nodes.
    fn local_poses(&self, object: &VectorObject, frame: u32) -> Option<[Affine; 2]> {
        let poses = self.tracks.each_ref().map(|track| track.at(frame));
        self.relative_poses(object, frame, poses)
    }

    fn relative_poses(
        &self,
        object: &VectorObject,
        frame: u32,
        poses: [RigidPose; 2],
    ) -> Option<[Affine; 2]> {
        if poses == self.bind_poses {
            return None;
        }
        let world = Affine::from_object(object, frame);
        let local = world.inverse();
        Some(std::array::from_fn(|i| {
            if poses[i] == self.bind_poses[i] {
                Affine::IDENTITY
            } else {
                let delta = poses[i].compose(self.bind_poses[i].inverse());
                local.compose(Affine::from_pose(delta)).compose(world)
            }
        }))
    }

    fn deform(&self, shape: &mut Shape, poses: [Affine; 2]) {
        for (contour, weights) in shape.contours_mut().iter_mut().zip(&self.weights) {
            for (node, &w) in contour.iter_mut().zip(weights) {
                let map = Affine::blend(poses, w);
                for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                    *p = map.point(p.map(f64::from)).map(|v| v as f32);
                }
            }
        }
    }
}

impl VectorObject {
    /// After changing this object's transform, recover the source path that keeps
    /// the previous posed points in the same normalized box coordinates. The
    /// weights and bone animation stay editable. Both objects share the binding.
    pub fn skin_source_after_transform(
        &self,
        previous: &VectorObject,
        frame: u32,
    ) -> Result<Option<Shape>, &'static str> {
        let Some(skin) = &self.skin else {
            return Ok(None);
        };
        let Some(before) = skin.local_poses(previous, frame) else {
            return Ok(None);
        };
        let after = skin
            .local_poses(self, frame)
            .expect("same binding and bone pose");
        let mut shape = previous.shape_at(frame).into_owned();
        let mut changed = false;
        for (contour, weights) in shape.contours_mut().iter_mut().zip(&skin.weights) {
            for (node, &weight) in contour.iter_mut().zip(weights) {
                let old = Affine::blend(before, weight);
                let new = Affine::blend(after, weight);
                let inverse = (new.det().abs() > 1e-12).then(|| new.inverse());
                for point in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                    let p = point.map(f64::from);
                    let posed = old.point(p);
                    if new.point(p).map(|v| v as f32) == posed.map(|v| v as f32) {
                        continue;
                    }
                    let corrected = inverse
                        .ok_or("The skin pose collapses this node.")?
                        .point(posed)
                        .map(|v| v as f32);
                    if !corrected.into_iter().all(f32::is_finite) {
                        return Err("The transformed skin exceeds the editable coordinate range.");
                    }
                    changed |= *point != corrected;
                    *point = corrected;
                }
            }
        }
        Ok(changed.then_some(shape))
    }

    /// Position-channel delta that translates every posed node by `world_delta`.
    /// Returns `None` when the pose cannot represent that translation.
    pub fn skin_position_delta_at(&self, world_delta: [f32; 2], frame: u32) -> Option<[f32; 2]> {
        if !world_delta.into_iter().all(f32::is_finite) {
            return None;
        }
        let Some(skin) = &self.skin else {
            // set_position_at already inverts a rigid attachment and motion offset.
            return Some(world_delta);
        };
        if world_delta == [0.; 2] {
            return Some(world_delta);
        }
        let turns: [f64; 2] = std::array::from_fn(|i| {
            (skin.tracks[i].at(frame).angle - skin.bind_poses[i].angle) % 360.
        });
        let weight = if (turns[0] - turns[1]) % 360. == 0. {
            0.
        } else {
            // Two distinct planar rotations send every nonzero vector to different
            // vectors. Varying weights therefore cannot give one common translation.
            let mut weights = skin.weights.iter().flatten().copied();
            let first = weights.next()?;
            if weights.any(|w| w != first) {
                return None;
            }
            first
        };
        if (weight == 0. && turns[0] == 0.) || (weight == 1. && turns[1] == 0.) {
            return Some(world_delta);
        }
        let rotations = turns.map(|angle| {
            Affine::from_pose(RigidPose {
                angle,
                translation: [0.; 2],
            })
        });
        let mut map = Affine::blend(rotations, weight);
        let mut reflected = [false; 2];
        for effect in &self.effects {
            if let crate::PathEffect::Mirror { horizontal } = effect {
                reflected[usize::from(!horizontal)] ^= true;
            }
        }
        if reflected != [false; 2] {
            // In local space a position change u moves the object by u and bends
            // each node by (map - I)u. Path effects mirror only this latter part.
            let world = Affine::from_object(self, frame);
            let local = world.inverse();
            map = local.compose(map).compose(world);
            for (axis, mirror) in reflected.into_iter().enumerate() {
                if mirror {
                    map.x[axis] = 2. * Affine::IDENTITY.x[axis] - map.x[axis];
                    map.y[axis] = 2. * Affine::IDENTITY.y[axis] - map.y[axis];
                }
            }
            map = world.compose(map).compose(local);
        }
        let wanted = world_delta.map(f64::from);
        let delta = if map.det().abs() > 1e-12 {
            map.inverse().vector(wanted)
        } else {
            // A collapsed map with one surviving axis still permits moves along
            // that axis. Mᵀ / ||M||² is the minimum-length inverse on its image.
            let norm = map.x.iter().chain(&map.y).map(|v| v * v).sum::<f64>();
            if norm <= 1e-12 {
                return None;
            }
            let solved = [map.x, map.y].map(|v| (v[0] * wanted[0] + v[1] * wanted[1]) / norm);
            let got = map.vector(solved);
            if (got[0] - wanted[0]).hypot(got[1] - wanted[1]) > 1e-10 * wanted[0].hypot(wanted[1]) {
                return None;
            }
            solved
        }
        .map(|v| v as f32);
        delta.into_iter().all(f32::is_finite).then_some(delta)
    }

    /// Morph the source path, then bend its nodes in response to the bone tracks.
    /// Path effects can subsequently add or remove nodes without changing weights.
    pub fn skinned_shape_at(&self, frame: u32) -> Cow<'_, Shape> {
        self.skin_shape_at(self.shape_at(frame), frame)
    }

    /// Apply the current pose to a staged source path with the bound topology.
    /// The editor uses this while dragging nodes of an uncommitted geometry key.
    pub fn skin_shape_at<'a>(&self, mut shape: Cow<'a, Shape>, frame: u32) -> Cow<'a, Shape> {
        if let Some(skin) = &self.skin {
            if let Some(poses) = skin.local_poses(self, frame) {
                skin.deform(shape.to_mut(), poses);
            }
        }
        shape
    }

    /// Recover an authored point from a drag on its posed anchor or handle.
    /// At equal weights a half-turn can collapse the node's influence map to a point.
    pub fn unskin_local_point_at(
        &self,
        contour: usize,
        node: usize,
        posed_local: [f32; 2],
        frame: u32,
    ) -> Option<[f32; 2]> {
        let Some(skin) = &self.skin else {
            return Some(posed_local);
        };
        let weight = *skin.weights.get(contour)?.get(node)?;
        let Some(poses) = skin.local_poses(self, frame) else {
            return Some(posed_local);
        };
        let map = Affine::blend(poses, weight);
        if map.det().abs() <= 1e-12 {
            return None;
        }
        let point = map
            .inverse()
            .point(posed_local.map(f64::from))
            .map(|v| v as f32);
        point.into_iter().all(f32::is_finite).then_some(point)
    }

    pub(super) fn bake_skin_at(&mut self, frame: u32) {
        if let Some(skin) = self.skin.take() {
            if let Some(poses) = skin.local_poses(self, frame) {
                skin.deform(&mut self.shape, poses);
                for shape in self.animated.geometry_keys.values_mut() {
                    skin.deform(shape, poses);
                }
            }
        }
    }
}

impl Document {
    /// Bind in the displayed pose. Existing rigid or weighted attachments are
    /// frozen at this frame before replacing them with the new pair of influences.
    pub fn bind_skin_at(
        &mut self,
        object_id: u64,
        child_id: u64,
        frame: u32,
    ) -> Result<(), String> {
        if frame >= self.frames {
            return Err("Choose a frame inside the clip.".into());
        }
        let index = self.editable_object_index(object_id)?;
        let object = &self.objects[index];
        if object.animated.guided_geometry.is_some() {
            return Err(
                "Skinning needs a fixed node layout. Choose a path without guided shape morphs."
                    .into(),
            );
        }
        if object.text.is_some() || object.is_instance() || object.shape.contours().is_empty() {
            return Err("Select an editable vector path to skin.".into());
        }
        let parent = self
            .skeleton
            .bones
            .iter()
            .find(|b| b.id == child_id)
            .and_then(|b| b.parent)
            .ok_or("Select the child bone of a two-bone chain.")?;
        if object
            .skin
            .as_ref()
            .is_some_and(|skin| skin.child == child_id && skin.parent == parent)
        {
            return Ok(());
        }
        let tracks = self.checked_rig()?;
        let tracks = [parent, child_id].map(|id| tracks[&id].clone());
        let bind_poses = tracks.each_ref().map(|t| t.at(frame));
        let segments = [
            self.bone_segment_at(parent, true, frame)?,
            self.bone_segment_at(child_id, true, frame)?,
        ];
        let mut staged = object.clone();
        staged.release_rig_at(frame);
        let world = Affine::from_object(&staged, frame);
        let weights = staged
            .shape_at(frame)
            .contours()
            .iter()
            .map(|nodes| {
                nodes
                    .iter()
                    .map(|n| initial_weight(world.point(n.point.map(f64::from)), segments))
                    .collect()
            })
            .collect();
        let skin = SkinBinding {
            parent,
            child: child_id,
            weights,
            bind_poses,
            tracks,
        };
        skin.validate(&staged, &self.skeleton, skin.tracks.each_ref(), self.frames)?;
        staged.skin = Some(Box::new(skin));
        self.objects[index] = staged;
        Ok(())
    }

    pub fn set_skin_weight(
        &mut self,
        object_id: u64,
        contour: usize,
        node: usize,
        child_weight: f32,
    ) -> Result<(), String> {
        let index = self.editable_object_index(object_id)?;
        if !(0.0..=1.0).contains(&child_weight) {
            return Err("Choose a child weight from 0 to 1.".into());
        }
        let skin = self.objects[index]
            .skin
            .as_mut()
            .ok_or("Skin the path to two bones first.")?;
        let weight = skin
            .weights
            .get_mut(contour)
            .and_then(|c| c.get_mut(node))
            .ok_or("Choose an existing path node.")?;
        let old = std::mem::replace(weight, child_weight);
        let object = &self.objects[index];
        let skin = object.skin.as_ref().unwrap();
        if let Err(error) = skin.validate_coordinates(object, skin.tracks.each_ref(), self.frames) {
            self.objects[index].skin.as_mut().unwrap().weights[contour][node] = old;
            return Err(error);
        }
        Ok(())
    }
}

/// Project onto the closest segment. Influence grows from zero at the
/// parent's head to one half at the elbow and one at the child's tip.
fn initial_weight(point: Point, segments: [([f32; 2], [f32; 2]); 2]) -> f32 {
    let [parent, child] = segments.map(|(head, tail)| {
        let head = head.map(f64::from);
        let v: Point = std::array::from_fn(|i| f64::from(tail[i]) - head[i]);
        let p: Point = std::array::from_fn(|i| point[i] - head[i]);
        let length_sq = v[0] * v[0] + v[1] * v[1];
        let t = if length_sq > 0. {
            ((p[0] * v[0] + p[1] * v[1]) / length_sq).clamp(0., 1.)
        } else {
            0.
        };
        (t, (p[0] - t * v[0]).hypot(p[1] - t * v[1]))
    });
    (if parent.1 <= child.1 {
        parent.0 * 0.5
    } else {
        0.5 + child.0 * 0.5
    }) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PathNode;

    fn sleeve() -> (Document, u64, u64, u64) {
        let mut doc = Document::default();
        let arm = doc.add_bone("Arm", None, [10., 10.], [20., 10.]).unwrap();
        let wrist = doc
            .add_bone("Wrist", Some(arm), [20., 10.], [30., 10.])
            .unwrap();
        let shape = Shape::Path {
            nodes: vec![
                PathNode::corner([0., 0.]),
                PathNode {
                    handle_style: Default::default(),
                    point: [0.5, 0.],
                    incoming: [0.4, 0.25],
                    outgoing: [0.6, -0.25],
                },
                PathNode::corner([1., 0.]),
            ],
            closed: false,
        };
        let id = doc.add_shape([10., 10.], [20., 4.], [255; 4], shape, 1);
        (doc, arm, wrist, id)
    }

    fn close(a: [f32; 2], b: [f32; 2]) {
        for i in 0..2 {
            assert!((a[i] - b[i]).abs() < 1e-4, "{a:?} vs {b:?}");
        }
    }

    fn check_translation(source: &VectorObject, frame: u32, delta: [f32; 2]) {
        let mut moved = source.clone();
        let pos = moved.position_at(frame);
        let change = moved.skin_position_delta_at(delta, frame).unwrap();
        moved.set_position_at(frame, std::array::from_fn(|i| pos[i] + change[i]));
        let controls = |o: &VectorObject| {
            o.evaluated_shape_at(frame)
                .contours()
                .iter()
                .flatten()
                .flat_map(|n| [n.point, n.incoming, n.outgoing])
                .map(o.local_to_world_at(frame))
                .collect::<Vec<_>>()
        };
        let (before, after) = (controls(source), controls(&moved));
        assert_eq!(before.len(), after.len());
        for (a, b) in before.into_iter().zip(after) {
            close(b, std::array::from_fn(|i| a[i] + delta[i]));
        }
        if let (Some(a), Some(b)) = (source.paint_space(frame), moved.paint_space(frame)) {
            close([b.tx, b.ty], [a.tx + delta[0], a.ty + delta[1]]);
        }
        assert_eq!(source.skin, moved.skin);
        assert_eq!(source.shape, moved.shape);
    }

    #[test]
    fn position_delta_translates_posed_nodes_handles_and_paint() {
        let (mut doc, arm, wrist, id) = sleeve();
        doc.frames = 6;
        let shift = [6., -3.];
        doc.bind_rigid(id, Some(arm)).unwrap();
        doc.set_bone_rotation_key(arm, 4, 30.).unwrap();
        check_translation(&doc.objects[0], 4, shift);
        doc.bind_skin_at(id, wrist, 4).unwrap();
        check_translation(&doc.objects[0], 4, shift);
        doc.set_bone_rotation_key(arm, 5, 60.).unwrap();
        // The whole chain turns together; different weights still translate as a unit.
        check_translation(&doc.objects[0], 5, shift);
        doc.set_bone_rotation_key(wrist, 5, 90.).unwrap();
        assert_eq!(doc.objects[0].skin_position_delta_at(shift, 5), None);
        for weight in [0., 0.25, 1.] {
            for n in 0..3 {
                doc.set_skin_weight(id, 0, n, weight).unwrap();
            }
            let mut o = doc.objects[0].clone();
            o.rotation = 19.;
            o.scale = [1.3, 0.7];
            o.animated.skew = -12.;
            check_translation(&o, 5, shift);
            for horizontal in [true, false, true] {
                o.effects.push(crate::PathEffect::Mirror { horizontal });
                check_translation(&o, 5, shift);
            }
        }
    }

    #[test]
    fn collapsed_skin_rejects_a_nonzero_position_delta() {
        let (mut doc, _, wrist, id) = sleeve();
        doc.bind_skin_at(id, wrist, 0).unwrap();
        for n in 0..3 {
            doc.set_skin_weight(id, 0, n, 0.5).unwrap();
        }
        doc.set_bone_rotation_key(wrist, 4, 180.).unwrap();
        let o = &doc.objects[0];
        assert_eq!(o.skin_position_delta_at([3., 0.], 4), None);
        assert_eq!(o.skin_position_delta_at([0.; 2], 4), Some([0.; 2]));
        assert_eq!(o.skin_position_delta_at([f32::INFINITY, 0.], 4), None);
        let mut mirrored = o.clone();
        mirrored
            .effects
            .push(crate::PathEffect::Mirror { horizontal: true });
        check_translation(&mirrored, 4, [3., 0.]);
        assert_eq!(mirrored.skin_position_delta_at([0., 3.], 4), None);
    }

    #[test]
    fn weights_blend_anchors_and_handles_in_artboard_space_at_each_pose() {
        let (mut doc, _, wrist, id) = sleeve();
        let source = doc.objects[0].shape.clone();
        doc.bind_skin_at(id, wrist, 0).unwrap();
        assert_eq!(
            doc.objects[0].skin.as_ref().unwrap().weights,
            [vec![0., 0.5, 1.]]
        );
        assert_eq!(doc.objects[0].skinned_shape_at(0).as_ref(), &source);
        doc.set_skin_weight(id, 0, 1, 0.25).unwrap();
        doc.set_bone_rotation_key(wrist, 0, 0.).unwrap();
        doc.set_bone_rotation_key(wrist, 4, 180.).unwrap();
        let object = &doc.objects[0];
        let shape = object.skinned_shape_at(2);
        let nodes = &shape.contours()[0];
        let world = |p| object.local_to_world(p, 2);
        close(world(nodes[0].point), [10., 10.]);
        close(world(nodes[1].point), [20., 10.]);
        // Incoming handle starts at (18, 11); at 90 degrees it reaches (19, 8).
        close(world(nodes[1].incoming), [18.25, 10.25]);
        close(world(nodes[1].outgoing), [21.75, 9.75]);
        close(world(nodes[2].point), [20., 20.]);
        close(
            object
                .unskin_local_point_at(0, 1, nodes[1].outgoing, 2)
                .unwrap(),
            source.contours()[0][1].outgoing,
        );
        assert_eq!(object.shape, source);
        let mut staged = source.clone();
        staged.contours_mut()[0][2] = PathNode::corner([0.8, 0.5]);
        let staged_pose = object.skin_shape_at(Cow::Borrowed(&staged), 2);
        close(world(staged_pose.contours()[0][2].point), [18., 16.]);
        assert_eq!(staged.contours()[0][2].point, [0.8, 0.5]);
        assert!(object.has_animated_rig());
        doc.set_skin_weight(id, 0, 1, 0.5).unwrap();
        assert_eq!(doc.objects[0].unskin_local_point_at(0, 1, [0.; 2], 4), None);
    }

    #[test]
    fn binding_a_posed_rigid_path_and_detaching_a_morph_preserve_the_selected_frame() {
        let (mut doc, arm, wrist, id) = sleeve();
        doc.bind_rigid(id, Some(arm)).unwrap();
        doc.set_bone_rotation_key(arm, 0, 0.).unwrap();
        doc.set_bone_rotation_key(arm, 4, 75.).unwrap();
        let object = &mut doc.objects[0];
        object.rotation = -32.;
        object.scale = [1.4, 0.7];
        object.animated.skew = 12.;
        let original = object.shape.clone();
        object.animated.geometry_keys.insert(0, original.clone());
        let mut morph = original.clone();
        morph.contours_mut()[0][0] = PathNode::corner([0.1, 0.5]);
        object.animated.geometry_keys.insert(4, morph);
        let before = object.outline(2).unwrap();
        doc.bind_skin_at(id, wrist, 2).unwrap();
        let object = &doc.objects[0];
        assert_eq!(object.shape, original);
        assert_eq!(object.rigid_bone, None);
        assert_eq!(object.outline(2).unwrap(), before);
        // Even with non-uniform scale and skew, 100% wrist weight must follow
        // the bone's world-space rotation around its actual posed head.
        doc.set_skin_weight(id, 0, 2, 1.).unwrap();
        let before_tip = doc.objects[0].local_to_world([1., 0.], 2);
        let (pivot, _) = doc.bone_segment_at(wrist, true, 2).unwrap();
        doc.set_bone_rotation(wrist, 90.).unwrap();
        let object = &doc.objects[0];
        let pose = object.skinned_shape_at(2);
        close(
            object.local_to_world(pose.contours()[0][2].point, 2),
            [
                pivot[0] - (before_tip[1] - pivot[1]),
                pivot[1] + (before_tip[0] - pivot[0]),
            ],
        );
        let displayed = pose.into_owned();
        doc.bind_rigid_at(id, None, 2).unwrap();
        doc.set_bone_rotation(arm, -80.).unwrap();
        let detached = &doc.objects[0];
        assert!(detached.skin.is_none());
        assert_eq!(detached.animated.geometry_keys.len(), 2);
        for (a, b) in detached.shape_at(2).contours()[0]
            .iter()
            .zip(&displayed.contours()[0])
        {
            close(a.point, b.point);
            close(a.incoming, b.incoming);
            close(a.outgoing, b.outgoing);
        }
        doc.validate().unwrap();
    }

    #[test]
    fn invalid_bindings_and_out_of_range_weights_leave_the_drawing_intact() {
        let (mut doc, arm, wrist, id) = sleeve();
        let rect = doc.add_rectangle([0.; 2], [2.; 2], [255; 4]);
        let before = doc.clone();
        assert!(doc.bind_skin_at(id, arm, 0).is_err());
        assert!(doc.bind_skin_at(rect, wrist, 0).is_err());
        assert!(doc.bind_skin_at(id, wrist, doc.frames).is_err());
        assert_eq!(doc, before);
        doc.bind_skin_at(id, wrist, 0).unwrap();
        let before = doc.clone();
        for w in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(doc.set_skin_weight(id, 0, 1, w).is_err());
        }
        assert!(doc.set_skin_weight(id, 1, 0, 0.5).is_err());
        assert!(doc.set_skin_weight(id, 0, 3, 0.5).is_err());
        assert_eq!(doc, before);
        doc.layers[0].locked = true;
        assert!(doc.set_skin_weight(id, 0, 1, 0.3).is_err());
        assert!(doc.bind_rigid_at(id, None, 0).is_err());
        assert_eq!(doc.objects, before.objects);
        doc.objects[0].skin.as_mut().unwrap().weights[0].pop();
        assert!(doc.refresh_rig().is_err());
        doc.objects[0] = before.objects[0].clone();
        doc.objects[0].rigid_bone = Some(arm);
        assert!(doc.validate().is_err());
    }

    #[test]
    fn rejects_a_bone_key_that_overflows_a_weighted_path_only_between_keys() {
        let (mut doc, _, wrist, id) = sleeve();
        let object = &mut doc.objects[0];
        object.size = [1.; 2];
        object.shape.contours_mut()[0][2] = PathNode::corner([f32::MAX * 0.8; 2]);
        doc.bind_skin_at(id, wrist, 0).unwrap();
        doc.set_skin_weight(id, 0, 2, 1.).unwrap();
        doc.set_bone_rotation_key(wrist, 0, 0.).unwrap();
        let original = doc.clone();
        assert!(doc.set_bone_rotation_key(wrist, 4, -90.).is_err());
        assert_eq!(doc, original);
        doc.validate().unwrap();
    }
}
