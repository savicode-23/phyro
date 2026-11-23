pub(super) use document_core::{KeyValue, Property as Channel};
use document_core::{Property, VectorObject};

#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum KeyMode {
    #[default]
    Tracked,
    Manual,
    Auto,
}

impl KeyMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Tracked => "Tracked",
            Self::Manual => "Manual",
            Self::Auto => "Auto",
        }
    }
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Tracked=>"Editing a tracked property adds a key. Untracked properties edit the base pose.",
            Self::Manual=>"Edit base values or an existing key. Add a key first to edit between keys.",
            Self::Auto=>"Editing any animatable property sets a key. The first frame preserves the original pose."
        }
    }
    pub(crate) fn allows(self, o: &VectorObject, t: u32, p: Property) -> bool {
        !p.is_muted(o) && (self != Self::Manual || p.last(o).is_none() || p.get(o, t).is_some())
    }
    pub(crate) fn set(self, o: &mut VectorObject, t: u32, p: Property, v: KeyValue) -> bool {
        if !p.compatible(o, &v) || !self.prepare(o, t, p) {
            return false;
        }
        if p.last(o).is_some() {
            return p.set(o, t, v);
        }
        match (p, v) {
            (Property::Fill, KeyValue::Color(c)) => o.fill = c,
            (Property::Stroke, KeyValue::Color(c)) => o.stroke = c,
            (Property::StrokeWidth, KeyValue::Scalar(x)) => o.stroke_width = x,
            (Property::Skew, KeyValue::Scalar(x)) => o.animated.skew = x,
            (Property::Visibility, KeyValue::Visible(x)) => o.animated.visible = x,
            (Property::MotionProgress, KeyValue::Scalar(x)) => o.animated.progress = x,
            _ => return false,
        }
        true
    }
    /// Seed the track before applying an actual change, preserving earlier poses.
    pub(crate) fn prepare(self, o: &mut VectorObject, t: u32, p: Property) -> bool {
        if !self.allows(o, t, p) {
            return false;
        }
        if self == Self::Auto && p.last(o).is_none() {
            p.add(o, 0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::Document;
    #[test]
    fn manual_and_auto_preserve_base_and_existing_keys() {
        let mut d = Document::default();
        d.add_rectangle([1., 2.], [5., 7.], [255; 4]);
        let o = &mut d.objects[0];
        assert!(KeyMode::Auto.prepare(o, 3, Property::Position));
        o.set_position_at(3, [12., 6.]);
        assert_eq!(o.position, [1., 2.]);
        assert_eq!(o.position_at(0), [1., 2.]);
        assert_eq!(o.position_at(3), [12., 6.]);
        assert!(!KeyMode::Manual.prepare(o, 1, Property::Position));
        assert!(KeyMode::Manual.prepare(o, 3, Property::Position));
        assert!(KeyMode::Tracked.prepare(o, 1, Property::Position));
    }
}
