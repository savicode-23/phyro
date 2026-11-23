use super::{Canvas, CanvasStyle};
use document_core::{Document, Gradient, KeyValue, Property, VectorObject};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaintChannel {
    Fill,
    Stroke,
}

impl PaintChannel {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
        }
    }
    pub(crate) fn property(self) -> Property {
        match self {
            Self::Fill => Property::Fill,
            Self::Stroke => Property::Stroke,
        }
    }
    pub(crate) fn is_none(self, object: &VectorObject) -> bool {
        match self {
            Self::Fill => object.no_fill,
            Self::Stroke => object.no_stroke,
        }
    }
    pub(crate) fn set_none(self, object: &mut VectorObject, none: bool) {
        match self {
            Self::Fill => object.no_fill = none,
            Self::Stroke => object.no_stroke = none,
        }
    }
    pub(crate) fn color_at(self, object: &VectorObject, frame: u32) -> [u8; 4] {
        match self {
            Self::Fill => object.fill_at(frame),
            Self::Stroke => object.stroke_at(frame),
        }
    }
    pub(crate) fn gradient(self, object: &VectorObject) -> Option<&Gradient> {
        match self {
            Self::Fill => object.gradient.as_ref(),
            Self::Stroke => object.stroke_gradient.as_ref(),
        }
    }
    pub(crate) fn gradient_mut(self, object: &mut VectorObject) -> &mut Option<Gradient> {
        match self {
            Self::Fill => &mut object.gradient,
            Self::Stroke => &mut object.stroke_gradient,
        }
    }
}

pub(crate) fn paint_picker_icon(
    painter: &eframe::egui::Painter,
    center: eframe::egui::Pos2,
    ink: eframe::egui::Color32,
) {
    use eframe::egui::{vec2, Shape, Stroke};
    let p = |x, y| center + vec2(x, y);
    let stroke = Stroke::new(1.4_f32, ink);
    painter.add(Shape::closed_line(
        vec![p(-7., 7.), p(-6., 3.), p(2., -5.), p(5., -2.), p(-3., 6.)],
        stroke,
    ));
    painter.line_segment([p(0., -6.), p(6., 0.)], stroke);
    painter.line_segment([p(4., -4.), p(7., -7.)], Stroke::new(3.0_f32, ink));
}

#[derive(Clone, Copy)]
pub(super) struct PaintSample {
    color: [u8; 4],
    stroke: bool,
}

impl PaintSample {
    pub(super) fn set_default(self, style: &mut CanvasStyle) {
        if self.stroke {
            style.no_stroke = false;
            style.stroke = self.color;
            style.stroke_gradient = None;
            if self.color[3] != 0 && style.stroke_width == 0. {
                style.stroke_width = 1.;
            }
        } else {
            style.no_fill = false;
            style.fill = self.color;
            style.gradient = None;
        }
    }

    pub(super) fn apply_to_selection(
        self,
        doc: &mut Document,
        selection: &[u64],
        frame: u32,
        style: &CanvasStyle,
    ) -> bool {
        if selection.is_empty() {
            return false;
        }
        let selected: HashSet<_> = selection.iter().copied().collect();
        let layers: HashSet<_> = doc
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked)
            .map(|l| l.id)
            .collect();
        let channel = if self.stroke {
            PaintChannel::Stroke
        } else {
            PaintChannel::Fill
        };
        let mut changed = false;
        for object in doc.objects.iter_mut().filter(|o| {
            selected.contains(&o.id)
                && layers.contains(&o.layer)
                && crate::commands::has_vector_paint(o)
        }) {
            if channel.color_at(object, frame) != self.color || channel.gradient(object).is_some() {
                if !style.key_mode.set(
                    object,
                    frame,
                    channel.property(),
                    KeyValue::Color(self.color),
                ) {
                    continue;
                }
                *channel.gradient_mut(object) = None;
                changed = true;
            }
            if channel.is_none(object) {
                if !style.key_mode.allows(object, frame, channel.property()) {
                    continue;
                }
                channel.set_none(object, false);
                changed = true;
            }
            if self.stroke && self.color[3] != 0 && object.stroke_width_at(frame) == 0. {
                let width = if style.stroke_width > 0. {
                    style.stroke_width
                } else {
                    1.
                };
                changed |= style.key_mode.set(
                    object,
                    frame,
                    Property::StrokeWidth,
                    KeyValue::Scalar(width),
                );
            }
        }
        changed
    }
}

impl Canvas {
    pub(crate) fn update_sampled_paint(
        &mut self,
        style: &mut CanvasStyle,
        paints: &mut super::PathPaintMemory,
    ) -> Option<[u8; 4]> {
        if let Some(sample) = self.sampled.take() {
            sample.set_default(style);
            paints.sampled(sample, style);
            return Some(sample.color);
        }
        None
    }

    pub(super) fn sample_paint(
        &self,
        doc: &Document,
        frame: u32,
        point: [f32; 2],
        density: f32,
        stroke: bool,
    ) -> Result<PaintSample, String> {
        let cached = self.texture.as_ref();
        let on_page = (0.0..doc.width as f32).contains(&point[0])
            && (0.0..doc.height as f32).contains(&point[1]);
        let prior = if on_page {
            cached
                .filter(|t| t.pending_split.is_none() && t.matches(doc, frame))
                .and_then(|t| self.detail.sample(t.authored(), t.frame, point))
                .or_else(|| {
                    cached
                        .and_then(|t| t.sample_texel(doc, frame, point))
                        .map(Ok)
                })
        } else {
            cached
                .filter(|t| t.pending_split.is_none() && t.matches(doc, frame))
                .and_then(|t| self.pasteboard.sample(t.authored(), t.frame, point))
        };
        let pixel = match prior {
            Some(pixel) => pixel?,
            None => {
                let scale = cached
                    .filter(|t| {
                        t.frame == frame
                            && t.document.width == doc.width
                            && t.document.height == doc.height
                    })
                    .map_or([density; 2], |t| {
                        let [w, h] = t.handle.size();
                        [w as f32 / doc.width as f32, h as f32 / doc.height as f32]
                    });
                renderer::sample_artwork(doc, frame, point, scale)?
            }
        };
        let [r, g, b, a] = pixel;
        let color = if a == 0 {
            [0; 4]
        } else {
            let straight =
                |v| ((u32::from(v) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            [straight(r), straight(g), straight(b), a]
        };
        Ok(PaintSample { color, stroke })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::KeyMode;
    use document_core::{Gradient, GradientKind, GradientStop, Layer};

    #[test]
    fn sampled_fill_uses_the_active_frame_and_protects_locked_and_manual_tracks() {
        let mut doc = Document::default();
        let first = doc.add_rectangle([0.; 2], [18.; 2], [55; 4]);
        let other = doc.add_rectangle([22.; 2], [18.; 2], [22; 4]);
        doc.layers.push(Layer {
            id: 2,
            locked: true,
            ..Default::default()
        });
        doc.objects[1].layer = 2;
        doc.objects[0]
            .animated
            .fill_keys
            .extend([(0, [50; 4]), (8, [210; 4])]);
        doc.objects[0].gradient = Some(Gradient {
            kind: GradientKind::Linear {
                start: [0., 0.],
                end: [1., 1.],
            },
            stops: vec![
                GradientStop {
                    offset: 0.,
                    color: [0; 4],
                },
                GradientStop {
                    offset: 1.,
                    color: [255; 4],
                },
            ],
        });
        let mut style = CanvasStyle {
            fill: [0; 4],
            gradient: None,
            stroke_gradient: None,
            no_fill: false,
            no_stroke: false,
            gradient_channel: crate::canvas::PaintChannel::Fill,
            cap: Default::default(),
            join: Default::default(),
            stroke: [0; 4],
            stroke_width: 1.,
            layer: 1,
            edit_motion: false,
            key_mode: KeyMode::Manual,
            snap: Default::default(),
        };
        let sample = PaintSample {
            color: [9, 48, 177, 255],
            stroke: false,
        };
        doc.objects[0].no_fill = true;
        assert!(!sample.apply_to_selection(&mut doc, &[first, other], 4, &style));
        assert!(doc.objects[0].no_fill && doc.objects[0].gradient.is_some());
        style.key_mode = KeyMode::Tracked;
        assert!(sample.apply_to_selection(&mut doc, &[first, other], 4, &style));
        assert!(!doc.objects[0].no_fill && doc.objects[0].gradient.is_none());
        assert_eq!(doc.objects[0].fill_at(0), [50; 4]);
        assert_eq!(doc.objects[0].fill_at(4), sample.color);
        assert_eq!(doc.objects[0].fill_at(8), [210; 4]);
        assert_eq!(doc.objects[1].fill_at(4), [22; 4]);

        let gradient = Gradient::from_color(
            GradientKind::Linear {
                start: [0.; 2],
                end: [1.; 2],
            },
            sample.color,
        );
        doc.objects[0].gradient = Some(gradient.clone());
        doc.objects[0].stroke_gradient = Some(gradient.clone());
        let stroke = PaintSample {
            color: doc.objects[0].stroke_at(4),
            stroke: true,
        };
        assert!(stroke.apply_to_selection(&mut doc, &[first], 4, &style));
        assert!(doc.objects[0].stroke_gradient.is_none());
        assert_eq!(doc.objects[0].gradient.as_ref(), Some(&gradient));

        // Sampling an equally transparent color turns None into solid paint
        // without creating a color key just to store the visibility change.
        doc.objects[0].no_stroke = true;
        doc.objects[0].stroke = [13, 12, 11, 0];
        let transparent = PaintSample {
            color: doc.objects[0].stroke,
            stroke: true,
        };
        style.key_mode = KeyMode::Auto;
        assert!(transparent.apply_to_selection(&mut doc, &[first], 4, &style));
        assert!(!doc.objects[0].no_stroke && doc.objects[0].animated.stroke_keys.is_empty());
    }
}
