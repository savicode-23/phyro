use super::{LineCap, LineJoin, PathNode, Shape, VectorObject};
use tiny_skia_path::{Path, PathBuilder, PathSegment, Rect, Stroke, Transform};

type Point = [f64; 2];
type Cubic = [Point; 4];

/// Convert evaluated outline segments into editable cubic anchors and handles.
/// The caller controls the coordinate frame. Quadratic segments are degree-elevated.
pub(crate) fn editable_contours(
    path: &Path,
    unit: impl Fn(tiny_skia_path::Point) -> [f32; 2],
) -> Option<Vec<Vec<PathNode>>> {
    let mut contours: Vec<Vec<PathNode>> = Vec::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => contours.push(vec![PathNode::corner(unit(p))]),
            PathSegment::LineTo(p) => contours.last_mut()?.push(PathNode::corner(unit(p))),
            PathSegment::QuadTo(c, p) => {
                let nodes = contours.last_mut()?;
                let last = nodes.last_mut()?;
                let c = unit(c);
                let end = unit(p);
                last.outgoing =
                    std::array::from_fn(|i| last.point[i] + (c[i] - last.point[i]) * 2. / 3.);
                nodes.push(PathNode {
                    handle_style: Default::default(),
                    point: end,
                    outgoing: end,
                    incoming: std::array::from_fn(|i| end[i] + (c[i] - end[i]) * 2. / 3.),
                });
            }
            PathSegment::CubicTo(c, d, p) => {
                let nodes = contours.last_mut()?;
                nodes.last_mut()?.outgoing = unit(c);
                let end = unit(p);
                nodes.push(PathNode {
                    handle_style: Default::default(),
                    point: end,
                    incoming: unit(d),
                    outgoing: end,
                });
            }
            PathSegment::Close => {
                let nodes = contours.last_mut()?;
                if nodes.len() > 1 && nodes.last()?.point == nodes[0].point {
                    nodes[0].incoming = nodes.pop()?.incoming;
                }
            }
        }
    }
    Some(contours)
}

impl Shape {
    /// Bounds enclosing the size box and every transformed Bézier control point.
    pub fn control_bounds(&self, [w, h]: [f32; 2], transform: Transform) -> Option<Rect> {
        let [mut l, mut t, mut r, mut b] = [0., 0., w, h];
        for n in self.contours().iter().flatten() {
            for [u, v] in [n.point, n.incoming, n.outgoing] {
                let (x, y) = (u * w, v * h);
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
        }
        Rect::from_ltrb(l, t, r, b)?;
        if !transform.has_scale() && !transform.has_skew() {
            return Rect::from_ltrb(
                l + transform.tx,
                t + transform.ty,
                r + transform.tx,
                b + transform.ty,
            );
        }
        let mut corners =
            [[l, t], [r, t], [r, b], [l, b]].map(|[x, y]| tiny_skia_path::Point::from_xy(x, y));
        transform.map_points(&mut corners);
        Rect::from_points(&corners)
    }

    /// Build a path in an unrotated local pixel frame, with its origin at (0, 0).
    pub fn local_outline(&self, [w, h]: [f32; 2]) -> Option<Path> {
        match self {
            Self::Rectangle => Some(PathBuilder::from_rect(Rect::from_xywh(0., 0., w, h)?)),
            Self::Ellipse => PathBuilder::from_oval(Rect::from_xywh(0., 0., w, h)?),
            Self::Path { .. } | Self::Compound { .. } => {
                let closed = !matches!(self, Self::Path { closed: false, .. });
                let scaled = |p: [f32; 2]| [p[0] * w, p[1] * h];
                let mut builder = PathBuilder::new();
                for nodes in self.contours() {
                    let first = nodes.first()?;
                    let [x, y] = scaled(first.point);
                    builder.move_to(x, y);
                    let mut segment = |a: &PathNode, b: &PathNode| {
                        let [x, y] = scaled(b.point);
                        if a.outgoing == a.point && b.incoming == b.point {
                            builder.line_to(x, y);
                        } else {
                            let [cx, cy] = scaled(a.outgoing);
                            let [qx, qy] = scaled(b.incoming);
                            builder.cubic_to(cx, cy, qx, qy, x, y);
                        }
                    };
                    for pair in nodes.windows(2) {
                        segment(&pair[0], &pair[1]);
                    }
                    if closed {
                        segment(nodes.last()?, first);
                        builder.close();
                    }
                }
                builder.finish()
            }
        }
    }
}

impl VectorObject {
    /// Unrotated display box, centered at the evaluated pose's visual center.
    pub fn scaled_box_at(&self, frame: u32) -> ([f32; 2], [f32; 2]) {
        self.box_with_scale(frame, self.scale_at(frame).map(f32::abs))
    }

    pub(crate) fn signed_box_at(&self, frame: u32) -> ([f32; 2], [f32; 2]) {
        self.box_with_scale(frame, self.scale_at(frame))
    }

    fn box_with_scale(&self, frame: u32, scale: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let position = self.position_at(frame);
        let size = std::array::from_fn(|axis| self.size[axis] * scale[axis]);
        let origin =
            std::array::from_fn(|axis| position[axis] + (self.size[axis] - size[axis]) * 0.5);
        (origin, size)
    }

    /// Convert normalized local coordinates to artboard pixels in the evaluated pose.
    pub fn local_to_world(&self, point: [f32; 2], frame: u32) -> [f32; 2] {
        self.local_to_world_at(frame)(point)
    }

    /// Sample the transform once when converting multiple points at the same frame.
    pub fn local_to_world_at(&self, frame: u32) -> impl Fn([f32; 2]) -> [f32; 2] {
        let ([x, y], [w, h]) = self.signed_box_at(frame);
        let (s, c) = self.rotation_at(frame).to_radians().sin_cos();
        let shear = self.skew_at(frame).to_radians().tan();
        move |[u, v]| {
            let dy = (v - 0.5) * h;
            let dx = (u - 0.5) * w + shear * dy;
            [
                x + (0.5 * w + c * dx - s * dy),
                y + (0.5 * h + s * dx + c * dy),
            ]
        }
    }

    pub fn world_to_local(&self, point: [f32; 2], frame: u32) -> [f32; 2] {
        self.world_to_local_at(frame)(point)
    }

    /// Evaluate the inverse transform once when converting a whole pose to local coordinates.
    pub fn world_to_local_at(&self, frame: u32) -> impl Fn([f32; 2]) -> [f32; 2] {
        let ([x, y], [w, h]) = self.signed_box_at(frame);
        let (s, c) = self.rotation_at(frame).to_radians().sin_cos();
        let shear = self.skew_at(frame).to_radians().tan();
        move |point| {
            let (dx, dy) = (point[0] - x - w * 0.5, point[1] - y - h * 0.5);
            let local_y = -s * dx + c * dy;
            [
                (c * dx + s * dy - shear * local_y) / w + 0.5,
                local_y / h + 0.5,
            ]
        }
    }

    /// Shared geometry for painting, bounding boxes and hit testing. Coordinates
    /// are already in artboard pixels so object scaling leaves stroke widths intact.
    pub fn outline(&self, frame: u32) -> Option<Path> {
        let (size, transform) = self.outline_transform_at(frame);
        self.evaluated_shape_at(frame)
            .local_outline(size)?
            .transform(transform)
    }

    pub fn box_transform(&self, frame: u32) -> Transform {
        self.outline_transform_at(frame).1
    }

    /// Local pixel dimensions and the matrix placing those pixels on the artboard.
    /// Both come from the same evaluated position, pivot and signed scale.
    pub fn outline_transform_at(&self, frame: u32) -> ([f32; 2], Transform) {
        let ([x, y], [w, h]) = self.signed_box_at(frame);
        let size = [w.abs(), h.abs()];
        let rotation = self.rotation_at(frame);
        let skew = self.skew_at(frame);
        if rotation == 0. && skew == 0. {
            return (
                size,
                Transform::from_row(w.signum(), 0., 0., h.signum(), x, y),
            );
        }
        let shear = skew.to_radians().tan();
        let transform = Transform::from_row(
            w.signum(),
            0.,
            shear * h.signum(),
            h.signum(),
            -shear * h * 0.5,
            0.,
        )
        .post_concat(Transform::from_rotate_at(rotation, w * 0.5, h * 0.5))
        .post_translate(x, y);
        (size, transform)
    }

    pub fn stroke_style(&self) -> Stroke {
        self.stroke_style_at(0)
    }
    pub fn stroke_style_at(&self, frame: u32) -> Stroke {
        Stroke {
            width: self.stroke_width_at(frame),
            line_cap: match self.cap {
                LineCap::Butt => tiny_skia_path::LineCap::Butt,
                LineCap::Round => tiny_skia_path::LineCap::Round,
                LineCap::Square => tiny_skia_path::LineCap::Square,
            },
            line_join: match self.join {
                LineJoin::Miter => tiny_skia_path::LineJoin::Miter,
                LineJoin::Round => tiny_skia_path::LineJoin::Round,
                LineJoin::Bevel => tiny_skia_path::LineJoin::Bevel,
            },
            miter_limit: 4.0,
            ..Default::default()
        }
    }

    /// Stroke geometry includes fully transparent paint. Picking checks paint visibility first.
    pub(crate) fn stroked(&self, outline: &Path, frame: u32) -> Option<Path> {
        if !self.no_stroke && self.stroke_width_at(frame) > 0. {
            if self.stroke_profile.is_empty() {
                outline.stroke(&self.stroke_style_at(frame), 1.)
            } else {
                self.pressure_outline(outline, frame)
            }
        } else {
            None
        }
    }

    /// Editing bounds include an enabled stroke's width even when its paint is transparent.
    pub fn bounds(&self, frame: u32) -> [f32; 4] {
        if let Some((art, child_frame)) = self.instance_at(frame) {
            let [w, h] = self.placement_source_size(art);
            let mut ext: [f32; 4] = [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ];
            for child in art
                .paint_order()
                .into_iter()
                .filter(|o| o.visible_at(child_frame) && o.opacity_at(child_frame) > 0.)
            {
                let [mut x, mut y, mut r, mut b] = child.bounds(child_frame);
                if self.clip_instance.is_some() {
                    x = x.max(0.);
                    y = y.max(0.);
                    r = r.min(art.width as f32);
                    b = b.min(art.height as f32);
                    if x >= r || y >= b {
                        continue;
                    }
                }
                for [px, py] in [[x, y], [x, b], [r, y], [r, b]] {
                    let pt = self.local_to_world([px / w, py / h], frame);
                    ext = [
                        ext[0].min(pt[0]),
                        ext[1].min(pt[1]),
                        ext[2].max(pt[0]),
                        ext[3].max(pt[1]),
                    ];
                }
            }
            if ext[0].is_finite() {
                return ext;
            }
            let [x, y] = self.local_to_world([0., 0.], frame);
            return [x, y, x, y];
        }
        let [x, y] = self.position_at(frame);
        let Some(path) = self.outline(frame) else {
            return [x, y, x, y];
        };
        self.outline_bounds(&path, frame, true)
    }

    /// Whether the evaluated bounds, including stroke, fit inside the inclusive
    /// artboard rectangle `[left, top, right, bottom]`.
    pub fn bounds_within(&self, [left, top, right, bottom]: [f32; 4], frame: u32) -> bool {
        let inside = |[x, y]: [f32; 2]| left <= x && x <= right && top <= y && y <= bottom;
        let bounds = if self.is_instance() {
            self.bounds(frame)
        } else {
            let Some(path) = self.outline(frame) else {
                return inside(self.position_at(frame));
            };
            // The first point belongs to the tight bounds even when the path is stroked.
            let first = path.points()[0];
            if self.stroke_profile.is_empty() && !inside([first.x, first.y]) {
                return false;
            }
            self.outline_bounds(&path, frame, true)
        };
        inside([bounds[0], bounds[1]]) && inside([bounds[2], bounds[3]])
    }

    /// Extent of the painted geometry of a single shape for tight export crops.
    /// Placed artwork measures its children in their own drawing.
    pub fn painted_shape_bounds(&self, frame: u32) -> Option<[f32; 4]> {
        if self.is_instance() {
            return None;
        }
        let stroke = self.has_stroke_at(frame);
        if !stroke && !self.has_fill_at(frame) {
            return None;
        }
        Some(self.outline_bounds(&self.outline(frame)?, frame, stroke))
    }

    fn outline_bounds(&self, path: &Path, frame: u32, include_stroke: bool) -> [f32; 4] {
        let stroked = if include_stroke {
            self.stroked(path, frame)
        } else {
            None
        };
        if !self.stroke_profile.is_empty() && !self.has_fill_at(frame) {
            if let Some(ink) = &stroked {
                let b = ink.compute_tight_bounds().unwrap_or_else(|| ink.bounds());
                return [b.left(), b.top(), b.right(), b.bottom()];
            }
        }
        let base = path.compute_tight_bounds().unwrap_or_else(|| path.bounds());
        let mut result = [base.left(), base.top(), base.right(), base.bottom()];
        if let Some(stroked) = stroked {
            let b = stroked
                .compute_tight_bounds()
                .unwrap_or_else(|| stroked.bounds());
            result = [
                result[0].min(b.left()),
                result[1].min(b.top()),
                result[2].max(b.right()),
                result[3].max(b.bottom()),
            ];
        }
        result
    }

    pub(crate) fn contains(&self, point: [f32; 2], frame: u32) -> bool {
        if !self.visible_at(frame) {
            return false;
        }
        if let Some((art, child_frame)) = self.instance_at(frame) {
            if self.opacity_at(frame) <= 0. || !self.stencil_contains(point, frame) {
                return false;
            }
            let [u, v] = self.world_to_local(point, frame);
            let [w, h] = self.placement_source_size(art);
            let local = [u * w, v * h];
            if self.clip_instance.is_some()
                && (!(0.0..art.width as f32).contains(&local[0])
                    || !(0.0..art.height as f32).contains(&local[1]))
            {
                return false;
            }
            return art
                .paint_order()
                .iter()
                .rev()
                .any(|o| o.contains(local, child_frame));
        }
        // Bound small authored paths before allocating their outlines. Large or
        // deformed paths use one exact evaluation even when the pointer is close.
        if matches!(&self.shape, Shape::Path { nodes, .. } if nodes.len() <= 64)
            && self.skin.is_none()
            && self.effects.is_empty()
            && self.animated.geometry_keys.is_empty()
        {
            let (size, placement) = self.outline_transform_at(frame);
            if self
                .shape
                .control_bounds(size, placement)
                .is_some_and(|b| far_from_outline(b, point, &self.stroke_style_at(frame)))
            {
                return false;
            }
        }
        if !self.stencil_contains(point, frame) {
            return false;
        }
        let filled = self.fill_visible_at(point, frame);
        let stroked = self.stroke_visible_at(point, frame);
        if self.opacity_at(frame) <= 0. || (!filled && !stroked) {
            return false;
        }
        if filled && self.shape == Shape::Rectangle {
            let [u, v] = self.world_to_local(point, frame);
            if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                return true;
            }
            if !stroked {
                return false;
            }
        }
        let Some(path) = self.outline(frame) else {
            return false;
        };
        if filled && path_contains_with_rule(&path, point, self.fill_rule) {
            return true;
        }
        if !stroked {
            return false;
        }
        let stroke = self.stroke_style_at(frame);
        if far_from_outline(path.bounds(), point, &stroke) {
            return false;
        }
        self.stroked(&path, frame)
            .is_some_and(|outline| path_contains(&outline, point))
    }
}

// Allow a full stroke width per miter-limit unit and room for square caps
// before rejecting a distant point without constructing the stroked path.
fn far_from_outline(bounds: Rect, [x, y]: [f32; 2], stroke: &Stroke) -> bool {
    let pad = 1. + stroke.width * stroke.miter_limit.max(2.);
    x < bounds.left() - pad
        || x > bounds.right() + pad
        || y < bounds.top() - pad
        || y > bounds.bottom() + pad
}

/// Nonzero winding for any number of contours. Subdivision is in world pixels,
/// with subpixel error and pruning by the control polygon on each ray crossing.
pub(crate) fn path_contains(path: &Path, point: [f32; 2]) -> bool {
    path_contains_with_rule(path, point, crate::FillRule::NonZero)
}

pub(crate) fn path_contains_with_rule(path: &Path, point: [f32; 2], rule: crate::FillRule) -> bool {
    let bounds = path.bounds();
    if point[0] < bounds.left()
        || point[1] < bounds.top()
        || point[0] > bounds.right()
        || point[1] > bounds.bottom()
    {
        return false;
    }
    let point = point.map(f64::from);
    let world = |p: tiny_skia_path::Point| [f64::from(p.x), f64::from(p.y)];
    let error = 0.015_f64.min(f64::from(bounds.width().max(bounds.height())) / 1024.0);
    let (mut start, mut last) = ([0.0; 2], [0.0; 2]);
    let mut winding = 0;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                winding += crossing(last, start, point);
                start = world(p);
                last = start;
            }
            PathSegment::LineTo(p) => {
                let end = world(p);
                winding += crossing(last, end, point);
                last = end;
            }
            PathSegment::QuadTo(q, p) => {
                let q = world(q);
                let end = world(p);
                let c1 = std::array::from_fn(|i| (last[i] + 2.0 * q[i]) / 3.0);
                let c2 = std::array::from_fn(|i| (end[i] + 2.0 * q[i]) / 3.0);
                winding += curve_crossings([last, c1, c2, end], point, error * error, 0);
                last = end;
            }
            PathSegment::CubicTo(a, b, p) => {
                let end = world(p);
                winding +=
                    curve_crossings([last, world(a), world(b), end], point, error * error, 0);
                last = end;
            }
            PathSegment::Close => {
                winding += crossing(last, start, point);
                last = start;
            }
        }
    }
    // A fill implicitly closes the last open subpath.
    winding += crossing(last, start, point);
    match rule {
        crate::FillRule::NonZero => winding != 0,
        crate::FillRule::EvenOdd => winding % 2 != 0,
    }
}

fn crossing(a: Point, b: Point, p: Point) -> i32 {
    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
    if a[1] <= p[1] && b[1] > p[1] && cross > 0.0 {
        1
    } else if a[1] > p[1] && b[1] <= p[1] && cross < 0.0 {
        -1
    } else {
        0
    }
}

fn distance_squared_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0.0, 1.0)
    };
    (p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)
}

fn midpoint(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn curve_crossings(curve: Cubic, point: Point, error_sq: f64, depth: u32) -> i32 {
    if curve.iter().all(|p| p[1] <= point[1])
        || curve.iter().all(|p| p[1] > point[1])
        || curve.iter().all(|p| p[0] < point[0])
    {
        return 0;
    }
    let [a, b, c, d] = curve;
    if depth >= 14
        || (distance_squared_to_segment(b, a, d) <= error_sq
            && distance_squared_to_segment(c, a, d) <= error_sq)
    {
        return crossing(a, d, point);
    }
    let ab = midpoint(a, b);
    let bc = midpoint(b, c);
    let cd = midpoint(c, d);
    let abc = midpoint(ab, bc);
    let bcd = midpoint(bc, cd);
    let center = midpoint(abc, bcd);
    curve_crossings([a, ab, abc, center], point, error_sq, depth + 1)
        + curve_crossings([center, bcd, cd, d], point, error_sq, depth + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, Layer};

    #[test]
    fn outline_placement_matches_normalized_coordinates_with_pivot_and_reflection() {
        let mut doc = Document::default();
        doc.add_rectangle([15., 42.], [18., 11.], [255; 4]);
        let object = &mut doc.objects[0];
        object.pivot = Some([1.2, -0.4]);
        object.keys.extend([(0, [15., 42.]), (4, [60., 23.])]);
        for (scale, rotation, skew) in [
            ([1., 1.], 0., 0.),
            ([-2., 1.5], 0., 0.),
            ([1.4, -0.8], -63., 24.),
        ] {
            object.scale = scale;
            object.rotation = rotation;
            object.animated.skew = skew;
            for frame in [0, 2, 4] {
                let (size, transform) = object.outline_transform_at(frame);
                for uv in [[0., 0.], [1., 1.], [-0.25, 0.77]] {
                    let mut point =
                        tiny_skia_path::Point::from_xy(uv[0] * size[0], uv[1] * size[1]);
                    transform.map_point(&mut point);
                    let [x, y] = object.local_to_world(uv, frame);
                    assert!((point.x - x).abs() < 1e-4 && (point.y - y).abs() < 1e-4);
                }
            }
        }
    }

    #[test]
    fn validates_node_count_across_the_whole_compound_and_each_contour() {
        let mut doc = Document::default();
        doc.add_shape(
            [0.; 2],
            [10.; 2],
            [255; 4],
            Shape::Compound {
                contours: Vec::new(),
            },
            1,
        );
        assert!(doc.validate().is_err());
        let n = PathNode::corner([0.; 2]);
        doc.objects[0].shape = Shape::Compound {
            contours: vec![vec![n; 2], vec![n]],
        };
        assert!(doc.validate().is_err());
        doc.objects[0].shape = Shape::Compound {
            contours: vec![vec![n; crate::MAX_PATH_NODES / 2]; 2],
        };
        assert!(doc.validate().is_ok());
        doc.objects[0].shape.contours_mut()[1].push(n);
        assert!(doc.validate().is_err());
    }

    #[test]
    fn pick_follows_layer_order_visibility_and_lock_state() {
        let mut doc = Document::default();
        doc.layers.push(Layer {
            id: 2,
            ..Layer::default()
        });
        let upper = doc.add_shape([0.; 2], [20.; 2], [255; 4], Shape::Ellipse, 2);
        let lower = doc.add_rectangle([0.; 2], [20.; 2], [255; 4]);
        assert_eq!(doc.hit_test([1., 1.], 0), Some(lower));
        assert_eq!(doc.hit_test([10., 10.], 0), Some(upper));
        doc.layers[1].locked = true;
        assert_eq!(doc.hit_test([10., 10.], 0), Some(lower));
        assert_eq!(doc.paint_order().last().unwrap().id, upper);
        doc.layers[1].visible = false;
        assert_eq!(doc.paint_order().len(), 1);
        doc.layers[1].visible = true;
        doc.layers[1].locked = false;
        doc.layers.reverse();
        assert_eq!(doc.hit_test([10., 10.], 0), Some(lower));
    }

    #[test]
    fn rotation_uses_object_center_at_evaluated_keyframe() {
        let mut doc = Document::default();
        doc.add_rectangle([0., 0.], [20., 10.], [255; 4]);
        let object = &mut doc.objects[0];
        object.rotation = 90.;
        object.keys.insert(1, [10., 20.]);
        let world = object.local_to_world([0.25, 0.25], 1);
        assert!((world[0] - 22.5).abs() < 1e-4 && (world[1] - 20.).abs() < 1e-4);
        for value in object.world_to_local(world, 1) {
            assert!((value - 0.25).abs() < 1e-4);
        }
        let bounds = object.bounds(1);
        for (actual, expected) in bounds.iter().zip([15., 15., 25., 35.]) {
            assert!((actual - expected).abs() < 1e-4);
        }
        assert!(object.contains([20., 16.], 1));
        assert!(!object.contains([12., 20.], 1));
    }

    #[test]
    fn curve_bounds_use_drawn_extrema_and_open_path_fill_closes() {
        let mut doc = Document::default();
        let mut a = PathNode::corner([0., 1.]);
        let mut b = PathNode::corner([1., 1.]);
        a.outgoing = [0., 0.];
        b.incoming = [1., 0.];
        doc.add_shape(
            [0.; 2],
            [100.; 2],
            [255; 4],
            Shape::Path {
                nodes: vec![a, b],
                closed: false,
            },
            1,
        );
        let obj = &doc.objects[0];
        assert!((obj.bounds(0)[1] - 25.).abs() < 1e-4);
        assert!(obj.contains([50., 35.], 0));
        assert!(!obj.contains([50., 20.], 0));
    }

    #[test]
    fn painted_stroke_envelope_observes_caps_and_joins() {
        let mut doc = Document::default();
        doc.add_shape(
            [10., 10.],
            [20., 10.],
            [0; 4],
            Shape::Path {
                nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([1., 0.])],
                closed: false,
            },
            1,
        );
        let obj = &mut doc.objects[0];
        obj.stroke_width = 8.;
        obj.cap = LineCap::Butt;
        assert!(!obj.contains([8., 10.], 0));
        assert_eq!(obj.bounds(0), [10., 6., 30., 14.]);
        obj.cap = LineCap::Round;
        assert!(obj.contains([8., 10.], 0));
        assert!(!obj.contains([6.5, 6.5], 0));
        obj.cap = LineCap::Square;
        assert!(obj.contains([6.5, 6.5], 0));
        obj.shape = Shape::Rectangle;
        obj.join = LineJoin::Miter;
        assert!(obj.contains([6.5, 6.5], 0));
        obj.join = LineJoin::Bevel;
        assert!(!obj.contains([6.5, 6.5], 0));
        assert!(!obj.contains([20., 15.], 0));
    }

    #[test]
    fn control_bound_rejection_matches_evaluated_path_picking_at_each_pose() {
        let mut doc = Document::default();
        let mut nodes = [[-1.4, 0.2], [0.4, 1.7], [0.6, -0.5], [1.2, 0.6]].map(PathNode::corner);
        nodes[0].outgoing = [-2.8, -1.6];
        nodes[1].incoming = [1.3, 0.1];
        nodes[3].outgoing = [2.5, 1.8];
        let shape = Shape::Path {
            nodes: nodes.to_vec(),
            closed: true,
        };
        doc.add_shape([51., 45.], [23., 17.], [255; 4], shape.clone(), 1);
        let o = &mut doc.objects[0];
        o.keys.extend([(0, [51., 45.]), (3, [28., 23.])]);
        o.rotation_keys.extend([(0, -28.), (3, 58.)]);
        o.scale_keys.extend([(0, [0.75, 1.2]), (3, [1.6, 0.7])]);
        o.animated.skew = 27.;
        o.stroke_width = 3.3;
        o.cap = LineCap::Square;
        o.join = LineJoin::Miter;
        let mut evaluated = o.clone();
        // Identical authored geometry on a morph track always takes the full path.
        evaluated.animated.geometry_keys.insert(0, shape);
        for frame in 0..4 {
            let b = o.outline(frame).unwrap().bounds();
            let hull = o
                .shape
                .control_bounds(o.scaled_box_at(frame).1, o.box_transform(frame))
                .unwrap();
            assert!(
                hull.left() <= b.left()
                    && hull.top() <= b.top()
                    && hull.right() >= b.right()
                    && hull.bottom() >= b.bottom()
            );
            for x in (-20..125).step_by(5) {
                for y in (-20..125).step_by(5) {
                    let p = [x as f32 + 0.25, y as f32 + 0.75];
                    assert_eq!(
                        o.contains(p, frame),
                        evaluated.contains(p, frame),
                        "{p:?}, {frame}"
                    );
                }
            }
        }
    }

    #[test]
    fn picking_a_transformed_stroke_matches_its_full_outline_in_each_pose() {
        let mut doc = Document::default();
        let arm = doc.add_bone("Arm", None, [20., 35.], [43., 42.]).unwrap();
        let wrist = doc
            .add_bone("Wrist", Some(arm), [43., 42.], [72., 37.])
            .unwrap();
        let mut nodes = [
            [-0.5, 0.6],
            [0.55, 0.08],
            [0.69, 0.9],
            [0.83, 0.04],
            [1.1, 0.6],
        ]
        .map(PathNode::corner);
        nodes[0].outgoing = [-1.3, -0.65];
        nodes[1].incoming = [0.16, -0.55];
        let id = doc.add_shape(
            [26., 28.],
            [41., 27.],
            [0; 4],
            Shape::Path {
                nodes: nodes.to_vec(),
                closed: false,
            },
            1,
        );
        doc.objects[0].rotation = 21.;
        doc.objects[0].animated.skew = -19.;
        doc.objects[0].scale = [1.3, 0.78];
        doc.bind_skin_at(id, wrist, 0).unwrap();
        doc.set_bone_rotation_key(wrist, 0, 0.).unwrap();
        doc.set_bone_rotation_key(wrist, 4, 54.).unwrap();
        for (frame, cap, join, width) in [
            (0, LineCap::Square, LineJoin::Miter, 7.4),
            (2, LineCap::Round, LineJoin::Round, 2.3),
            (4, LineCap::Butt, LineJoin::Bevel, 4.1),
        ] {
            let obj = &mut doc.objects[0];
            obj.cap = cap;
            obj.join = join;
            obj.stroke_width = width;
            let path = obj.outline(frame).unwrap();
            let painted = path.stroke(&obj.stroke_style_at(frame), 1.).unwrap();
            let b = painted.bounds();
            let mut outside_box = false;
            for x in (b.left().floor() as i32 - 4..=b.right().ceil() as i32 + 4).step_by(2) {
                for y in (b.top().floor() as i32 - 4..=b.bottom().ceil() as i32 + 4).step_by(2) {
                    let point = [x as f32 + 0.35, y as f32 + 0.71];
                    let covered = path_contains(&painted, point);
                    assert_eq!(
                        doc.hit_test(point, frame),
                        covered.then_some(id),
                        "{point:?} at {frame}"
                    );
                    outside_box |= covered
                        && doc.objects[0]
                            .world_to_local(point, frame)
                            .iter()
                            .any(|v| !(0.0..=1.).contains(v));
                }
            }
            assert!(
                outside_box,
                "covered points should extend beyond the transform frame"
            );
        }
    }
}
