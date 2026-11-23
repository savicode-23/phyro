use crate::{Easing, Interpolation, Shape, VectorObject, MAX_FRAMES};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum Property {
    Position,
    Rotation,
    Scale,
    Opacity,
    Skew,
    Fill,
    Stroke,
    StrokeWidth,
    Visibility,
    Geometry,
    MotionProgress,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum KeyValue {
    Scalar(f32),
    Pair([f32; 2]),
    Color([u8; 4]),
    Visible(bool),
    Shape(Shape),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimatedProperties {
    pub skew: f32,
    pub visible: bool,
    pub progress: f32,
    pub skew_keys: BTreeMap<u32, f32>,
    pub fill_keys: BTreeMap<u32, [u8; 4]>,
    pub stroke_keys: BTreeMap<u32, [u8; 4]>,
    pub width_keys: BTreeMap<u32, f32>,
    pub visible_keys: BTreeMap<u32, bool>,
    pub geometry_keys: BTreeMap<u32, Shape>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guided_geometry: Option<crate::GuidedGeometry>,
    pub progress_keys: BTreeMap<u32, f32>,
    pub easing: BTreeMap<Property, BTreeMap<u32, Easing>>,
    pub muted: BTreeSet<Property>,
}
impl Default for AnimatedProperties {
    fn default() -> Self {
        Self {
            visible: true,
            skew: 0.,
            progress: 0.,
            skew_keys: Default::default(),
            fill_keys: Default::default(),
            stroke_keys: Default::default(),
            width_keys: Default::default(),
            visible_keys: Default::default(),
            geometry_keys: Default::default(),
            guided_geometry: None,
            progress_keys: Default::default(),
            easing: Default::default(),
            muted: Default::default(),
        }
    }
}
impl AnimatedProperties {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

impl Property {
    pub const ALL: [Self; 11] = [
        Self::Position,
        Self::Rotation,
        Self::Scale,
        Self::Opacity,
        Self::Fill,
        Self::Stroke,
        Self::StrokeWidth,
        Self::Visibility,
        Self::Skew,
        Self::Geometry,
        Self::MotionProgress,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Position => "Position",
            Self::Rotation => "Rotation",
            Self::Scale => "Scale",
            Self::Opacity => "Opacity",
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
            Self::StrokeWidth => "Stroke width",
            Self::Visibility => "Visibility",
            Self::Skew => "Skew",
            Self::Geometry => "Geometry",
            Self::MotionProgress => "Path progress",
        }
    }
    pub fn is_muted(self, o: &VectorObject) -> bool {
        o.animated.muted.contains(&self)
    }
    pub fn set_muted(self, o: &mut VectorObject, mute: bool) {
        if mute {
            o.animated.muted.insert(self);
        } else {
            o.animated.muted.remove(&self);
        }
    }
    pub fn easing_at(self, o: &VectorObject, frame: u32) -> Easing {
        if self == Self::Visibility {
            return Easing::Hold;
        }
        o.animated
            .easing
            .get(&self)
            .and_then(|m| m.get(&frame))
            .copied()
            .unwrap_or(match o.interpolation {
                Interpolation::Hold => Easing::Hold,
                Interpolation::Linear => Easing::Linear,
            })
    }
    pub fn set_easing(self, o: &mut VectorObject, frame: u32, curve: Easing) -> bool {
        if self == Self::Visibility
            || self.get(o, frame).is_none()
            || !curve.valid()
            || self.easing_at(o, frame) == curve
        {
            return false;
        }
        o.animated
            .easing
            .entry(self)
            .or_default()
            .insert(frame, curve);
        true
    }
    pub fn move_key(self, o: &mut VectorObject, from: u32, to: u32) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        if self == Self::Geometry && o.animated.guided_geometry.is_some() {
            return o.move_guided_keys(&BTreeMap::from([(from, to)]));
        }
        let value = self
            .get(o, from)
            .ok_or("The source key no longer exists.")?;
        if to >= MAX_FRAMES || self.get(o, to).is_some() {
            return Err(format!("{} cannot be moved to that frame.", self.label()));
        }
        let easing = o
            .animated
            .easing
            .get(&self)
            .and_then(|m| m.get(&from))
            .copied();
        self.remove(o, from);
        self.set(o, to, value);
        if let Some(e) = easing {
            o.animated.easing.entry(self).or_default().insert(to, e);
        }
        Ok(())
    }
    pub fn add(self, o: &mut VectorObject, frame: u32) -> bool {
        let value = self.sample(o, frame);
        self.set(o, frame, value)
    }
    pub fn remove(self, o: &mut VectorObject, frame: u32) -> bool {
        if self == Self::Geometry && !o.remove_geometry_key(frame) {
            return false;
        }
        if let Some(m) = o.animated.easing.get_mut(&self) {
            m.remove(&frame);
            if m.is_empty() {
                o.animated.easing.remove(&self);
            }
        }
        match self {
            Self::Position => o.keys.remove(&frame).is_some(),
            Self::Rotation => o.rotation_keys.remove(&frame).is_some(),
            Self::Scale => o.scale_keys.remove(&frame).is_some(),
            Self::Opacity => o.opacity_keys.remove(&frame).is_some(),
            Self::Skew => o.animated.skew_keys.remove(&frame).is_some(),
            Self::Fill => o.animated.fill_keys.remove(&frame).is_some(),
            Self::Stroke => o.animated.stroke_keys.remove(&frame).is_some(),
            Self::StrokeWidth => o.animated.width_keys.remove(&frame).is_some(),
            Self::Visibility => o.animated.visible_keys.remove(&frame).is_some(),
            Self::Geometry => true,
            Self::MotionProgress => o.animated.progress_keys.remove(&frame).is_some(),
        }
    }
    pub fn get(self, o: &VectorObject, frame: u32) -> Option<KeyValue> {
        match self {
            Self::Position => o.keys.get(&frame).cloned().map(KeyValue::Pair),
            Self::Rotation => o.rotation_keys.get(&frame).cloned().map(KeyValue::Scalar),
            Self::Scale => o.scale_keys.get(&frame).cloned().map(KeyValue::Pair),
            Self::Opacity => o.opacity_keys.get(&frame).cloned().map(KeyValue::Scalar),
            Self::Skew => o
                .animated
                .skew_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Scalar),
            Self::Fill => o
                .animated
                .fill_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Color),
            Self::Stroke => o
                .animated
                .stroke_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Color),
            Self::StrokeWidth => o
                .animated
                .width_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Scalar),
            Self::Visibility => o
                .animated
                .visible_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Visible),
            Self::Geometry => o
                .animated
                .geometry_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Shape),
            Self::MotionProgress => o
                .animated
                .progress_keys
                .get(&frame)
                .cloned()
                .map(KeyValue::Scalar),
        }
    }
    pub fn frames(self, o: &VectorObject) -> Vec<u32> {
        match self {
            Self::Position => o.keys.keys().copied().collect(),
            Self::Rotation => o.rotation_keys.keys().copied().collect(),
            Self::Scale => o.scale_keys.keys().copied().collect(),
            Self::Opacity => o.opacity_keys.keys().copied().collect(),
            Self::Skew => o.animated.skew_keys.keys().copied().collect(),
            Self::Fill => o.animated.fill_keys.keys().copied().collect(),
            Self::Stroke => o.animated.stroke_keys.keys().copied().collect(),
            Self::StrokeWidth => o.animated.width_keys.keys().copied().collect(),
            Self::Visibility => o.animated.visible_keys.keys().copied().collect(),
            Self::Geometry => o.animated.geometry_keys.keys().copied().collect(),
            Self::MotionProgress => o.animated.progress_keys.keys().copied().collect(),
        }
    }
    pub fn last(self, o: &VectorObject) -> Option<u32> {
        match self {
            Self::Position => o.keys.last_key_value().map(|(&f, _)| f),
            Self::Rotation => o.rotation_keys.last_key_value().map(|(&f, _)| f),
            Self::Scale => o.scale_keys.last_key_value().map(|(&f, _)| f),
            Self::Opacity => o.opacity_keys.last_key_value().map(|(&f, _)| f),
            Self::Skew => o.animated.skew_keys.last_key_value().map(|(&f, _)| f),
            Self::Fill => o.animated.fill_keys.last_key_value().map(|(&f, _)| f),
            Self::Stroke => o.animated.stroke_keys.last_key_value().map(|(&f, _)| f),
            Self::StrokeWidth => o.animated.width_keys.last_key_value().map(|(&f, _)| f),
            Self::Visibility => o.animated.visible_keys.last_key_value().map(|(&f, _)| f),
            Self::Geometry => o.animated.geometry_keys.last_key_value().map(|(&f, _)| f),
            Self::MotionProgress => o.animated.progress_keys.last_key_value().map(|(&f, _)| f),
        }
    }
    pub fn supports_image(self) -> bool {
        !matches!(
            self,
            Self::Fill | Self::Stroke | Self::StrokeWidth | Self::Geometry
        )
    }
    pub fn compatible(self, o: &VectorObject, value: &KeyValue) -> bool {
        use KeyValue as V;
        if o.image.is_some() && !self.supports_image() {
            return false;
        }
        match (self, value) {
            (Self::Position, V::Pair(v)) => o.valid_bounds(*v),
            (Self::Scale, V::Pair(v)) => v
                .iter()
                .zip(o.size)
                .all(|(&s, extent)| s.is_finite() && (s * extent).is_finite() && s * extent != 0.),
            (Self::Rotation, V::Scalar(v)) => v.is_finite(),
            (Self::Opacity | Self::MotionProgress, V::Scalar(v)) => (0. ..=1.).contains(v),
            (Self::StrokeWidth, V::Scalar(v)) => v.is_finite() && *v >= 0.,
            (Self::Skew, V::Scalar(v)) => (-85. ..=85.).contains(v),
            (Self::Fill | Self::Stroke, V::Color(_)) | (Self::Visibility, V::Visible(_)) => true,
            (Self::Geometry, V::Shape(s)) => o.accepts_geometry_key(s),
            _ => false,
        }
    }
    pub fn set(self, o: &mut VectorObject, t: u32, v: KeyValue) -> bool {
        if t >= MAX_FRAMES || !self.compatible(o, &v) || self.get(o, t).as_ref() == Some(&v) {
            return false;
        }
        match (self, v) {
            (Self::Position, KeyValue::Pair(x)) => {
                o.keys.insert(t, x);
            }
            (Self::Rotation, KeyValue::Scalar(x)) => {
                o.rotation_keys.insert(t, x);
            }
            (Self::Scale, KeyValue::Pair(x)) => {
                o.scale_keys.insert(t, x);
            }
            (Self::Opacity, KeyValue::Scalar(x)) => {
                o.opacity_keys.insert(t, x);
            }
            (Self::Skew, KeyValue::Scalar(x)) => {
                o.animated.skew_keys.insert(t, x);
            }
            (Self::Fill, KeyValue::Color(x)) => {
                o.animated.fill_keys.insert(t, x);
            }
            (Self::Stroke, KeyValue::Color(x)) => {
                o.animated.stroke_keys.insert(t, x);
            }
            (Self::StrokeWidth, KeyValue::Scalar(x)) => {
                o.animated.width_keys.insert(t, x);
            }
            (Self::Visibility, KeyValue::Visible(x)) => {
                o.animated.visible_keys.insert(t, x);
            }
            (Self::Geometry, KeyValue::Shape(x)) => {
                return o.write_geometry_key(t, x).is_ok();
            }
            (Self::MotionProgress, KeyValue::Scalar(x)) => {
                o.animated.progress_keys.insert(t, x);
            }
            _ => return false,
        };
        true
    }
    pub fn sample(self, o: &VectorObject, t: u32) -> KeyValue {
        match self {
            Self::Position => {
                KeyValue::Pair(o.sample_map(Property::Position, &o.keys, o.position, t, pair))
            }
            Self::Rotation => KeyValue::Scalar(o.sample_map(
                Property::Rotation,
                &o.rotation_keys,
                o.rotation,
                t,
                lerp,
            )),
            Self::Scale => KeyValue::Pair(o.scale_at(t)),
            Self::Opacity => KeyValue::Scalar(o.opacity_at(t)),
            Self::Skew => KeyValue::Scalar(o.skew_at(t)),
            Self::Fill => KeyValue::Color(o.fill_at(t)),
            Self::Stroke => KeyValue::Color(o.stroke_at(t)),
            Self::StrokeWidth => KeyValue::Scalar(o.stroke_width_at(t)),
            Self::Visibility => KeyValue::Visible(o.visible_at(t)),
            Self::Geometry => KeyValue::Shape(o.shape_at(t).into_owned()),
            Self::MotionProgress => KeyValue::Scalar(o.progress_at(t)),
        }
    }
}

/// A path morph requires matching contours, winding order, node count and closure.
pub fn morphable(left: &Shape, right: &Shape) -> bool {
    let compatible = match (left, right) {
        (Shape::Path { closed: a, .. }, Shape::Path { closed: b, .. }) => a == b,
        (Shape::Compound { .. }, Shape::Compound { .. }) => true,
        _ => false,
    };
    compatible
        && left.contours().len() == right.contours().len()
        && left
            .contours()
            .iter()
            .zip(right.contours())
            .all(|(a, b)| a.len() == b.len())
}
pub(crate) fn lerp(a: f32, b: f32, t: f64) -> f32 {
    (f64::from(a) * (1. - t) + f64::from(b) * t) as f32
}
pub(crate) fn pair(a: [f32; 2], b: [f32; 2], t: f64) -> [f32; 2] {
    std::array::from_fn(|i| lerp(a[i], b[i], t))
}
fn color(a: [u8; 4], b: [u8; 4], t: f64) -> [u8; 4] {
    std::array::from_fn(|i| lerp(a[i] as f32, b[i] as f32, t).round() as u8)
}
impl VectorObject {
    fn accepts_geometry_key(&self, shape: &Shape) -> bool {
        self.text.is_none()
            && self.image.is_none()
            && !self.is_instance()
            && (morphable(&self.shape, shape) || self.accepts_guided_pose(shape))
            && shape.valid_nodes(self.size)
    }
    pub(crate) fn sample_map<T: Copy>(
        &self,
        prop: Property,
        keys: &BTreeMap<u32, T>,
        base: T,
        t: u32,
        mix: impl Fn(T, T, f64) -> T,
    ) -> T {
        if keys.is_empty() || prop.is_muted(self) {
            return base;
        }
        match (keys.range(..=t).next_back(), keys.range(t..).next()) {
            (Some((&a, &x)), Some((&b, &y))) if a != b => mix(
                x,
                y,
                prop.easing_at(self, a)
                    .sample(f64::from(t - a) / f64::from(b - a)),
            ),
            (Some((_, &value)), _) | (_, Some((_, &value))) => value,
            _ => base,
        }
    }
    pub fn skew_at(&self, t: u32) -> f32 {
        self.sample_map(
            Property::Skew,
            &self.animated.skew_keys,
            self.animated.skew,
            t,
            lerp,
        )
    }
    pub fn progress_at(&self, t: u32) -> f32 {
        self.sample_map(
            Property::MotionProgress,
            &self.animated.progress_keys,
            self.animated.progress,
            t,
            lerp,
        )
    }
    pub fn fill_at(&self, t: u32) -> [u8; 4] {
        self.sample_map(
            Property::Fill,
            &self.animated.fill_keys,
            self.fill,
            t,
            color,
        )
    }
    pub fn stroke_at(&self, t: u32) -> [u8; 4] {
        self.sample_map(
            Property::Stroke,
            &self.animated.stroke_keys,
            self.stroke,
            t,
            color,
        )
    }
    pub fn stroke_width_at(&self, t: u32) -> f32 {
        self.sample_map(
            Property::StrokeWidth,
            &self.animated.width_keys,
            self.stroke_width,
            t,
            lerp,
        )
    }
    pub fn visible_at(&self, t: u32) -> bool {
        self.sample_map(
            Property::Visibility,
            &self.animated.visible_keys,
            self.animated.visible,
            t,
            |a, _, _| a,
        )
    }
    pub fn set_shape_at(&mut self, t: u32, shape: Shape) -> Result<(), String> {
        if self.skin.is_some() && !morphable(&self.shape, &shape) {
            return Err("Detach the path before changing its skinned node layout.".into());
        }
        if self.animated.geometry_keys.is_empty() {
            if !shape.valid_nodes(self.size) {
                return Err("Invalid path geometry.".into());
            }
            self.shape = shape;
        } else {
            if !self.accepts_geometry_key(&shape) {
                if self.animated.guided_geometry.is_some() {
                    return Err("A guided Geometry key needs a single outline with the same closure as the other poses.".into());
                }
                return Err(
                    "Keep the same contour count, node count and closure for a geometry track."
                        .into(),
                );
            }
            self.write_geometry_key(t, shape)?;
        }
        Ok(())
    }
    pub fn shape_at(&self, t: u32) -> Cow<'_, Shape> {
        let keys = &self.animated.geometry_keys;
        if keys.is_empty() || Property::Geometry.is_muted(self) {
            return Cow::Borrowed(&self.shape);
        }
        match (keys.range(..=t).next_back(), keys.range(t..).next()) {
            (Some((&a, x)), Some((&b, y))) if a != b => {
                let f = Property::Geometry
                    .easing_at(self, a)
                    .sample(f64::from(t - a) / f64::from(b - a));
                if f == 0. {
                    return Cow::Borrowed(x);
                }
                if let Some(plan) = self.guided_plan(a, b, x, y) {
                    return Cow::Owned(plan.sample(f));
                }
                if !morphable(x, y) {
                    return Cow::Borrowed(x);
                }
                let mut shape = x.clone();
                for (ours, theirs) in shape.contours_mut().iter_mut().zip(y.contours()) {
                    for (a, b) in ours.iter_mut().zip(theirs) {
                        a.point = pair(a.point, b.point, f);
                        a.incoming = pair(a.incoming, b.incoming, f);
                        a.outgoing = pair(a.outgoing, b.outgoing, f);
                    }
                }
                Cow::Owned(shape)
            }
            (Some((_, v)), _) | (_, Some((_, v))) => Cow::Borrowed(v),
            _ => Cow::Borrowed(&self.shape),
        }
    }
    pub(crate) fn validate_extended_tracks(&self, frames: u32) -> Result<(), String> {
        self.validate_guided_geometry()?;
        let a = &self.animated;
        if !(-85. ..=85.).contains(&a.skew) || !(0. ..=1.).contains(&a.progress) {
            return Err("Invalid skew or motion progress.".into());
        }
        for p in Property::ALL {
            for f in p.frames(self) {
                if f >= frames || !p.compatible(self, &p.get(self, f).unwrap()) {
                    return Err(format!(
                        "Invalid {} key on object {} at frame {}.",
                        p.label(),
                        self.id,
                        f
                    ));
                }
            }
        }
        for (p, keys) in &a.easing {
            if keys
                .iter()
                .any(|(f, e)| !e.valid() || p.get(self, *f).is_none())
            {
                return Err("An easing segment must belong to a key.".into());
            }
        }
        Ok(())
    }
}
