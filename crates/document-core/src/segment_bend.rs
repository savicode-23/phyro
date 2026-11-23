use crate::{path_edit::HandleStyle, PathNode, Shape, VectorObject};
use std::borrow::Cow;

impl VectorObject {
    /// Bend one segment of the displayed pose by its press-to-pointer artboard
    /// displacement. `start` identifies its outgoing node in the chosen contour.
    /// The returned source geometry can be committed using the normal geometry-key policy.
    pub fn bent_segment(
        &self,
        frame: u32,
        contour: usize,
        start: usize,
        t: f32,
        world_delta: [f32; 2],
    ) -> Option<Shape> {
        if !(0. < t && t < 1.)
            || world_delta == [0.; 2]
            || !world_delta.into_iter().all(f32::is_finite)
            || self.text.is_some()
            || self.image.is_some()
            || self.is_instance()
        {
            return None;
        }
        let source = self.shape_at(frame);
        let posed = self.skin_shape_at(Cow::Borrowed(&source), frame);
        let nodes = posed.contours().get(contour)?;
        let first = *nodes.get(start)?;
        let closed = !matches!(posed.as_ref(), Shape::Path { closed: false, .. });
        if nodes.len() < 2 || (!closed && start + 1 == nodes.len()) {
            return None;
        }
        let end = (start + 1) % nodes.len();
        let last = nodes[end];
        let inverse = self.box_transform(frame).invert()?;
        let [w, h] = self.scaled_box_at(frame).1.map(f64::from);
        let [dx, dy] = world_delta.map(f64::from);
        let delta = [
            (f64::from(inverse.sx) * dx + f64::from(inverse.kx) * dy) / w,
            (f64::from(inverse.ky) * dx + f64::from(inverse.sy) * dy) / h,
        ];
        let mut t = f64::from(t);
        let (outgoing, incoming) = if first.outgoing == first.point && last.incoming == last.point {
            if first.point == last.point {
                return None;
            }
            // A straight edge with retracted handles has eased cubic timing.
            // Give it two tangent handles with linear timing and keep the same grasp.
            t = t * t * (3. - 2. * t);
            let a = first.point.map(f64::from);
            let b = last.point.map(f64::from);
            let third = |p: [f64; 2], q: [f64; 2]| {
                std::array::from_fn(|axis| p[axis] + (q[axis] - p[axis]) / 3.)
            };
            (third(a, b), third(b, a))
        } else {
            (first.outgoing.map(f64::from), last.incoming.map(f64::from))
        };
        // The two cubic Bernstein coefficients convert displacement of a handle
        // to displacement at t. Favor the nearer end and share equally at halfway.
        let u = 1. - t;
        let ease = |x: f64| 0.5 * (3. * x - 0.5).max(0.).powi(3);
        let w_end = if t <= 0.5 { ease(t) } else { 1. - ease(u) };
        let gain_start = (1. - w_end) / (3. * t * u * u);
        let gain_end = w_end / (3. * t * t * u);
        let authored = &source.contours()[contour];
        let move_handle = |node, p: [f64; 2], gain, original, posed| {
            let target = std::array::from_fn(|i| (p[i] + gain * delta[i]) as f32);
            if target == posed {
                return Some(original);
            }
            let point = self.unskin_local_point_at(contour, node, target, frame)?;
            point
                .iter()
                .zip(self.size)
                .all(|(x, s)| (x * s).is_finite())
                .then_some(point)
        };
        let outgoing = move_handle(
            start,
            outgoing,
            gain_start,
            authored[start].outgoing,
            first.outgoing,
        )?;
        let incoming = move_handle(
            end,
            incoming,
            gain_end,
            authored[end].incoming,
            last.incoming,
        )?;
        if outgoing == authored[start].outgoing && incoming == authored[end].incoming {
            return None;
        }
        let mut shape = source.into_owned();
        let nodes = &mut shape.contours_mut()[contour];
        if nodes[start].outgoing != outgoing {
            nodes[start].outgoing = outgoing;
            if closed || start > 0 {
                retain_compatible_style(&mut nodes[start]);
            }
        }
        if nodes[end].incoming != incoming {
            nodes[end].incoming = incoming;
            if closed || end + 1 < nodes.len() {
                retain_compatible_style(&mut nodes[end]);
            }
        }
        Some(shape)
    }
}

// A segment edit leaves the adjacent segments untouched. If the resulting
// handles no longer satisfy their coupling, expose the node as a corner.
pub(crate) fn retain_compatible_style(node: &mut PathNode) {
    if node.handle_style == HandleStyle::Corner {
        return;
    }
    let offset = |p: [f32; 2]| {
        [
            f64::from(p[0]) - f64::from(node.point[0]),
            f64::from(p[1]) - f64::from(node.point[1]),
        ]
    };
    let [x, y] = offset(node.incoming);
    let [u, v] = offset(node.outgoing);
    let tolerance = 1e-5;
    let compatible = match node.handle_style {
        HandleStyle::Symmetric => (x + u).hypot(y + v) <= tolerance * x.hypot(y).max(u.hypot(v)),
        _ => x * u + y * v <= 0. && (x * v - y * u).abs() <= tolerance * x.hypot(y) * u.hypot(v),
    };
    if !compatible {
        node.handle_style = HandleStyle::Corner;
    }
}

#[cfg(test)]
mod tests;
