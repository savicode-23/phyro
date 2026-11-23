use super::{editable_selection, Clipboard};
use crate::timeline::KeyMode;
use document_core::{Document, Gradient, KeyValue, Property, VectorObject};
use std::collections::HashSet;

struct PaintStyle {
    fill: [u8; 4],
    no_fill: bool,
    gradient: Option<Gradient>,
    stroke_gradient: Option<Gradient>,
    stroke: [u8; 4],
    no_stroke: bool,
    stroke_width: f32,
}

impl PaintStyle {
    fn at(object: &VectorObject, frame: u32) -> Self {
        Self {
            fill: object.fill_at(frame),
            no_fill: object.no_fill,
            gradient: object.gradient.clone(),
            stroke_gradient: object.stroke_gradient.clone(),
            stroke: object.stroke_at(frame),
            no_stroke: object.no_stroke,
            stroke_width: object.stroke_width_at(frame),
        }
    }

    fn apply(&self, object: &mut VectorObject, frame: u32, mode: KeyMode) -> bool {
        let fill = self.fill != object.fill_at(frame)
            && mode.set(object, frame, Property::Fill, KeyValue::Color(self.fill));
        let fill_paint = (self.gradient != object.gradient || self.no_fill != object.no_fill)
            && mode.allows(object, frame, Property::Fill);
        if fill_paint {
            object.no_fill = self.no_fill;
            object.gradient.clone_from(&self.gradient);
        }
        let stroke = self.stroke != object.stroke_at(frame)
            && mode.set(
                object,
                frame,
                Property::Stroke,
                KeyValue::Color(self.stroke),
            );
        let stroke_paint = (self.stroke_gradient != object.stroke_gradient
            || self.no_stroke != object.no_stroke)
            && mode.allows(object, frame, Property::Stroke);
        if stroke_paint {
            object.no_stroke = self.no_stroke;
            object.stroke_gradient.clone_from(&self.stroke_gradient);
        }
        let width = self.stroke_width != object.stroke_width_at(frame)
            && mode.set(
                object,
                frame,
                Property::StrokeWidth,
                KeyValue::Scalar(self.stroke_width),
            );
        fill || fill_paint || stroke || stroke_paint || width
    }
}

pub(crate) fn has_style_target(doc: &Document, ids: &[u64]) -> bool {
    if ids.is_empty() {
        return false;
    }
    let selected: HashSet<_> = editable_selection(doc, ids).into_iter().collect();
    doc.objects
        .iter()
        .any(|o| super::has_vector_paint(o) && selected.contains(&o.id))
}

impl Clipboard {
    /// Copies retain paint order, even across layer-local groups and native clipboard round trips.
    pub(super) fn style_source(&self) -> Option<&VectorObject> {
        self.objects
            .iter()
            .rev()
            .find(|o| super::has_vector_paint(o))
    }

    pub(crate) fn has_style(&self) -> bool {
        self.style_source().is_some()
    }

    pub(crate) fn paste_style(
        &self,
        doc: &mut Document,
        ids: &[u64],
        frame: u32,
        mode: KeyMode,
    ) -> usize {
        let Some(source) = self.style_source() else {
            return 0;
        };
        let style = PaintStyle::at(source, self.source_frame);
        let selected: HashSet<_> = editable_selection(doc, ids).into_iter().collect();
        let mut changed = 0;
        for object in &mut doc.objects {
            if selected.contains(&object.id)
                && super::has_vector_paint(object)
                && style.apply(object, frame, mode)
            {
                changed += 1;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::{GradientKind, GradientStop, LineCap, PathNode, Shape};

    fn path(doc: &mut Document, x: f32, layer: u64) -> u64 {
        doc.add_shape(
            [x, 8.],
            [12., 7.],
            [15, 65, 120, 200],
            Shape::Path {
                nodes: [[0., 0.], [1., 0.], [0.5, 1.]]
                    .map(PathNode::corner)
                    .to_vec(),
                closed: true,
            },
            layer,
        )
    }

    fn gradient() -> Gradient {
        Gradient {
            kind: GradientKind::Linear {
                start: [0., 0.1],
                end: [1., 0.8],
            },
            stops: vec![
                GradientStop {
                    offset: 0.,
                    color: [15, 90, 160, 210],
                },
                GradientStop {
                    offset: 1.,
                    color: [240, 70, 90, 75],
                },
            ],
        }
    }

    #[test]
    fn round_trip_uses_frontmost_source_at_copy_time_and_only_changes_selected_paint() {
        let mut source = Document::default();
        let high = crate::commands::add_layer(&mut source);
        let top = path(&mut source, 8., high);
        let bottom = path(&mut source, 40., 1);
        let o = &mut source.objects[0];
        o.no_fill = true;
        o.gradient = Some(gradient());
        o.stroke_gradient = Some(Gradient::from_color(
            GradientKind::Rectangular {
                center: [0.2, 0.8],
                radius: 0.6,
            },
            [94, 71, 130, 219],
        ));
        o.animated.fill_keys.extend([(0, [14; 4]), (4, [114; 4])]);
        o.animated
            .stroke_keys
            .extend([(0, [120; 4]), (4, [200; 4])]);
        o.animated.width_keys.extend([(0, 2.), (4, 8.)]);
        let paint = PaintStyle::at(o, 2);
        let mut copied = Clipboard::default();
        copied.copy(&source, &[top, bottom], 2);
        let encoded = copied.encode().unwrap();
        assert!(encoded.starts_with("Phyro/vector-objects/4\n"));
        let copied = Clipboard::decode(&encoded).unwrap();
        assert!(copied.has_style());
        let mut dst = Document::default();
        let sibling = crate::commands::add_layer(&mut dst);
        let locked = crate::commands::add_layer(&mut dst);
        dst.layers.last_mut().unwrap().locked = true;
        let a = path(&mut dst, 8., 1);
        path(&mut dst, 30., 1);
        path(&mut dst, 61., sibling);
        let ignored = path(&mut dst, 80., locked);
        for o in &mut dst.objects {
            o.no_stroke = true;
            o.group = Some(17);
            o.opacity = 0.4;
            o.rotation = 23.;
            o.cap = LineCap::Square;
        }
        let mut expected = dst.clone();
        for o in &mut expected.objects[..2] {
            o.no_fill = true;
            o.no_stroke = false;
            o.fill = paint.fill;
            o.gradient = paint.gradient.clone();
            o.stroke_gradient = paint.stroke_gradient.clone();
            o.stroke = paint.stroke;
            o.stroke_width = paint.stroke_width;
        }
        assert!(has_style_target(&dst, &[a]));
        assert!(!has_style_target(&dst, &[ignored]));
        assert_eq!(
            copied.paste_style(&mut dst, &[a, ignored], 3, KeyMode::Tracked),
            2
        );
        assert_eq!(dst, expected);
        assert_eq!(copied.paste_style(&mut dst, &[a], 3, KeyMode::Auto), 0);
        assert_eq!(dst, expected);
        let count = dst.objects.len();
        assert_eq!(copied.paste(&mut dst, 1).len(), 2);
        assert_eq!(
            dst.objects[count + 1].animated.fill_keys,
            source.objects[0].animated.fill_keys
        );
    }

    #[test]
    fn legacy_payload_pastes_solid_style_with_tracked_manual_and_auto_rules() {
        let mut source = Document::default();
        path(&mut source, 8., 1);
        let o = &mut source.objects[0];
        o.animated.fill_keys.extend([(0, [100; 4]), (4, [25; 4])]);
        o.stroke = [215, 50, 120, 190];
        o.stroke_width = 3.;
        let legacy =
            crate::commands::encode_clipboard(crate::commands::CLIPBOARD_HEADER, &source.objects)
                .unwrap();
        let copied = Clipboard::decode(&legacy).unwrap();
        for mode in [KeyMode::Tracked, KeyMode::Manual, KeyMode::Auto] {
            let mut dst = Document::default();
            let id = path(&mut dst, 10., 1);
            let o = &mut dst.objects[0];
            o.gradient = Some(gradient());
            o.animated.fill_keys.extend([(0, [9; 4]), (4, [80; 4])]);
            Property::Stroke.set_muted(o, true);
            let before = o.clone();
            assert_eq!(copied.paste_style(&mut dst, &[id], 2, mode), 1);
            let o = &dst.objects[0];
            assert_eq!(o.stroke, before.stroke);
            assert_eq!(o.animated.stroke_keys, before.animated.stroke_keys);
            assert_eq!(o.animated.fill_keys[&0], [9; 4]);
            assert_eq!(o.animated.fill_keys[&4], [80; 4]);
            if mode == KeyMode::Manual {
                assert_eq!(o.gradient, before.gradient);
                assert_eq!(o.animated.fill_keys, before.animated.fill_keys);
            } else {
                assert!(o.gradient.is_none());
                assert_eq!(o.fill_at(2), [100; 4]);
            }
            if mode == KeyMode::Auto {
                assert_eq!(o.animated.width_keys.get(&0), Some(&before.stroke_width));
                assert_eq!(o.animated.width_keys.get(&2), Some(&3.));
            } else {
                assert!(o.animated.width_keys.is_empty());
                assert_eq!(o.stroke_width, 3.);
            }
        }
    }

    #[test]
    fn independent_artwork_keeps_its_internal_paint_as_source_and_destination() {
        let mut inner = Document {
            frames: 1,
            ..Default::default()
        };
        path(&mut inner, 3., 1);
        let mut doc = Document::default();
        let ordinary = path(&mut doc, 8., 1);
        let placed = doc.add_rectangle([3.; 2], [24.; 2], [0; 4]);
        doc.objects[1].embedded_artwork = Some(std::sync::Arc::new(inner).into());
        let mut copied = Clipboard::default();
        copied.copy(&doc, &[placed], 0);
        assert!(!copied.has_style());
        assert!(!has_style_target(&doc, &[placed]));
        let before = doc.clone();
        assert_eq!(
            copied.paste_style(&mut doc, &[ordinary], 0, KeyMode::Tracked),
            0
        );
        copied.copy(&doc, &[ordinary, placed], 0);
        assert!(copied.has_style());
        assert_eq!(copied.paste_style(&mut doc, &[placed], 0, KeyMode::Auto), 0);
        assert_eq!(doc, before);
    }
}
