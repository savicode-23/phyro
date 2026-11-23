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
    if let Some(space) = object.paint_space(frame) {
        let pixels = space.post_concat(transform);
        if let Some(path) = stencil.outline(pixels) {
            let rule = match stencil.fill_rule {
                document_core::FillRule::NonZero => FillRule::Winding,
                document_core::FillRule::EvenOdd => FillRule::EvenOdd,
            };
            mask.fill_path(&path, rule, true, Transform::identity());
        }
        if let Some([a, b]) = stencil.alpha {
            if let Some(inv) = object
                .stencil_alpha_space(frame)
                .and_then(|space| space.post_concat(transform).invert())
            {
                for (y, row) in mask
                    .data_mut()
                    .chunks_exact_mut(surface.width() as usize)
                    .enumerate()
                {
                    let row_start = inv.tx + inv.kx * (y as f32 + 0.5);
                    for (x, coverage) in row.iter_mut().enumerate().filter(|(_, n)| **n != 0) {
                        let t = (row_start + inv.sx * (x as f32 + 0.5)).clamp(0., 1.);
                        let alpha = f32::from(a) * (1. - t) + f32::from(b) * t;
                        *coverage = (f32::from(*coverage) * alpha / 255.).round() as u8;
                    }
                }
            }
        }
    }
    Ok(Some(mask))
}
