//! Commands on the original cubic nodes. All indices refer to authored path order.
use crate::{path_effects::lerp, PathNode, Shape};
mod continuation;
mod simplify;
mod stroke;
pub use continuation::PathEnd;
pub use simplify::{can_simplify_path, simplify_paths};
pub use stroke::{has_convertible_stroke, stroke_to_paths};

impl crate::Document {
    /// Add a pen path whose anchors and absolute Bézier handles are in artboard pixels.
    pub fn add_artboard_path(
        &mut self,
        mut nodes: Vec<PathNode>,
        closed: bool,
        layer: u64,
        fill: [u8; 4],
    ) -> Result<u64, String> {
        let minimum = if closed { 3 } else { 2 };
        if !(minimum..=crate::MAX_PATH_NODES).contains(&nodes.len()) {
            return Err(format!(
                "Provide {minimum} to {} nodes for this path.",
                crate::MAX_PATH_NODES
            ));
        }
        if !crate::arrange::is_editable(self, layer) {
            return Err("Choose a visible, unlocked layer for the path.".into());
        }
        let mut low = [f32::INFINITY; 2];
        let mut high = [f32::NEG_INFINITY; 2];
        for point in nodes.iter().flat_map(|n| [n.point, n.incoming, n.outgoing]) {
            for axis in 0..2 {
                if !point[axis].is_finite() {
                    return Err("Path coordinates must be finite artboard pixels.".into());
                }
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        // Leave a one-pixel local frame on a degenerate axis (e.g. a vertical line).
        let size = std::array::from_fn(|axis| (high[axis] - low[axis]).max(1.));
        if size.iter().any(|s| !s.is_finite()) {
            return Err("The path exceeds the supported coordinate range.".into());
        }
        for node in &mut nodes {
            for point in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                *point = std::array::from_fn(|axis| (point[axis] - low[axis]) / size[axis]);
            }
        }
        Ok(self.add_shape(low, size, fill, Shape::Path { nodes, closed }, layer))
    }
}

pub fn can_combine_nodes(
    shape: &Shape,
    contour: usize,
    first: usize,
    last: usize,
    count: usize,
) -> bool {
    let Some(nodes) = shape.contours().get(contour) else {
        return false;
    };
    let open = matches!(shape, Shape::Path { closed: false, .. });
    let closes_path = open && first == 0 && last == nodes.len().saturating_sub(1) && count == 2;
    let minimum = if open && !closes_path { 2 } else { 3 };
    count >= 2 && nodes.len() + 1 >= minimum + count
}

/// Collapse anchors in one contour to their centroid. Node indices are sorted and unique.
pub fn combine_nodes(shape: &mut Shape, contour: usize, selected: &[usize]) -> Option<usize> {
    let nodes = shape.contours().get(contour)?;
    let &[first, .., last] = selected else {
        return None;
    };
    if last >= nodes.len() || !selected.windows(2).all(|pair| pair[0] < pair[1]) {
        return None;
    }
    let open = matches!(shape, Shape::Path { closed: false, .. });
    let closes_path = open && first == 0 && last == nodes.len() - 1 && selected.len() == 2;
    if !can_combine_nodes(shape, contour, first, last, selected.len()) {
        return None;
    }
    let point: [f32; 2] = std::array::from_fn(|axis| {
        let sum: f64 = selected
            .iter()
            .map(|&i| f64::from(nodes[i].point[axis]))
            .sum();
        (sum / selected.len() as f64) as f32
    });
    let translate = |node: PathNode, handle: [f32; 2]| {
        std::array::from_fn(|i| point[i] + (handle[i] - node.point[i]))
    };
    // Preserve the controls facing surviving neighbours of the merged anchor.
    let head = selected
        .iter()
        .copied()
        .enumerate()
        .take_while(|&(offset, index)| index == first + offset)
        .last()
        .map_or(first, |(_, index)| index);
    let incoming = if closes_path {
        nodes[last]
    } else if !open && first == 0 && last == nodes.len() - 1 {
        let tail = selected
            .iter()
            .rev()
            .copied()
            .enumerate()
            .take_while(|&(offset, index)| index == last - offset)
            .last()
            .map_or(last, |(_, index)| index);
        nodes[tail]
    } else {
        nodes[first]
    };
    let outgoing = nodes[head];
    let merged = PathNode {
        point,
        incoming: translate(incoming, incoming.incoming),
        outgoing: translate(outgoing, outgoing.outgoing),
        handle_style: HandleStyle::Corner,
    };
    let nodes = &mut shape.contours_mut()[contour];
    nodes[first] = merged;
    let mut removed = selected[1..].iter().copied().peekable();
    let mut i = 0;
    nodes.retain(|_| {
        let keep = removed.next_if_eq(&i).is_none();
        i += 1;
        keep
    });
    if let Shape::Path { closed, .. } = shape {
        *closed |= closes_path;
    }
    Some(first)
}

/// How moving an authored handle affects the handle across its anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HandleStyle {
    #[default]
    Corner,
    Smooth,
    Symmetric,
}

impl HandleStyle {
    pub(crate) fn is_corner(&self) -> bool {
        *self == Self::Corner
    }
}

impl crate::VectorObject {
    pub fn has_styled_nodes(&self) -> bool {
        let styled = |nodes: &[PathNode]| nodes.iter().any(|n| !n.handle_style.is_corner());
        let styled_shape = |s: &Shape| s.contours().iter().any(|nodes| styled(nodes));
        styled_shape(&self.shape)
            || self.animated.geometry_keys.values().any(styled_shape)
            || self
                .motion
                .as_ref()
                .is_some_and(|path| styled(path.nodes()))
            || self
                .stencil
                .as_ref()
                .is_some_and(|s| styled_shape(&s.shape))
    }
}

fn vector(start: [f32; 2], end: [f32; 2], size: [f32; 2]) -> [f64; 2] {
    std::array::from_fn(|i| (f64::from(end[i]) - f64::from(start[i])) * f64::from(size[i]))
}

fn length([x, y]: [f64; 2]) -> f64 {
    x.hypot(y)
}

fn unit(v: [f64; 2]) -> [f64; 2] {
    let d = length(v);
    if d == 0. {
        [0.; 2]
    } else {
        v.map(|x| x / d)
    }
}

pub fn style_nodes(
    nodes: &mut [PathNode],
    closed: bool,
    size: [f32; 2],
    range: std::ops::RangeInclusive<usize>,
    style: HandleStyle,
) -> bool {
    let n = nodes.len();
    if n == 0 {
        return false;
    }
    let mut changed = false;
    for i in *range.start()..=(*range.end()).min(n - 1) {
        let node = nodes[i];
        if style == HandleStyle::Corner {
            let corner = PathNode::corner(node.point);
            if corner != node {
                nodes[i] = corner;
                changed = true;
            }
            continue;
        }
        if node.handle_style == style {
            continue;
        }
        let p = node.point;
        let has_in = i > 0 || closed;
        let has_out = i + 1 < n || closed;
        let previous = if has_in {
            nodes[(i + n - 1) % n].point
        } else {
            p
        };
        let next = if has_out { nodes[(i + 1) % n].point } else { p };
        let before = vector(previous, p, size);
        let after = vector(p, next, size);
        let incoming = if has_in {
            vector(node.incoming, p, size)
        } else {
            [0.; 2]
        };
        let outgoing = if has_out {
            vector(p, node.outgoing, size)
        } else {
            [0.; 2]
        };
        // Keep an existing tangent. For a sharp corner with no handles, bisect its edges.
        let tangent = if length(outgoing) > 0. {
            unit(outgoing)
        } else if length(incoming) > 0. {
            unit(incoming)
        } else {
            let a = unit(before);
            let b = unit(after);
            let sum = [a[0] + b[0], a[1] + b[1]];
            if length(sum) > 1e-9 {
                unit(sum)
            } else {
                b
            }
        };
        let mut left = length(incoming);
        let mut right = length(outgoing);
        if style == HandleStyle::Symmetric && has_in && has_out {
            let radius = match (left > 0., right > 0.) {
                (true, true) => (left + right) * 0.5,
                (false, false) => length(before).min(length(after)) / 3.,
                _ => left.max(right),
            };
            left = radius;
            right = radius;
        } else {
            if left == 0. {
                left = length(before) / 3.;
            }
            if right == 0. {
                right = length(after) / 3.;
            }
        }
        let handle = |offset: f64| {
            std::array::from_fn(|axis| {
                (f64::from(p[axis]) + tangent[axis] * offset / f64::from(size[axis])) as f32
            })
        };
        let incoming = if has_in { handle(-left) } else { node.incoming };
        let outgoing = if has_out {
            handle(right)
        } else {
            node.outgoing
        };
        if [incoming, outgoing]
            .iter()
            .all(|p| p.iter().zip(size).all(|(x, s)| (x * s).is_finite()))
        {
            nodes[i] = PathNode {
                incoming,
                outgoing,
                handle_style: style,
                ..node
            };
            changed = true;
        }
    }
    changed
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HandleSide {
    Incoming,
    Outgoing,
}

impl PathNode {
    /// A smooth node retains its opposite handle length measured in artboard pixels.
    /// Rotation and uniform scaling cancel in the ratio; include the object's scale and shear.
    pub fn move_handle(
        &mut self,
        side: HandleSide,
        target: [f32; 2],
        opposite_active: bool,
        size: [f32; 2],
        shear: f32,
    ) -> bool {
        let (moved, opposite) = match side {
            HandleSide::Incoming => (&mut self.incoming, &mut self.outgoing),
            HandleSide::Outgoing => (&mut self.outgoing, &mut self.incoming),
        };
        if target == *moved || target.iter().zip(size).any(|(x, s)| !(x * s).is_finite()) {
            return false;
        }
        if opposite_active && self.handle_style != HandleStyle::Corner {
            let anchor = self.point;
            let distance = |p| {
                let [x, y] = vector(anchor, p, size);
                (x + f64::from(shear) * y).hypot(y)
            };
            let ratio = if self.handle_style == HandleStyle::Symmetric {
                1.
            } else {
                let d = distance(target);
                if d == 0. {
                    *moved = target;
                    return true;
                }
                distance(*opposite) / d
            };
            let other = std::array::from_fn(|i| {
                (f64::from(anchor[i]) - (f64::from(target[i]) - f64::from(anchor[i])) * ratio)
                    as f32
            });
            if other.iter().zip(size).any(|(x, s)| !(x * s).is_finite()) {
                return false;
            }
            *opposite = other;
        }
        *moved = target;
        true
    }
}

pub fn subdivide(nodes: &mut Vec<PathNode>, closed: bool, i: usize) -> bool {
    if i >= nodes.len() || !closed && i + 1 == nodes.len() {
        return false;
    }
    let j = (i + 1) % nodes.len();
    let (a, b) = (nodes[i], nodes[j]);
    let node = if a.outgoing == a.point && b.incoming == b.point {
        PathNode::corner(lerp(a.point, b.point, 0.5))
    } else {
        let (l, m, r) = (
            lerp(a.point, a.outgoing, 0.5),
            lerp(a.outgoing, b.incoming, 0.5),
            lerp(b.incoming, b.point, 0.5),
        );
        let (incoming, outgoing) = (lerp(l, m, 0.5), lerp(m, r, 0.5));
        nodes[i].outgoing = l;
        nodes[j].incoming = r;
        PathNode {
            handle_style: HandleStyle::Smooth,
            point: lerp(incoming, outgoing, 0.5),
            incoming,
            outgoing,
        }
    };
    nodes.insert(i + 1, node);
    true
}

pub fn reverse(nodes: &mut [PathNode]) {
    nodes.reverse();
    for n in nodes {
        std::mem::swap(&mut n.incoming, &mut n.outgoing);
    }
}

/// Opening at an anchor retains the final cubic by duplicating the seam endpoint.
pub fn open_at(nodes: &mut Vec<PathNode>, i: usize) {
    nodes.rotate_left(i);
    let mut end = nodes[0];
    nodes[0].incoming = nodes[0].point;
    end.outgoing = end.point;
    nodes.push(end);
}

pub fn close(nodes: &mut Vec<PathNode>) {
    if nodes.len() > 3 && nodes.first().unwrap().point == nodes.last().unwrap().point {
        let end = nodes.pop().unwrap();
        nodes[0].incoming = end.incoming;
    }
}

pub fn primitive_path(shape: &Shape) -> Option<Shape> {
    let nodes = match shape {
        Shape::Rectangle => [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .map(PathNode::corner)
            .to_vec(),
        Shape::Ellipse => {
            let k = 0.2761424;
            vec![
                PathNode {
                    handle_style: HandleStyle::Symmetric,
                    point: [0.5, 0.],
                    incoming: [0.5 - k, 0.],
                    outgoing: [0.5 + k, 0.],
                },
                PathNode {
                    handle_style: HandleStyle::Symmetric,
                    point: [1., 0.5],
                    incoming: [1., 0.5 - k],
                    outgoing: [1., 0.5 + k],
                },
                PathNode {
                    handle_style: HandleStyle::Symmetric,
                    point: [0.5, 1.],
                    incoming: [0.5 + k, 1.],
                    outgoing: [0.5 - k, 1.],
                },
                PathNode {
                    handle_style: HandleStyle::Symmetric,
                    point: [0., 0.5],
                    incoming: [0., 0.5 + k],
                    outgoing: [0., 0.5 - k],
                },
            ]
        }
        _ => return None,
    };
    Some(Shape::Path {
        nodes,
        closed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_and_closing_keeps_the_bezier_seam() {
        let Shape::Path { mut nodes, .. } = primitive_path(&Shape::Ellipse).unwrap() else {
            panic!()
        };
        let initial = nodes.clone();
        open_at(&mut nodes, 0);
        assert_eq!(nodes.first().unwrap().point, nodes.last().unwrap().point);
        close(&mut nodes);
        assert_eq!(nodes, initial);
        reverse(&mut nodes);
        reverse(&mut nodes);
        assert_eq!(nodes, initial);
    }
    #[test]
    fn subdividing_curves_and_styling_nodes() {
        let mut nodes = vec![
            PathNode {
                handle_style: Default::default(),
                point: [0., 0.],
                incoming: [0., 0.],
                outgoing: [0., 1.],
            },
            PathNode {
                handle_style: Default::default(),
                point: [1., 0.],
                incoming: [1., 1.],
                outgoing: [1., 0.],
            },
        ];
        assert!(!subdivide(&mut nodes, false, 1));
        assert!(subdivide(&mut nodes, false, 0));
        assert_eq!(nodes[1].point, [0.5, 0.75]);
        assert_eq!(nodes[1].incoming, [0.25, 0.75]);
        assert_eq!(nodes[1].outgoing, [0.75, 0.75]);
        let split = nodes[1];
        assert!(style_nodes(
            &mut nodes,
            false,
            [100., 30.],
            1..=1,
            HandleStyle::Corner
        ));
        assert_eq!(nodes[1], PathNode::corner(split.point));
        style_nodes(
            &mut nodes,
            false,
            [100., 30.],
            1..=1,
            HandleStyle::Symmetric,
        );
        assert_eq!(nodes[1].outgoing[1], nodes[1].incoming[1]);
        assert!((nodes[1].outgoing[0] + nodes[1].incoming[0] - 1.).abs() < 1e-6);
    }

    #[test]
    fn dragging_couples_only_the_selected_handle_style() {
        let size = [200., 40.];
        let mut node = PathNode {
            point: [0.5, 0.5],
            incoming: [0.4, 0.5],
            outgoing: [0.7, 0.5],
            handle_style: HandleStyle::Smooth,
        };
        // On a stretched, sheared object, the original opposite handle is 20 px long.
        let shear = 0.75;
        assert!(node.move_handle(HandleSide::Outgoing, [0.5, 1.0], true, size, shear));
        assert_eq!(node.incoming[0], node.point[0]);
        assert!((node.incoming[1] - 0.1).abs() < 1e-6);
        assert!(node.move_handle(HandleSide::Outgoing, node.point, true, size, shear));
        assert!((node.incoming[1] - 0.1).abs() < 1e-6);
        node.handle_style = HandleStyle::Symmetric;
        assert!(node.move_handle(HandleSide::Incoming, [0.6, 0.8], true, size, shear));
        assert!(node
            .outgoing
            .iter()
            .zip([0.4, 0.2])
            .all(|(a, b)| (a - b).abs() < 1e-6));
        node.handle_style = HandleStyle::Corner;
        let other = node.outgoing;
        assert!(node.move_handle(HandleSide::Incoming, [0.1, 0.3], true, size, shear));
        assert_eq!(node.outgoing, other);
    }

    #[test]
    fn styling_an_open_range_keeps_unused_handles_and_is_idempotent() {
        let mut nodes = [[0., 0.], [0.5, 1.], [1., 0.]].map(PathNode::corner);
        for style in [HandleStyle::Smooth, HandleStyle::Symmetric] {
            assert!(style_nodes(&mut nodes, false, [200., 40.], 0..=2, style));
            assert_eq!(nodes[0].incoming, nodes[0].point);
            assert_eq!(nodes[2].outgoing, nodes[2].point);
            assert_ne!(nodes[0].outgoing, nodes[0].point);
            assert_ne!(nodes[2].incoming, nodes[2].point);
            assert!(!style_nodes(&mut nodes, false, [200., 40.], 0..=2, style));
        }
    }

    #[test]
    fn corner_retracts_smooth_handles_in_one_action_and_then_becomes_a_no_op() {
        let mut nodes = [[0., 0.], [0.5, 1.], [1., 0.]].map(PathNode::corner);
        let convert =
            |nodes: &mut [PathNode], style| style_nodes(nodes, false, [100.; 2], 1..=1, style);
        assert!(convert(&mut nodes, HandleStyle::Smooth));
        let rounded = nodes[1];
        assert!(convert(&mut nodes, HandleStyle::Corner));
        assert_ne!(rounded.incoming, rounded.point);
        assert_eq!(nodes[1], PathNode::corner(rounded.point));
        assert!(!convert(&mut nodes, HandleStyle::Corner));
    }
}
