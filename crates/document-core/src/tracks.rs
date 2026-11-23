use crate::properties::{lerp, pair};
use crate::{Property, VectorObject};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(test)]
mod pivot_tests;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    #[default]
    Linear,
    Hold,
}

fn author<T>(base: &mut T, keys: &mut BTreeMap<u32, T>, frame: u32, value: T) {
    if keys.is_empty() {
        *base = value;
    } else {
        keys.insert(frame, value);
    }
}

impl VectorObject {
    pub fn position_at(&self, frame: u32) -> [f32; 2] {
        let base = self.sample_map(Property::Position, &self.keys, self.position, frame, pair);
        let offset = self.motion_offset_at(frame);
        let mut position = [base[0] + offset[0], base[1] + offset[1]];
        if let Some(pivot) = self.pivot {
            let delta = self.pivot_offset_at(pivot, frame);
            position = std::array::from_fn(|i| position[i] + delta[i]);
        }
        self.rigged_origin(position, frame)
    }
    pub fn rotation_at(&self, frame: u32) -> f32 {
        self.unrigged_rotation_at(frame) + self.rig_angle_at(frame)
    }
    fn unrigged_rotation_at(&self, frame: u32) -> f32 {
        self.sample_map(
            Property::Rotation,
            &self.rotation_keys,
            self.rotation,
            frame,
            lerp,
        ) + self.motion_heading_at(frame)
    }
    pub fn scale_at(&self, frame: u32) -> [f32; 2] {
        self.sample_map(
            Property::Scale,
            &self.scale_keys,
            self.scale,
            frame,
            |a, b, t| {
                // A flip is discrete. Blend the magnitudes while retaining the
                // preceding key's orientation until the next key itself is sampled.
                std::array::from_fn(|i| lerp(a[i].abs(), b[i].abs(), t).copysign(a[i]))
            },
        )
    }
    pub fn opacity_at(&self, frame: u32) -> f32 {
        self.sample_map(
            Property::Opacity,
            &self.opacity_keys,
            self.opacity,
            frame,
            lerp,
        )
    }
    pub fn set_position_at(&mut self, frame: u32, value: [f32; 2]) {
        let mut value = self.unrigged_origin(value, frame);
        if let Some(pivot) = self.pivot {
            let delta = self.pivot_offset_at(pivot, frame);
            value = std::array::from_fn(|i| value[i] - delta[i]);
        }
        let offset = self.motion_offset_at(frame);
        author(
            &mut self.position,
            &mut self.keys,
            frame,
            [value[0] - offset[0], value[1] - offset[1]],
        );
    }
    pub fn set_rotation_at(&mut self, frame: u32, value: f32) {
        let value = value - self.motion_heading_at(frame) - self.rig_angle_at(frame);
        author(&mut self.rotation, &mut self.rotation_keys, frame, value);
    }
    pub fn set_scale_at(&mut self, frame: u32, value: [f32; 2]) {
        author(&mut self.scale, &mut self.scale_keys, frame, value);
    }
    pub fn set_opacity_at(&mut self, frame: u32, value: f32) {
        author(&mut self.opacity, &mut self.opacity_keys, frame, value);
    }

    /// Translate the center-based box by q − A q, where q is the local
    /// center-to-pivot vector and A is the evaluated scale, shear and rotation.
    /// The rig transforms the entire result, including the pivot, afterwards.
    fn pivot_offset_at(&self, pivot: [f32; 2], frame: u32) -> [f32; 2] {
        let [qx, qy] = std::array::from_fn(|i| (pivot[i] - 0.5) * self.size[i]);
        let [sx, sy] = self.scale_at(frame);
        let y = sy * qy;
        let x = sx * qx + self.skew_at(frame).to_radians().tan() * y;
        let (sin, cos) = self.unrigged_rotation_at(frame).to_radians().sin_cos();
        [qx - (cos * x - sin * y), qy - (sin * x + cos * y)]
    }

    pub fn pivot_at(&self, frame: u32) -> [f32; 2] {
        self.local_to_world(self.pivot.unwrap_or([0.5; 2]), frame)
    }

    /// Place the transform pivot in artboard pixels without moving this pose.
    pub fn set_pivot_at(&mut self, frame: u32, world: [f32; 2]) -> bool {
        if !world.into_iter().all(f32::is_finite) || world == self.pivot_at(frame) {
            return false;
        }
        if world == self.local_to_world([0.5; 2], frame) {
            return self.reset_pivot_at(frame);
        }
        let local = self.world_to_local(world, frame);
        self.rebase_pivot(frame, (local != [0.5; 2]).then_some(local))
    }

    pub fn reset_pivot_at(&mut self, frame: u32) -> bool {
        self.rebase_pivot(frame, None)
    }

    fn rebase_pivot(&mut self, frame: u32, pivot: Option<[f32; 2]>) -> bool {
        if self.pivot == pivot {
            return false;
        }
        let offset = |p: Option<[f32; 2]>| p.map_or([0.; 2], |p| self.pivot_offset_at(p, frame));
        let old = offset(self.pivot);
        let new = offset(pivot);
        let delta: [f32; 2] = std::array::from_fn(|i| old[i] - new[i]);
        if delta != [0.; 2] && !self.rebase_box(self.size, delta) {
            return false;
        }
        self.pivot = pivot;
        true
    }

    /// Resize the base box, keeping a custom pivot fixed in parent space for every pose.
    pub fn set_base_size(&mut self, size: [f32; 2]) -> bool {
        if self.size == size || !size.into_iter().all(|v| v.is_finite() && v > 0.) {
            return false;
        }
        let shift = self.pivot.map_or([0.; 2], |p| {
            std::array::from_fn(|i| (self.size[i] - size[i]) * p[i])
        });
        self.rebase_box(size, shift)
    }

    fn rebase_box(&mut self, size: [f32; 2], delta: [f32; 2]) -> bool {
        // Use one constant translation so position interpolation, easing and
        // key timing survive. Check the whole edit before changing any value.
        let shifted = |p: [f32; 2]| std::array::from_fn(|i| p[i] + delta[i]);
        if std::iter::once(&self.position)
            .chain(self.keys.values())
            .any(|&p| !Self::valid_box(shifted(p), size))
        {
            return false;
        }
        if delta != [0.; 2] {
            for p in std::iter::once(&mut self.position).chain(self.keys.values_mut()) {
                *p = shifted(*p);
            }
        }
        self.size = size;
        true
    }

    pub fn last_keyframe(&self) -> Option<u32> {
        let drawing_key = self.clip_instance.as_ref().and_then(|instance| {
            instance
                .drawing_keys
                .last_key_value()
                .map(|(&tick, _)| tick)
        });
        Property::ALL
            .into_iter()
            .filter_map(|p| p.last(self))
            .chain(drawing_key)
            .max()
    }
    pub(crate) fn validate_tracks(&self, frames: u32) -> Result<(), String> {
        let valid_scale = |scale: &[f32; 2]| {
            scale
                .iter()
                .zip(self.size)
                .all(|(&s, size)| s.is_finite() && (s * size).is_finite() && s * size != 0.)
        };
        if self.last_keyframe().is_some_and(|key| key >= frames)
            || !valid_scale(&self.scale)
            || !self.scale_keys.values().all(valid_scale)
            || !self.rotation_keys.values().all(|angle| angle.is_finite())
            || !self
                .opacity_keys
                .values()
                .chain([&self.opacity])
                .all(|o| (0.0..=1.0).contains(o))
        {
            return Err(format!(
                "Object {} has invalid animation properties or key timing.",
                self.id
            ));
        }
        self.validate_extended_tracks(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn linear_and_hold_tracks_keep_the_authored_base_pose() {
        let mut doc = Document::default();
        doc.add_rectangle([1., 2.], [10., 10.], [255; 4]);
        let object = &mut doc.objects[0];
        object.rotation_keys.extend([(0, 0.), (4, 360.)]);
        object.opacity_keys.extend([(0, 0.25), (4, 0.75)]);
        object.scale_keys.extend([(0, [0.5, 1.]), (4, [1.5, 3.])]);
        assert_eq!(object.rotation_at(2), 180.);
        assert_eq!(object.opacity_at(2), 0.5);
        assert_eq!(object.scale_at(2), [1., 2.]);
        object.interpolation = Interpolation::Hold;
        assert_eq!(object.scale_at(3), [0.5, 1.]);
        assert_eq!(object.scale_at(4), [1.5, 3.]);
        object.set_opacity_at(2, 0.4);
        assert_eq!(object.opacity, 1.);
        assert_eq!(object.opacity_at(3), 0.4);
        assert_eq!(object.rotation, 0.);
    }

    #[test]
    fn validate_all_property_tracks_and_static_values() {
        let mut doc = Document::default();
        doc.add_rectangle([2., 2.], [10., 10.], [255; 4]);
        doc.objects[0].rotation_keys.insert(0, f32::INFINITY);
        assert!(doc.validate().is_err());
        doc.objects[0].rotation_keys.clear();
        doc.objects[0].scale_keys.insert(0, [0., 1.]);
        assert!(doc.validate().is_err());
        doc.objects[0].scale_keys.clear();
        doc.objects[0].opacity_keys.insert(doc.frames, 0.5);
        assert!(doc.validate().is_err());
        doc.objects[0].opacity_keys.clear();
        doc.objects[0].set_opacity_at(0, 1.01);
        assert!(doc.validate().is_err());
        doc.objects[0].set_opacity_at(0, 0.);
        assert!(doc.validate().is_ok());
    }
}
