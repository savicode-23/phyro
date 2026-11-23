//! Live modifiers evaluate the authored cubic path in the object's unscaled local frame.
use crate::{PathNode, Shape, VectorObject};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

pub const MAX_EFFECTS: usize = 16;
type Point = [f32; 2];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PathEffect {
    /// Reflect around the local center line on one axis.
    Mirror { horizontal: bool },
    /// Distance to cut back on each adjacent straight edge in local artboard pixels.
    RoundCorners { distance: f32 },
    /// Maximum deviation of a removed straight anchor from the replacement segment.
    Simplify { tolerance: f32 },
}

impl VectorObject {
    pub(crate) fn validate_effects(&self) -> Result<(), String> {
        if self.effects.len() > MAX_EFFECTS
            || !self.effects.is_empty() && (self.shape.contours().is_empty() || self.text.is_some())
        {
            return Err(format!(
                "Object {} has an invalid path effect stack.",
                self.id
            ));
        }
        for effect in &self.effects {
            let amount = match effect {
                PathEffect::RoundCorners { distance } => distance,
                PathEffect::Simplify { tolerance } => tolerance,
                PathEffect::Mirror { .. } => continue,
            };
            if !(0.0..=4096.).contains(amount) {
                return Err(format!(
                    "Object {} has an invalid effect distance.",
                    self.id
                ));
            }
        }
        Ok(())
    }

    pub fn evaluated_shape(&self) -> Cow<'_, Shape> {
        self.evaluated_shape_at(0)
    }

    pub fn evaluated_shape_at(&self, frame: u32) -> Cow<'_, Shape> {
        let source = self.skinned_shape_at(frame);
        if self.effects.is_empty() || source.contours().is_empty() {
            return source;
        }
        let mut shape = source.into_owned();
        let closed = !matches!(shape, Shape::Path { closed: false, .. });
        for nodes in shape.contours_mut() {
            for &effect in &self.effects {
                match effect {
                    PathEffect::Mirror { horizontal } => {
                        let axis = usize::from(!horizontal);
                        for node in &mut *nodes {
                            for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                                p[axis] = 1. - p[axis];
                            }
                        }
                    }
                    PathEffect::RoundCorners { distance } => {
                        *nodes = round_corners(nodes, closed, self.size, distance);
                    }
                    PathEffect::Simplify { tolerance } => {
                        *nodes = simplify(nodes, closed, self.size, tolerance);
                    }
                }
            }
        }
        Cow::Owned(shape)
    }
}

pub fn lerp(a: Point, b: Point, t: f32) -> Point {
    std::array::from_fn(|i| {
        (f64::from(a[i]) * f64::from(1. - t) + f64::from(b[i]) * f64::from(t)) as f32
    })
}

fn vector(a: Point, b: Point, size: Point) -> [f64; 2] {
    std::array::from_fn(|i| (f64::from(b[i]) - f64::from(a[i])) * f64::from(size[i]))
}

fn round_corners(nodes: &[PathNode], closed: bool, size: Point, distance: f32) -> Vec<PathNode> {
    if distance <= 0. || nodes.len() < 3 {
        return nodes.to_vec();
    }
    let n = nodes.len();
    let mut result = Vec::with_capacity(n);
    for (i, &node) in nodes.iter().enumerate() {
        let prev = nodes[(i + n - 1) % n];
        let next = nodes[(i + 1) % n];
        if !closed && (i == 0 || i + 1 == n)
            || node.incoming != node.point
            || node.outgoing != node.point
            || prev.outgoing != prev.point
            || next.incoming != next.point
        {
            result.push(node);
            continue;
        }
        let a = vector(node.point, prev.point, size);
        let b = vector(node.point, next.point, size);
        let (la, lb) = (a[0].hypot(a[1]), b[0].hypot(b[1]));
        if (a[0] * b[1] - a[1] * b[0]).abs() <= 1e-8 * la * lb {
            result.push(node);
            continue;
        }
        let cut = f64::from(distance).min(0.5 * la.min(lb));
        let incoming = lerp(node.point, prev.point, (cut / la) as f32);
        let outgoing = lerp(node.point, next.point, (cut / lb) as f32);
        // A quadratic through the old corner converted exactly to a cubic.
        result.push(PathNode {
            outgoing: lerp(incoming, node.point, 2. / 3.),
            ..PathNode::corner(incoming)
        });
        result.push(PathNode {
            incoming: lerp(outgoing, node.point, 2. / 3.),
            ..PathNode::corner(outgoing)
        });
    }
    result
}

fn approx_straight(a: PathNode, b: PathNode, c: PathNode, size: Point, tolerance: f32) -> bool {
    if a.outgoing != a.point
        || b.incoming != b.point
        || b.outgoing != b.point
        || c.incoming != c.point
    {
        return false;
    }
    let ab = vector(a.point, b.point, size);
    let ac = vector(a.point, c.point, size);
    let len = ac[0] * ac[0] + ac[1] * ac[1];
    if len == 0. {
        return false;
    }
    let t = ((ab[0] * ac[0] + ab[1] * ac[1]) / len).clamp(0., 1.);
    (ab[0] - t * ac[0]).hypot(ab[1] - t * ac[1]) <= f64::from(tolerance)
}

fn simplify(nodes: &[PathNode], closed: bool, size: Point, tolerance: f32) -> Vec<PathNode> {
    let minimum = if closed { 3 } else { 2 };
    if nodes.len() <= minimum {
        return nodes.to_vec();
    }
    let mut result = vec![nodes[0]];
    let mut i = 1;
    while i + 1 < nodes.len() {
        // Remove disjoint pairs of edges so the deviation bound applies to
        // every replaced segment and intentional cubic handles are retained.
        if result.len() + nodes.len() - i > minimum
            && approx_straight(nodes[i - 1], nodes[i], nodes[i + 1], size, tolerance)
        {
            i += 1;
        }
        result.push(nodes[i]);
        i += 1;
    }
    if i < nodes.len() {
        result.push(nodes[i]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    #[test]
    fn modifiers_stack_without_changing_source_nodes() {
        let nodes = [[0., 0.], [0.5, 0.], [1., 0.], [0.2, 1.]]
            .map(PathNode::corner)
            .to_vec();
        let shape = Shape::Path {
            nodes,
            closed: true,
        };
        let mut doc = Document::default();
        doc.add_shape([0.; 2], [120., 40.], [255; 4], shape.clone(), 1);
        let obj = &mut doc.objects[0];
        obj.effects = vec![
            PathEffect::Simplify { tolerance: 0. },
            PathEffect::Mirror { horizontal: true },
            PathEffect::RoundCorners { distance: 6. },
        ];
        let Shape::Path { nodes, .. } = obj.evaluated_shape().into_owned() else {
            panic!()
        };
        assert_eq!(nodes.len(), 6);
        assert!(nodes.iter().any(|n| n.point != n.incoming));
        assert_eq!(obj.shape, shape);
        assert!(doc.validate().is_ok());
        doc.objects[0].effects.clear();
        assert_eq!(doc.objects[0].evaluated_shape().as_ref(), &shape);
    }
    #[test]
    fn simplifying_does_not_change_cubic_segments() {
        let mut nodes = [[0., 0.], [0.3, 0.], [0.7, 0.], [1., 0.]].map(PathNode::corner);
        nodes[1].outgoing = [0.4, 0.1];
        assert_eq!(simplify(&nodes, false, [1.; 2], 5.), nodes);
    }
}
