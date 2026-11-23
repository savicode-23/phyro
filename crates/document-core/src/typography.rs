//! Editable lettering with deterministic, portable glyph outlines.
use crate::{Document, PathNode, Shape, VectorObject, MAX_PATH_NODES};
use rustybuzz::{
    ttf_parser::{GlyphId, OutlineBuilder},
    Face, UnicodeBuffer,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Typeface {
    #[default]
    UbuntuLight,
    HackRegular,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    pub content: String,
    pub face: Typeface,
    pub size: f32,
    pub letter_spacing: f32,
    pub line_height: f32,
    pub align: TextAlign,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            content: "Text".into(),
            face: Typeface::default(),
            size: 24.,
            letter_spacing: 0.,
            line_height: 1.2,
            align: TextAlign::Left,
        }
    }
}

pub struct TextLayout {
    pub size: [f32; 2],
    pub shape: Shape,
    /// Alphabetic baseline in local pixels measured from the top of the layout.
    pub baseline: f32,
}

impl TextStyle {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.content.trim().is_empty()
            || self.content.chars().count() > 2048
            || !self.content.chars().all(|c| c == '\n' || !c.is_control())
            || !(0.25..=4096.).contains(&self.size)
            || !(-128.0..=1024.).contains(&self.letter_spacing)
            || !(0.25..=12.).contains(&self.line_height)
        {
            return Err("Enter 1–2048 characters, a positive type size and valid spacing.".into());
        }
        Ok(())
    }

    /// Size is the layout box. Outlines can overhang it, as italic letters or accents do.
    pub fn layout(&self) -> Result<TextLayout, String> {
        self.validate()?;
        let bytes = match self.face {
            Typeface::UbuntuLight => epaint_default_fonts::UBUNTU_LIGHT,
            Typeface::HackRegular => epaint_default_fonts::HACK_REGULAR,
        };
        let face = Face::from_slice(bytes, 0).ok_or("Cannot read the bundled typeface.")?;
        self.layout_face(face)
    }

    /// Shape installed-font lettering once on SVG import. Only the returned
    /// vector contours enter the drawing, making the result portable.
    pub fn outline_font(
        &self,
        bytes: &[u8],
        index: u32,
        weight: u16,
    ) -> Result<TextLayout, String> {
        self.validate()?;
        let mut face =
            Face::from_slice(bytes, index).ok_or("Cannot read the installed SVG typeface.")?;
        let tag = rustybuzz::ttf_parser::Tag::from_bytes(b"wght");
        if let Some(axis) = face.variation_axes().into_iter().find(|a| a.tag == tag) {
            let value = f32::from(weight);
            if !(axis.min_value..=axis.max_value).contains(&value) {
                return Err("The installed typeface does not contain the requested weight.".into());
            }
            face.set_variations(&[rustybuzz::Variation { tag, value }]);
        } else if face.weight().to_number() != weight {
            return Err("The requested font weight is not installed.".into());
        }
        self.layout_face(face)
    }

    fn layout_face(&self, face: Face<'_>) -> Result<TextLayout, String> {
        let scale = self.size / face.units_per_em() as f32;
        let ascent = f32::from(face.ascender()) * scale;
        let mut lines = Vec::new();
        let mut width = 0.0_f32;
        for line in self.content.split('\n') {
            let mut glyphs = Vec::new();
            let mut x = 0.0_f32;
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(line);
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &[], buffer);
            let mut previous_cluster = None;
            for (glyph, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                if glyph.glyph_id == 0 {
                    let missing = line
                        .get(glyph.cluster as usize..)
                        .and_then(|s| s.chars().next());
                    return Err(format!(
                        "The selected typeface has no glyph for {}.",
                        missing.unwrap_or('?')
                    ));
                }
                if previous_cluster.is_some_and(|cluster| cluster != glyph.cluster) {
                    x += self.letter_spacing;
                }
                glyphs.push((
                    GlyphId(glyph.glyph_id as u16),
                    x + pos.x_offset as f32 * scale,
                    -(pos.y_offset as f32) * scale,
                ));
                x += pos.x_advance as f32 * scale;
                previous_cluster = Some(glyph.cluster);
            }
            width = width.max(x);
            lines.push((glyphs, x));
        }
        let height = (lines.len() - 1) as f32 * self.size * self.line_height + ascent
            - f32::from(face.descender()) * scale;
        let size = [width.max(0.01), height.max(0.01)];
        let mut builder = Glyphs {
            contours: Vec::new(),
            pen: [0.; 2],
            scale,
            size,
        };
        let mut nodes = 0usize;
        for (line, (glyphs, advance)) in lines.into_iter().enumerate() {
            let offset = match self.align {
                TextAlign::Left => 0.,
                TextAlign::Center => (width - advance) * 0.5,
                TextAlign::Right => width - advance,
            };
            for (id, x, y) in glyphs {
                builder.pen = [
                    offset + x,
                    y + ascent + line as f32 * self.size * self.line_height,
                ];
                let start = builder.contours.len();
                face.outline_glyph(id, &mut builder);
                nodes += builder.contours[start..]
                    .iter()
                    .map(Vec::len)
                    .sum::<usize>();
                if nodes > MAX_PATH_NODES {
                    return Err("The lettering exceeds the path node limit.".into());
                }
            }
        }
        builder.contours.retain(|c| c.len() >= 2);
        if builder.contours.is_empty() {
            return Err("Enter text containing a visible glyph.".into());
        }
        Ok(TextLayout {
            size,
            shape: Shape::Compound {
                contours: builder.contours,
            },
            baseline: ascent,
        })
    }

    pub fn insert(
        self,
        doc: &mut Document,
        layer: u64,
        position: [f32; 2],
        fill: [u8; 4],
    ) -> Result<u64, String> {
        if !doc
            .layers
            .iter()
            .any(|l| l.id == layer && !l.locked && l.visible)
        {
            return Err("Choose an unlocked, visible layer for the text.".into());
        }
        let layout = self.layout()?;
        let id = doc.add_shape(position, layout.size, fill, layout.shape, layer);
        doc.objects.last_mut().unwrap().text = Some(self);
        Ok(id)
    }

    pub fn apply(self, object: &mut VectorObject) -> Result<(), String> {
        self.apply_at(object, 0)
    }

    pub fn apply_at(self, object: &mut VectorObject, frame: u32) -> Result<(), String> {
        let layout = self.layout()?;
        let mut size = layout.size;
        // The renderer transforms around the center of the size box. Reflow
        // changes that center. Pin the chosen edge of the first baseline while
        // keeping an artist's explicit placement keys untouched.
        let horizontal = match self.align {
            TextAlign::Left => 0.,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.,
        };
        let mut anchor = None;
        if let Some(old) = &object.text {
            let old = old.layout()?;
            if object.keys.is_empty() {
                anchor =
                    Some(object.local_to_world([horizontal, old.baseline / old.size[1]], frame));
            }
            for axis in 0..2 {
                size[axis] *= object.size[axis] / old.size[axis];
            }
        }
        object.shape = layout.shape;
        object.size = size;
        if let Some(anchor) = anchor {
            let after =
                object.local_to_world([horizontal, layout.baseline / layout.size[1]], frame);
            if after != anchor {
                let position = object.position_at(frame);
                object.set_position_at(
                    frame,
                    std::array::from_fn(|i| position[i] + (anchor[i] - after[i])),
                );
            }
        }
        object.text = Some(self);
        Ok(())
    }
}

struct Glyphs {
    contours: Vec<Vec<PathNode>>,
    pen: [f32; 2],
    scale: f32,
    size: [f32; 2],
}

impl Glyphs {
    fn point(&self, x: f32, y: f32) -> [f32; 2] {
        [
            (self.pen[0] + x * self.scale) / self.size[0],
            (self.pen[1] - y * self.scale) / self.size[1],
        ]
    }

    fn curve(&mut self, c1: [f32; 2], c2: [f32; 2], end: [f32; 2]) {
        let nodes = self.contours.last_mut().unwrap();
        nodes.last_mut().unwrap().outgoing = c1;
        nodes.push(PathNode {
            handle_style: Default::default(),
            point: end,
            incoming: c2,
            outgoing: end,
        });
    }
}

impl OutlineBuilder for Glyphs {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(vec![PathNode::corner(self.point(x, y))]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.contours.last_mut().unwrap().push(PathNode::corner(p));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let start = self.contours.last().unwrap().last().unwrap().point;
        let ctrl = self.point(x1, y1);
        let end = self.point(x, y);
        let cubic = |p: [f32; 2]| std::array::from_fn(|i| p[i] + (ctrl[i] - p[i]) * (2. / 3.));
        self.curve(cubic(start), cubic(end), end);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.curve(self.point(x1, y1), self.point(x2, y2), self.point(x, y));
    }
    fn close(&mut self) {
        let nodes = self.contours.last_mut().unwrap();
        if nodes.len() > 1 && nodes.first().unwrap().point == nodes.last().unwrap().point {
            nodes[0].incoming = nodes.pop().unwrap().incoming;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlining_a_supplied_face_preserves_shaping_and_requires_the_requested_weight() {
        let style = TextStyle {
            face: Typeface::HackRegular,
            content: "120 · 12:49".into(),
            ..Default::default()
        };
        let native = style.layout().unwrap();
        let face = epaint_default_fonts::HACK_REGULAR;
        let outline = style.outline_font(face, 0, 400).unwrap();
        assert_eq!(outline.shape, native.shape);
        assert_eq!(outline.size, native.size);
        assert_eq!(outline.baseline, native.baseline);
        assert!(style.outline_font(face, 0, 700).is_err());
        assert!(style.outline_font(face, 12, 400).is_err());
    }
    #[test]
    fn reflow_pins_the_first_baseline_at_the_aligned_edge_of_the_current_pose() {
        for (align, x) in [
            (TextAlign::Left, 0.),
            (TextAlign::Center, 0.5),
            (TextAlign::Right, 1.),
        ] {
            let mut doc = Document::default();
            let style = TextStyle {
                content: "Pine".into(),
                size: 15.,
                align,
                ..Default::default()
            };
            let before = style.layout().unwrap();
            style
                .clone()
                .insert(&mut doc, 1, [30., 53.], [255; 4])
                .unwrap();
            let o = &mut doc.objects[0];
            o.size[0] *= 1.3;
            o.scale_keys.extend([(0, [0.8, 1.1]), (8, [1.4, 0.7])]);
            o.rotation_keys.extend([(0, -60.), (8, 12.)]);
            o.animated.skew = 19.;
            let frame = 5;
            let anchored = o.local_to_world([x, before.baseline / before.size[1]], frame);
            let after = TextStyle {
                content: "Pine tree\nOn the hill".into(),
                size: 22.,
                ..style
            };
            let box_after = after.layout().unwrap();
            let scale_keys = o.scale_keys.clone();
            let rotation_keys = o.rotation_keys.clone();
            after.clone().apply_at(o, frame).unwrap();
            let actual = o.local_to_world([x, box_after.baseline / box_after.size[1]], frame);
            for i in 0..2 {
                assert!((actual[i] - anchored[i]).abs() < 0.0001);
            }
            assert_eq!(o.scale_keys, scale_keys);
            assert_eq!(o.rotation_keys, rotation_keys);
            assert!(o.keys.is_empty());
            o.keys.extend([(0, [40., 20.]), (8, [77., 29.])]);
            let position = (o.position, o.keys.clone());
            TextStyle {
                content: "Short".into(),
                ..after
            }
            .apply_at(o, frame)
            .unwrap();
            assert_eq!((o.position, o.keys.clone()), position);
        }
    }

    #[test]
    fn editable_multiline_type_reflows_and_preserves_artistic_resize() {
        let mut doc = Document::default();
        let mut style = TextStyle {
            content: "OO\nO".into(),
            ..TextStyle::default()
        };
        style
            .clone()
            .insert(&mut doc, 1, [3., 4.], [255; 4])
            .unwrap();
        doc.validate().unwrap();
        let obj = &mut doc.objects[0];
        let original = obj.shape.clone();
        let size = obj.size;
        assert_eq!(original.contours().len(), 6);
        obj.size[0] *= 2.;
        style.align = TextAlign::Right;
        style.clone().apply(obj).unwrap();
        assert_ne!(original, obj.shape);
        assert_eq!(obj.size, [size[0] * 2., size[1]]);
        assert_eq!(obj.position, [3., 4.]);
        let before = obj.clone();
        style.content = String::new();
        assert!(style.apply(obj).is_err());
        assert_eq!(obj, &before);
    }
}
