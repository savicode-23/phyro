use document_core::{Gradient, GradientKind, VectorObject};
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, Pattern, Pixmap,
    PixmapRef, Point, RadialGradient, Rect, SpreadMode, Transform,
};
mod rectangular;

pub(super) struct ObjectPaint<'a> {
    object: &'a VectorObject,
    frame: u32,
    opacity: f32,
    transform: Transform,
    size: [u32; 2],
    tile: Option<Pixmap>,
}

impl<'a> ObjectPaint<'a> {
    pub(super) fn new(
        object: &'a VectorObject,
        frame: u32,
        opacity: f32,
        transform: Transform,
        surface: &Pixmap,
    ) -> Self {
        Self {
            object,
            frame,
            opacity,
            transform,
            size: [surface.width(), surface.height()],
            tile: None,
        }
    }

    pub(super) fn fill(&mut self, bounds: Rect) -> Option<Paint<'_>> {
        if let Some(image) = &self.object.image {
            let (w, h) = (image.width(), image.height());
            let pixels = PixmapRef::from_bytes(image.rgba(), w, h)?;
            let space = Transform::from_scale(1. / w as f32, 1. / h as f32)
                .post_concat(self.object.paint_space(self.frame)?);
            return Some(Paint {
                shader: Pattern::new(
                    pixels,
                    SpreadMode::Pad,
                    FilterQuality::Bilinear,
                    self.opacity,
                    space,
                ),
                anti_alias: true,
                ..Default::default()
            });
        }
        if self.object.no_fill {
            return None;
        }
        match &self.object.gradient {
            Some(g) => self.gradient(g, self.object.paint_space(self.frame)?, bounds),
            None => self.flat(self.object.fill_at(self.frame)),
        }
    }

    pub(super) fn stroke(&mut self, bounds: Rect) -> Option<Paint<'_>> {
        match &self.object.stroke_gradient {
            Some(g) => self.gradient(g, self.object.stroke_paint_space(self.frame)?, bounds),
            None => self.flat(self.object.stroke_at(self.frame)),
        }
    }

    fn flat(&self, color: [u8; 4]) -> Option<Paint<'static>> {
        (color[3] > 0).then(|| super::paint(color, self.opacity))
    }

    fn gradient(&mut self, g: &Gradient, space: Transform, bounds: Rect) -> Option<Paint<'_>> {
        let opacity = self.opacity;
        let stops = || {
            g.stops
                .iter()
                .map(|s| {
                    let [r, g, b, a] = s.color;
                    GradientStop::new(
                        s.offset,
                        Color::from_rgba8(r, g, b, (f32::from(a) * opacity).round() as u8),
                    )
                })
                .collect()
        };
        let p = |[x, y]: [f32; 2]| Point::from_xy(x, y);
        let shader = match g.kind {
            GradientKind::Linear { start, end } => {
                LinearGradient::new(p(start), p(end), stops(), SpreadMode::Pad, space)
            }
            GradientKind::Radial { center, radius } => RadialGradient::new(
                p(center),
                p(center),
                radius,
                stops(),
                SpreadMode::Pad,
                space,
            ),
            GradientKind::Rectangular { center, radius } => rectangular::shader(
                &g.stops,
                center,
                radius,
                opacity,
                space,
                bounds,
                self.transform,
                self.size,
                &mut self.tile,
            ),
        }?;
        Some(Paint {
            anti_alias: true,
            shader,
            ..Default::default()
        })
    }
}

pub(super) fn mask(
    surface: &Pixmap,
    object: &VectorObject,
    frame: u32,
    transform: Transform,
) -> Result<Option<Mask>, String> {
    let Some(stencil) = &object.stencil else {
        return Ok(None);
    };
    let mut mask = Mask::new(surface.width(), surface.height())
        .ok_or("Could not allocate the clipping mask.")?;
    let Some(path) = object
        .paint_space(frame)
        .and_then(|space| stencil.outline(space.post_concat(transform)))
    else {
        return Ok(Some(mask));
    };
    let rule = match stencil.fill_rule {
        document_core::FillRule::NonZero => FillRule::Winding,
        document_core::FillRule::EvenOdd => FillRule::EvenOdd,
    };
    mask.fill_path(&path, rule, true, Transform::identity());
    if let Some([a, b]) = stencil.alpha {
        if let Some(inv) = object
            .stencil_alpha_space(frame)
            .and_then(|space| space.post_concat(transform).invert())
        {
            // Pixels outside the path's bounds and antialiasing margin stay zero.
            let bounds = path.bounds();
            let width = surface.width() as usize;
            let height = surface.height() as usize;
            let left = ((bounds.left().floor() - 1.) as usize).min(width);
            let right = ((bounds.right().ceil() + 1.) as usize).min(width);
            let top = ((bounds.top().floor() - 1.) as usize).min(height);
            let bottom = ((bounds.bottom().ceil() + 1.) as usize).min(height);
            for (row_index, row) in mask.data_mut()[top * width..bottom * width]
                .chunks_exact_mut(width)
                .enumerate()
            {
                let y = top + row_index;
                let row_start = inv.tx + inv.kx * (y as f32 + 0.5);
                for (column, coverage) in row[left..right]
                    .iter_mut()
                    .enumerate()
                    .filter(|(_, n)| **n != 0)
                {
                    let x = left + column;
                    let t = (row_start + inv.sx * (x as f32 + 0.5)).clamp(0., 1.);
                    let alpha = f32::from(a) * (1. - t) + f32::from(b) * t;
                    *coverage = (f32::from(*coverage) * alpha / 255.).round() as u8;
                }
            }
        }
    }
    Ok(Some(mask))
}

#[cfg(test)]
#[test]
fn bounded_alpha_ramp_matches_full_mask_at_fractional_viewport_edges() {
    use document_core::{Document, Shape, Stencil};

    let mut doc = Document::default();
    doc.add_rectangle([0.2, 1.5], [16.3, 15.8], [255; 4]);
    let object = &mut doc.objects[0];
    object.stencil = Some(Stencil {
        shape: Shape::Ellipse,
        origin: [-0.15, 0.08],
        size: [1.; 2],
        fill_rule: Default::default(),
        alpha: None,
    });
    let surface = Pixmap::new(33, 25).unwrap();
    for transform in [
        Transform::from_translate(-7.6, -3.2),
        Transform::from_row(-1.21, 0.2, 0.35, 0.82, 43.6, 16.8),
        Transform::from_translate(35.2, 0.),
    ] {
        let mut expected = mask(&surface, object, 0, transform).unwrap().unwrap();
        let inverse = object
            .stencil_alpha_space(0)
            .unwrap()
            .post_concat(transform)
            .invert()
            .unwrap();
        for (i, coverage) in expected.data_mut().iter_mut().enumerate() {
            let x = (i % 33) as f32 + 0.5;
            let y = (i / 33) as f32 + 0.5;
            let t = (inverse.tx + inverse.kx * y + inverse.sx * x).clamp(0., 1.);
            *coverage = (f32::from(*coverage) * (30. * (1. - t) + 210. * t) / 255.).round() as u8;
        }
        object.stencil.as_mut().unwrap().alpha = Some([30, 210]);
        let actual = mask(&surface, object, 0, transform).unwrap().unwrap();
        assert_eq!(actual.data(), expected.data());
        object.stencil.as_mut().unwrap().alpha = None;
    }
}
