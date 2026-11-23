use crate::{geometry::path_contains_with_rule, FillRule, Shape, VectorObject};
use serde::{Deserialize, Serialize};
use tiny_skia_path::{Path, Point, Transform};

/// All paint coordinates are fractions of the tight, unrotated fill bounding box (SVG objectBoundingBox).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GradientKind {
    Linear { start: [f32; 2], end: [f32; 2] },
    Radial { center: [f32; 2], radius: f32 },
    Rectangular { center: [f32; 2], radius: f32 },
}

impl GradientKind {
    pub fn valid(&self) -> bool {
        match *self {
            Self::Linear { start, end } => {
                start != end && start.into_iter().chain(end).all(f32::is_finite)
            }
            Self::Radial { center, radius } | Self::Rectangular { center, radius } => {
                center.into_iter().all(f32::is_finite) && radius.is_finite() && radius > 0.
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    pub offset: f32,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
}

impl Gradient {
    /// Start with a visible swatch when the source was an unfilled shape.
    pub fn from_color(kind: GradientKind, mut color: [u8; 4]) -> Self {
        if color[3] == 0 {
            color[3] = 255;
        }
        Self {
            kind,
            stops: vec![
                GradientStop { offset: 0., color },
                GradientStop {
                    offset: 1.,
                    color: [color[0], color[1], color[2], 0],
                },
            ],
        }
    }

    pub fn valid(&self) -> bool {
        self.kind.valid()
            && (2..=256).contains(&self.stops.len())
            && self.stops.iter().all(|s| (0. ..=1.).contains(&s.offset))
            && self.stops.windows(2).all(|w| w[0].offset <= w[1].offset)
    }

    fn visible_in(&self, point: [f32; 2], space: Option<Transform>) -> bool {
        space.and_then(|space| space.invert()).is_some_and(|space| {
            let mut p = Point::from_xy(point[0], point[1]);
            space.map_point(&mut p);
            self.visible_at([p.x, p.y])
        })
    }

    pub fn visible_at(&self, p: [f32; 2]) -> bool {
        let t = match self.kind {
            GradientKind::Linear { start, end } => {
                let dx = end[0] - start[0];
                let dy = end[1] - start[1];
                ((p[0] - start[0]) * dx + (p[1] - start[1]) * dy) / (dx * dx + dy * dy)
            }
            GradientKind::Radial { center, radius } => {
                (p[0] - center[0]).hypot(p[1] - center[1]) / radius
            }
            GradientKind::Rectangular { center, radius } => {
                (p[0] - center[0]).abs().max((p[1] - center[1]).abs()) / radius
            }
        };
        let right = self.stops.partition_point(|s| s.offset <= t);
        let Some(left) = right.checked_sub(1).and_then(|i| self.stops.get(i)) else {
            return self.stops.first().is_some_and(|s| s.color[3] > 0);
        };
        left.color[3] > 0
            || (t > left.offset && self.stops.get(right).is_some_and(|s| s.color[3] > 0))
    }
}

/// Embedded silhouette. None means geometric clipping; an alpha pair fades horizontally
/// from left to right through the silhouette, including both the target fill and stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stencil {
    pub shape: Shape,
    #[serde(default, skip_serializing_if = "FillRule::is_nonzero")]
    pub fill_rule: FillRule,
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub alpha: Option<[u8; 2]>,
}

impl Stencil {
    /// Capture the upper object's evaluated silhouette in the lower object's paint box.
    pub fn capture(
        source: &VectorObject,
        target: &VectorObject,
        frame: u32,
        alpha: Option<[u8; 2]>,
    ) -> Option<Self> {
        Self::capture_outline(
            source.outline(frame)?,
            source.fill_rule,
            target.paint_space(frame)?,
            alpha,
        )
    }

    pub(super) fn capture_outline(
        path: Path,
        fill_rule: FillRule,
        target_space: Transform,
        alpha: Option<[u8; 2]>,
    ) -> Option<Self> {
        let path = path.transform(target_space.invert()?)?;
        let bounds = path.compute_tight_bounds()?.to_non_zero_rect()?;
        let origin = [bounds.x(), bounds.y()];
        let size = [bounds.width(), bounds.height()];
        let unit = |p: Point| [(p.x - origin[0]) / size[0], (p.y - origin[1]) / size[1]];
        let mut contours = crate::geometry::editable_contours(&path, unit)?;
        let shape = if contours.len() == 1 {
            Shape::Path {
                nodes: contours.pop()?,
                closed: true,
            }
        } else {
            Shape::Compound { contours }
        };
        let stencil = Self {
            shape,
            fill_rule,
            origin,
            size,
            alpha,
        };
        stencil.valid().then_some(stencil)
    }
    pub fn valid(&self) -> bool {
        matches!(
            self.shape,
            Shape::Ellipse
                | Shape::Rectangle
                | Shape::Path { closed: true, .. }
                | Shape::Compound { .. }
        ) && self.origin.iter().all(|n| n.is_finite())
            && self.size.iter().all(|&n| n.is_finite() && n > 0.)
            && self.shape.valid_nodes(self.size)
    }
    pub fn transform(&self, paint_space: Transform) -> Transform {
        Transform::from_row(
            self.size[0],
            0.,
            0.,
            self.size[1],
            self.origin[0],
            self.origin[1],
        )
        .post_concat(paint_space)
    }
    pub fn outline(&self, paint_space: Transform) -> Option<Path> {
        self.shape
            .local_outline([1.; 2])?
            .transform(self.transform(paint_space))
    }
}

impl VectorObject {
    pub fn paint_space(&self, frame: u32) -> Option<Transform> {
        self.paint_bounds_transform(frame, None)
    }

    /// The stroke follows the same centerline paint box as the fill. On an exactly
    /// horizontal or vertical path, the stroke's width supplies the missing dimension.
    pub fn stroke_paint_space(&self, frame: u32) -> Option<Transform> {
        self.paint_bounds_transform(frame, Some(self.stroke_width_at(frame)))
    }

    fn paint_bounds_transform(&self, frame: u32, stroke: Option<f32>) -> Option<Transform> {
        let (_, size) = self.scaled_box_at(frame);
        let outline = self.evaluated_shape_at(frame).local_outline(size)?;
        let bounds = outline
            .compute_tight_bounds()
            .unwrap_or_else(|| outline.bounds());
        // A horizontal or vertical open path can still paint a stroke. Borrow
        // the object's authored extent on the degenerate axis for its clip box.
        let axis = |start, extent, authored| {
            if extent > 0. {
                (start, extent)
            } else if let Some(width) = stroke.filter(|w| *w > 0.) {
                (start - width / 2., width)
            } else {
                (start, authored)
            }
        };
        let (x, width) = axis(bounds.x(), bounds.width(), size[0]);
        let (y, height) = axis(bounds.y(), bounds.height(), size[1]);
        Some(
            Transform::from_row(width, 0., 0., height, x, y).post_concat(self.box_transform(frame)),
        )
    }
    pub fn has_fill(&self) -> bool {
        self.has_fill_at(0)
    }
    pub fn has_fill_at(&self, frame: u32) -> bool {
        if let Some(image) = &self.image {
            return image.has_ink();
        }
        if self.no_fill {
            return false;
        }
        self.gradient
            .as_ref()
            .map_or(self.fill_at(frame)[3] > 0, |g| {
                g.stops.iter().any(|s| s.color[3] > 0)
            })
    }
    pub(crate) fn fill_visible_at(&self, point: [f32; 2], frame: u32) -> bool {
        if let Some(image) = &self.image {
            return image.visible_at(self.world_to_local(point, frame));
        }
        if self.no_fill {
            return false;
        }
        let Some(gradient) = &self.gradient else {
            return self.fill_at(frame)[3] > 0;
        };
        gradient.visible_in(point, self.paint_space(frame))
    }

    pub fn has_stroke_at(&self, frame: u32) -> bool {
        !self.no_stroke
            && self.stroke_width_at(frame) > 0.
            && self.stroke_gradient.as_ref().map_or_else(
                || self.stroke_at(frame)[3] > 0,
                |g| g.stops.iter().any(|s| s.color[3] > 0),
            )
    }

    pub(crate) fn stroke_visible_at(&self, point: [f32; 2], frame: u32) -> bool {
        !self.no_stroke
            && self.stroke_width_at(frame) > 0.
            && self.stroke_gradient.as_ref().map_or_else(
                || self.stroke_at(frame)[3] > 0,
                |g| g.visible_in(point, self.stroke_paint_space(frame)),
            )
    }
    pub fn stencil_contains(&self, point: [f32; 2], frame: u32) -> bool {
        let Some(stencil) = &self.stencil else {
            return true;
        };
        self.paint_space(frame).is_some_and(|s| {
            if let Some([a, b]) = stencil.alpha {
                let Some(inv) = self
                    .stencil_alpha_space(frame)
                    .and_then(|space| space.invert())
                else {
                    return false;
                };
                let mut p = Point::from_xy(point[0], point[1]);
                inv.map_point(&mut p);
                let t = p.x.clamp(0., 1.);
                if f32::from(a) * (1. - t) + f32::from(b) * t <= 0. {
                    return false;
                }
            }
            stencil
                .outline(s)
                .is_some_and(|path| path_contains_with_rule(&path, point, stencil.fill_rule))
        })
    }
    pub(crate) fn validate_appearance(&self) -> Result<(), String> {
        if self.stencil.as_ref().is_some_and(|s| !s.valid())
            || self.gradient.as_ref().is_some_and(|g| !g.valid())
            || self.stroke_gradient.as_ref().is_some_and(|g| !g.valid())
        {
            return Err(format!(
                "Object {} has an invalid gradient or mask.",
                self.id
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn a_new_gradient_fades_the_source_color_even_on_an_unfilled_path() {
        let kind = GradientKind::Rectangular {
            center: [0.5; 2],
            radius: 0.5,
        };
        for (source, start) in [(0, 255), (87, 87), (255, 255)] {
            let gradient = Gradient::from_color(kind.clone(), [10, 20, 30, source]);
            assert!(gradient.valid());
            assert_eq!(gradient.stops[0].color, [10, 20, 30, start]);
            assert_eq!(gradient.stops[1].color, [10, 20, 30, 0]);
        }
    }

    #[test]
    fn capturing_a_rotated_silhouette_preserves_its_world_geometry_and_follows_the_target() {
        let mut doc = Document::default();
        let target = doc.add_rectangle([20., 20.], [40., 20.], [255; 4]);
        doc.add_shape([30., 20.], [15., 25.], [255; 4], Shape::Ellipse, 1);
        doc.objects[0].rotation = 21.;
        doc.objects[1].rotation = 48.;
        let sample = |p| doc.objects[1].local_to_world(p, 0);
        let inside = sample([0.5, 0.5]);
        let outside = sample([0.03, 0.03]);
        let clip = Stencil::capture(&doc.objects[1], &doc.objects[0], 0, None).unwrap();
        doc.objects[0].stencil = Some(clip);
        doc.objects.pop();
        assert_eq!(doc.hit_test(inside, 0), Some(target));
        assert_eq!(doc.hit_test(outside, 0), None);
        doc.objects[0]
            .keys
            .extend([(0, [20., 20.]), (4, [45., 20.])]);
        assert_eq!(doc.hit_test([inside[0] + 25., inside[1]], 4), Some(target));
    }

    #[test]
    fn malformed_stops_and_stencil_bounds_are_rejected_before_export() {
        let mut doc = Document::default();
        doc.add_rectangle([1.; 2], [8.; 2], [255; 4]);
        doc.objects[0].gradient = Some(Gradient {
            kind: GradientKind::Radial {
                center: [0.5; 2],
                radius: 0.5,
            },
            stops: vec![
                GradientStop {
                    offset: 1.,
                    color: [255; 4],
                },
                GradientStop {
                    offset: 0.,
                    color: [0; 4],
                },
            ],
        });
        assert!(doc.validate().is_err());
        doc.objects[0].stroke_gradient = doc.objects[0].gradient.take();
        assert!(doc.validate().is_err());
        doc.objects[0].stroke_gradient = None;
        doc.objects[0].stencil = Some(Stencil {
            fill_rule: FillRule::NonZero,
            shape: Shape::Rectangle,
            origin: [f32::NAN, 0.],
            size: [1.; 2],
            alpha: Some([255, 0]),
        });
        assert!(doc.validate().is_err());
    }
}
