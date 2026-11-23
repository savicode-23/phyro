use super::*;
use crate::timeline::KeyMode;
use document_core::KeyValue;

fn factors(span: Vec2, dragged: Vec2, proportional: bool) -> Vec2 {
    // The two midpoints on opposite edges share one coordinate.
    if proportional {
        return Vec2::splat((dragged.dot(span) / span.length_sq()).max(0.001));
    }
    let ratio = |span: f32, drag: f32| {
        if span == 0. {
            1.
        } else {
            (drag / span).max(0.001)
        }
    };
    egui::vec2(ratio(span.x, dragged.x), ratio(span.y, dragged.y))
}

fn limited_scale(requested: Vec2, amount: f32) -> Vec2 {
    if amount == 1. {
        requested
    } else {
        Vec2::splat(1.) + (requested - Vec2::splat(1.)) * amount
    }
}

/// Unit horizontal basis and local horizontal shear before the group stretch.
/// Object size/scale, rotation and skew together represent its full affine basis.
struct Basis {
    rotation: f32,
    skew: f32,
    cos: f64,
    sin: f64,
    shear: f64,
}

impl Basis {
    fn new(object: &VectorObject, frame: u32) -> Self {
        let rotation = object.rotation_at(frame);
        let skew = object.skew_at(frame);
        let angle = f64::from(rotation).rem_euclid(360.);
        let (sin, cos) = match angle {
            0. => (0., 1.),
            90. => (1., 0.),
            180. => (0., -1.),
            270. => (-1., 0.),
            _ => angle.to_radians().sin_cos(),
        };
        Self {
            rotation,
            skew,
            sin,
            cos,
            shear: f64::from(skew).to_radians().tan(),
        }
    }

    fn shear_at(&self, scale: Vec2) -> f64 {
        let ratio = f64::from(scale.x) / f64::from(scale.y);
        // dot(stretched x, stretched y) / det(stretched basis) gives tan(skew).
        let x = self.cos * (self.cos * self.shear - self.sin);
        let y = self.sin * (self.sin * self.shear + self.cos);
        ratio * x + y / ratio
    }

    fn limit(&self, requested: Vec2, amount: f32) -> f32 {
        let within_limit =
            |t| self.shear_at(limited_scale(requested, t)).abs() <= 85_f64.to_radians().tan();
        if requested.x == requested.y || within_limit(amount) {
            return amount;
        }
        // Limit the whole group at its first member's representable skew so
        // even at the document format's limit the outlines stay together.
        let (mut lo, mut hi) = (0., amount);
        for _ in 0..32 {
            let mid = (lo + hi) * 0.5;
            if within_limit(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    fn decompose(&self, scale: Vec2) -> (Vec2, f32) {
        if scale.x == scale.y {
            return (scale, 0.);
        }
        let (sx, sy) = (f64::from(scale.x), f64::from(scale.y));
        let (x, y) = (sx * self.cos, sy * self.sin);
        let length = x.hypot(y);
        // Rotation is a delta from the sampled angle, preserving full keyed turns.
        let turn = (self.cos * y - self.sin * x).atan2(self.cos * x + self.sin * y);
        (
            egui::vec2(length as f32, (sx * sy / length) as f32),
            turn.to_degrees() as f32,
        )
    }
}

struct Stretch {
    center: Pos2,
    size: Vec2,
    angle: Option<f32>,
    skew: Option<f32>,
}

impl Stretch {
    fn new(object: &VectorObject, basis: &Basis, frame: u32, fixed: Pos2, scale: Vec2) -> Self {
        let base_size = Vec2::from(object.size);
        let center = Pos2::from(object.position_at(frame)) + base_size * 0.5;
        let (extent, turn) = basis.decompose(scale);
        let rotation = basis.rotation;
        let skew = basis.skew;
        let next_skew = if scale.x == scale.y {
            skew
        } else {
            basis.shear_at(scale).atan().to_degrees() as f32
        };
        Self {
            center: fixed + (center - fixed) * scale,
            size: base_size * Vec2::from(object.scale_at(frame)).abs() * extent,
            angle: (rotation + turn != rotation).then_some(rotation + turn),
            skew: (next_skew != skew).then_some(next_skew),
        }
    }

    fn allowed(&self, o: &VectorObject, frame: u32, mode: KeyMode) -> bool {
        self.center.is_finite()
            && self.size.is_finite()
            && self.size.x.min(self.size.y) > 0.
            && mode.allows(o, frame, Property::Position)
            && mode.allows(o, frame, Property::Scale)
            && (self.angle.is_none() || mode.allows(o, frame, Property::Rotation))
            && (self.skew.is_none() || mode.allows(o, frame, Property::Skew))
    }

    fn apply(&self, o: &mut VectorObject, frame: u32, mode: KeyMode) {
        mode.prepare(o, frame, Property::Position);
        mode.prepare(o, frame, Property::Scale);
        if let Some(angle) = self.angle {
            mode.prepare(o, frame, Property::Rotation);
            set_angle(o, frame, angle);
        }
        if let Some(skew) = self.skew {
            mode.set(o, frame, Property::Skew, KeyValue::Scalar(skew));
        }
        resize_about_center(o, frame, self.size, self.center);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply(
    doc: &mut Document,
    ids: &GestureIds,
    frame: u32,
    fixed: Pos2,
    handle: Pos2,
    target: Pos2,
    proportional: bool,
    mode: KeyMode,
) -> bool {
    let requested = factors(handle - fixed, target - fixed, proportional);
    let one = Vec2::splat(1.);
    if !requested.is_finite() || requested == one {
        return false;
    }
    let mut amount = 1.;
    let mut members = Vec::with_capacity(ids.0.len());
    for (index, object) in doc
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| ids.contains(o.id))
    {
        let basis = Basis::new(object, frame);
        amount = basis.limit(requested, amount);
        members.push((index, basis));
    }
    let scale = limited_scale(requested, amount);
    if scale == one || members.is_empty() {
        return false;
    }
    // Preflight all affected tracks before writing any member in manual key mode.
    let mut edits = Vec::with_capacity(members.len());
    let mut skinned = Vec::new();
    for (index, basis) in members {
        let object = &doc.objects[index];
        let edit = Stretch::new(object, &basis, frame, fixed, scale);
        if !edit.allowed(object, frame, mode) {
            return false;
        }
        if object.skin.is_some() {
            let mut draft = object.clone();
            edit.apply(&mut draft, frame, mode);
            match draft.skin_source_after_transform(object, frame) {
                Ok(Some(shape)) => {
                    if !mode.prepare(&mut draft, frame, Property::Geometry)
                        || draft.set_shape_at(frame, shape).is_err()
                    {
                        return false;
                    }
                }
                Ok(None) => {}
                Err(_) => return false,
            }
            skinned.push((index, draft));
        } else {
            edits.push((index, edit));
        }
    }
    for (index, edit) in edits {
        edit.apply(&mut doc.objects[index], frame, mode);
    }
    for (index, object) in skinned {
        doc.objects[index] = object;
    }
    true
}
