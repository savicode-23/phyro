use crate::{
    arrange::is_object_editable,
    bezier::{distance_sq, interpolate, split_cubic, split_cubic_at},
    path_edit::HandleStyle,
    segment_bend::retain_compatible_style,
    Document, PathNode, Property, Shape, VectorObject,
};
use std::collections::HashSet;

mod clearance;
mod fit;

const MAX_ERROR: f64 = 0.75;
const MAX_FIT_SPANS: usize = 32;
type Point = [f64; 2];
type Cubic = [Point; 4];

fn minimum_anchors(shape: &Shape) -> usize {
    if matches!(shape, Shape::Path { closed: false, .. }) {
        2
    } else {
        3
    }
}

pub fn can_simplify_path(object: &VectorObject) -> bool {
    !object.locked
        && object.text.is_none()
        && object.image.is_none()
        && !object.is_instance()
        && object.animated.geometry_keys.is_empty()
        && !Property::Geometry.is_muted(object)
        && object.skin.is_none()
        && object.effects.is_empty()
        && object.shape.contours().iter().any(|nodes| nodes.len() >= 3)
}

/// Refit authored contours at the given pose. Each changed object reports the
/// number of removed anchors, which can be zero when only its handles changed.
pub fn simplify_paths(doc: &mut Document, ids: &[u64], frame: u32) -> Vec<(u64, usize)> {
    if frame >= doc.frames || ids.is_empty() {
        return Vec::new();
    }
    let selected: HashSet<_> = ids.iter().copied().collect();
    let edits: Vec<_> = doc
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            selected.contains(&o.id)
                && is_object_editable(doc, o)
                && o.visible_at(frame)
                && can_simplify_path(o)
        })
        .filter_map(|(index, object)| {
            simplify(object, frame).map(|(shape, removed)| (index, shape, removed))
        })
        .collect();
    edits
        .into_iter()
        .map(|(index, shape, removed)| {
            let object = &mut doc.objects[index];
            object.shape = shape;
            (object.id, removed)
        })
        .collect()
}

fn simplify(object: &VectorObject, frame: u32) -> Option<(Shape, usize)> {
    let ([width, height], transform) = object.outline_transform_at(frame);
    let display_vector = |[dx, dy]: Point| {
        let (x, y) = (dx * f64::from(width), dy * f64::from(height));
        [
            f64::from(transform.sx) * x + f64::from(transform.kx) * y,
            f64::from(transform.ky) * x + f64::from(transform.sy) * y,
        ]
    };
    let mut shape = object.shape.clone();
    let minimum = minimum_anchors(&shape);
    let closed = minimum == 3;
    let protected =
        clearance::protected_spans(shape.contours(), closed, &display_vector, MAX_ERROR)?;
    let mut removed = 0;
    for (nodes, protected) in shape.contours_mut().iter_mut().zip(protected) {
        if nodes.len() < 3 {
            continue;
        }
        let contour = simplify_contour(nodes, closed, minimum, &protected, &display_vector);
        removed += nodes.len() - contour.len();
        *nodes = contour;
    }
    (shape != object.shape && shape.valid_nodes(object.size)).then_some((shape, removed))
}

fn simplify_contour(
    source: &[PathNode],
    closed: bool,
    minimum: usize,
    protected: &[bool],
    display: &impl Fn(Point) -> Point,
) -> Vec<PathNode> {
    let count = source.len();
    let spans: Vec<_> = (0..count - usize::from(!closed))
        .map(|i| segment(source[i], source[(i + 1) % count]))
        .collect();
    let blocked = |i| protected[i];
    let node = |i| source[i % count];
    let mut output = Vec::with_capacity(source.len());
    output.push(source[0]);
    let mut start = 0;
    let mut remaining = count - minimum;
    while start < spans.len() {
        let max = (start + MAX_FIT_SPANS.min(remaining + 1)).min(spans.len());
        let mut end = start + 1;
        while end < max
            && !blocked(start)
            && !blocked(end)
            && !sharp_turn(spans[end - 1], spans[end], display)
        {
            end += 1;
        }
        let mut fitted = None;
        while end > start + 1 {
            let exact = (end == start + 2)
                .then(|| merge(node(start), node(start + 1), node(end), display))
                .flatten();
            fitted = exact.or_else(|| {
                let curve = fit::cubic(&spans[start..end], display, MAX_ERROR)?;
                Some(if curve == line(curve[0], curve[3]) {
                    (node(start).point, node(end).point)
                } else {
                    (curve[1].map(|v| v as f32), curve[2].map(|v| v as f32))
                })
            });
            if fitted.is_some() {
                break;
            }
            end = start + (end - start) / 2;
        }
        let mut last = node(end);
        if let Some((outgoing, incoming)) = fitted {
            let first = output.last_mut().unwrap();
            first.outgoing = outgoing;
            last.incoming = incoming;
            remaining -= end - start - 1;
        }
        let first = output.last_mut().unwrap();
        if *first != node(start) && (!closed || start != 0) {
            retain_compatible_style(first);
        }
        if end == count {
            output[0].incoming = last.incoming;
            if output[0] != source[0] {
                retain_compatible_style(&mut output[0]);
            }
        } else {
            output.push(last);
        }
        start = end;
    }
    if !closed && output.last() != source.last() {
        retain_compatible_style(output.last_mut().unwrap());
    }
    if output.len() == count {
        round_gentle_turns(&mut output, closed, &blocked, display);
    }
    output
}

fn dot([x, y]: Point, [u, v]: Point) -> f64 {
    x * u + y * v
}

fn sharp_turn(left: Cubic, right: Cubic, display: &impl Fn(Point) -> Point) -> bool {
    let joint = left[3];
    let before = [left[2], left[1], left[0]]
        .into_iter()
        .find(|p| *p != joint)
        .map(|p| display(delta(p, joint)));
    let after = right[1..]
        .iter()
        .find(|&&p| p != joint)
        .map(|&p| display(delta(joint, p)));
    before.zip(after).is_none_or(|(before, after)| {
        dot(before, after) < std::f64::consts::FRAC_1_SQRT_2 * length(before) * length(after)
    })
}

fn round_gentle_turns(
    nodes: &mut [PathNode],
    closed: bool,
    blocked: &impl Fn(usize) -> bool,
    display: &impl Fn(Point) -> Point,
) {
    let count = nodes.len();
    for i in usize::from(!closed)..count - usize::from(!closed) {
        let previous = (i + count - 1) % count;
        let [first, mut middle, last] = [nodes[previous], nodes[i], nodes[(i + 1) % count]];
        let [a, b, c] = [first.point, middle.point, last.point].map(|p| p.map(f64::from));
        if blocked(previous)
            || blocked(i)
            || first.outgoing != first.point
            || middle.incoming != middle.point
            || middle.outgoing != middle.point
            || last.incoming != last.point
            || sharp_turn(line(a, b), line(b, c), display)
        {
            continue;
        }
        let left = delta(a, b);
        let right = delta(b, c);
        let left_length = length(display(left));
        let right_length = length(display(right));
        if left_length == 0. || right_length == 0. {
            continue;
        }
        // Change just the two handles at this joint. The projections of both
        // cubics still cover their original straight edge in order.
        let axis = std::array::from_fn(|j| (left[j] / left_length + right[j] / right_length) / 2.);
        let [x, y] = display(axis);
        let [u, v] = display(left).map(|v| v / left_length);
        let bend = (x * v - y * u).abs();
        if bend < 1e-4 {
            continue;
        }
        let radius = (0.9 * MAX_ERROR / bend).min(left_length.min(right_length) / 3.);
        let handle = |step: f64| std::array::from_fn(|j| (b[j] + axis[j] * step) as f32);
        let incoming = handle(-radius);
        let outgoing = handle(radius);
        if incoming == middle.point
            || outgoing == middle.point
            || ![(incoming, a), (outgoing, c)]
                .into_iter()
                .all(|(handle, neighbor)| {
                    let ray = display(delta(b, handle.map(f64::from)));
                    let edge = display(delta(b, neighbor));
                    (0. ..=dot(edge, edge)).contains(&dot(ray, edge))
                        && distance_sq(ray, [0.; 2], edge) <= MAX_ERROR * MAX_ERROR
                })
        {
            continue;
        }
        middle.incoming = incoming;
        middle.outgoing = outgoing;
        middle.handle_style = HandleStyle::Smooth;
        retain_compatible_style(&mut middle);
        nodes[i] = middle;
    }
}

fn line(a: Point, b: Point) -> Cubic {
    [a, interpolate(a, b, 1. / 3.), interpolate(a, b, 2. / 3.), b]
}

fn segment(a: PathNode, b: PathNode) -> Cubic {
    let [p, u, v, q] = [a.point, a.outgoing, b.incoming, b.point].map(|p| p.map(f64::from));
    if p == u && q == v {
        line(p, q)
    } else {
        [p, u, v, q]
    }
}

fn delta(a: Point, b: Point) -> Point {
    [b[0] - a[0], b[1] - a[1]]
}

fn length([x, y]: Point) -> f64 {
    x.hypot(y)
}

fn within_error(difference: Cubic, limit_squared: f64, depth: u8) -> bool {
    let inside = |[x, y]: Point| x * x + y * y <= limit_squared;
    if !inside(difference[0]) || !inside(difference[3]) {
        return false;
    }
    (inside(difference[1]) && inside(difference[2]))
        || (depth > 0
            && split_cubic(difference)
                .into_iter()
                .all(|half| within_error(half, limit_squared, depth - 1)))
}

fn handles_through(
    p: Point,
    q: Point,
    joint: Point,
    start_axis: Point,
    end_axis: Point,
    t: f64,
    display: &impl Fn(Point) -> Point,
) -> Option<[f64; 2]> {
    // Solve the two tangent lengths that put the cubic at the old join.
    let a = display(start_axis);
    let b = display(end_axis);
    let u = 1. - t;
    let base = interpolate(p, q, t * t * (3. - 2. * t));
    let offset = display(delta(base, joint));
    let cross = |[x, y]: Point, [u, v]: Point| x * v - y * u;
    let det = cross(a, b);
    if det.abs() <= 1e-4 * length(a) * length(b) {
        return None;
    }
    let factors = [
        cross(offset, b) / (det * 3. * u * u * t),
        cross(a, offset) / (det * 3. * u * t * t),
    ];
    factors
        .into_iter()
        .all(|f| f >= 0. && f.is_finite())
        .then_some(factors)
}

fn merge(
    a: PathNode,
    b: PathNode,
    c: PathNode,
    display: &impl Fn(Point) -> Point,
) -> Option<([f32; 2], [f32; 2])> {
    let left = segment(a, b);
    let right = segment(b, c);
    let (p, joint, q) = (left[0], left[3], right[3]);
    let (first, second) = (
        length(display(delta(p, joint))),
        length(display(delta(joint, q))),
    );
    let chord = display(delta(p, q));
    if first == 0. || second == 0. || chord == [0.; 2] {
        return None;
    }
    // Use a tighter budget near a tiny feature and avoid removing a span
    // which doubles back along the merged chord.
    let tolerance = MAX_ERROR.min(0.25 * first.min(second));
    let monotone = |curve: Cubic| {
        curve
            .windows(2)
            .all(|w| dot(display(delta(w[0], w[1])), chord) >= 0.)
    };
    if !monotone(left) || !monotone(right) {
        return None;
    }
    let fits = |curve: Cubic, t: f64| {
        monotone(curve)
            && split_cubic_at(curve, t)
                .into_iter()
                .zip([left, right])
                // The difference is itself a cubic whose control hull bounds its
                // entire deviation, including between samples and at the join.
                .all(|(candidate, source)| {
                    let difference =
                        std::array::from_fn(|i| display(delta(candidate[i], source[i])));
                    within_error(difference, tolerance * tolerance, 4)
                })
    };
    let by_chord = first / (first + second);
    if fits(line(p, q), by_chord) {
        return Some((a.point, c.point));
    }
    // For a subdivided cubic, the lengths of the two join handles reveal
    // the split parameter. Chord length is a useful fallback on freehand spans.
    let (m, n) = (
        length(display(delta(left[2], joint))),
        length(display(delta(joint, right[1]))),
    );
    let by_tangent = m / (m + n);
    let start_axis = delta(p, left[1]);
    let end_axis = delta(q, right[2]);
    for t in [by_tangent, by_chord] {
        if !(0.0 < t && t < 1.) {
            continue;
        }
        let handle = |endpoint: Point, axis: Point, factor: f64| -> [f32; 2] {
            std::array::from_fn(|i| (endpoint[i] + factor * axis[i]) as f32)
        };
        let extended = [1. / t, 1. / (1. - t)];
        for factors in [
            Some(extended),
            handles_through(p, q, joint, start_axis, end_axis, t, display),
        ]
        .into_iter()
        .flatten()
        {
            let outgoing = handle(p, start_axis, factors[0]);
            let incoming = handle(q, end_axis, factors[1]);
            if fits([p, outgoing.map(f64::from), incoming.map(f64::from), q], t) {
                return Some((outgoing, incoming));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_edit::subdivide;

    #[test]
    fn joins_a_subdivided_bezier_at_the_displayed_pose() {
        let mut a = PathNode::corner([0., 0.]);
        let mut b = PathNode::corner([1., 0.5]);
        a.outgoing = [0.2, 0.4];
        b.incoming = [0.7, 0.6];
        let mut nodes = vec![a, b];
        assert!(subdivide(&mut nodes, false, 0));
        let mut doc = Document::default();
        let id = doc.add_shape(
            [31., 41.],
            [90., 52.],
            [255; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        doc.objects[0].rotation = 32.;
        doc.objects[0].animated.skew = 12.;
        doc.objects[0].scale = [-1.2, 1.8];
        let mut expected = doc.objects[0].clone();
        assert_eq!(simplify_paths(&mut doc, &[id], 0), [(id, 1)]);
        let nodes = &doc.objects[0].shape.contours()[0];
        assert_eq!([nodes[0].point, nodes[1].point], [a.point, b.point]);
        for (actual, source) in nodes[0]
            .outgoing
            .into_iter()
            .chain(nodes[1].incoming)
            .zip(a.outgoing.into_iter().chain(b.incoming))
        {
            assert!((actual - source).abs() < 1e-6);
        }
        expected.shape.clone_from(&doc.objects[0].shape);
        assert_eq!(doc.objects[0], expected);
        assert!(simplify_paths(&mut doc, &[id], 0).is_empty());
    }

    #[test]
    fn fits_a_smooth_arc_across_independently_authored_cubics() {
        let step = std::f64::consts::FRAC_PI_4;
        let handle = 4. / 3. * (step / 4.).tan();
        let nodes = [0., step, 2. * step]
            .map(|angle| {
                let (sin, cos) = angle.sin_cos();
                PathNode {
                    point: [cos as f32, sin as f32],
                    incoming: [(cos + sin * handle) as f32, (sin - cos * handle) as f32],
                    outgoing: [(cos - sin * handle) as f32, (sin + cos * handle) as f32],
                    handle_style: crate::path_edit::HandleStyle::Smooth,
                }
            })
            .to_vec();
        let mut doc = Document::default();
        let id = doc.add_shape(
            [0.; 2],
            [100.; 2],
            [255; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        assert_eq!(simplify_paths(&mut doc, &[id], 0), [(id, 1)]);
        let nodes = &doc.objects[0].shape.contours()[0];
        let curve = segment(nodes[0], nodes[1]);
        for step in 0..=16 {
            let point = split_cubic_at(curve, f64::from(step) / 16.)[0][3];
            assert!((100. * (length(point) - 1.)).abs() < 0.75);
        }
    }

    #[test]
    fn removes_jitter_while_keeping_endpoints_corners_and_the_compound_hole() {
        let mut doc = Document::default();
        let open = [
            [0., 0.],
            [0.1, 0.003],
            [0.2, 0.],
            [0.3, 0.003],
            [0.4, 0.],
            [0.4, 1.],
        ]
        .map(PathNode::corner);
        let open_id = doc.add_shape(
            [0.; 2],
            [100.; 2],
            [255; 4],
            Shape::Path {
                nodes: open.to_vec(),
                closed: false,
            },
            1,
        );
        let outer =
            [[0., 0.], [0.5, 0.], [1., 0.], [1., 0.5], [1., 1.], [0., 1.]].map(PathNode::corner);
        let hole = [
            [0.25, 0.25],
            [0.25, 0.5],
            [0.25, 0.75],
            [0.75, 0.75],
            [0.75, 0.25],
        ]
        .map(PathNode::corner);
        let compound_id = doc.add_shape(
            [0.; 2],
            [100.; 2],
            [255; 4],
            Shape::Compound {
                contours: vec![outer.to_vec(), hole.to_vec()],
            },
            1,
        );
        assert_eq!(
            simplify_paths(&mut doc, &[open_id, compound_id], 0),
            [(open_id, 3), (compound_id, 3)]
        );
        assert_eq!(
            doc.objects[0].shape,
            Shape::Path {
                nodes: [open[0], open[4], open[5]].to_vec(),
                closed: false,
            }
        );
        assert_eq!(
            doc.objects[1].shape,
            Shape::Compound {
                contours: vec![
                    [outer[0], outer[2], outer[4], outer[5]].to_vec(),
                    [hole[0], hole[2], hole[3], hole[4]].to_vec(),
                ],
            }
        );
        assert_eq!(doc.hit_test([50., 50.], 0), None);
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn keeps_large_deviations_at_the_playhead_and_skips_layout_dependent_artwork() {
        let mut doc = Document::default();
        let shape = Shape::Path {
            nodes: [[0., 0.4], [0.5, 0.406], [1., 0.4]]
                .map(PathNode::corner)
                .to_vec(),
            closed: false,
        };
        let ids = std::array::from_fn::<_, 4, _>(|_| {
            doc.add_shape([0.; 2], [100.; 2], [255; 4], shape.clone(), 1)
        });
        doc.objects[1]
            .animated
            .geometry_keys
            .insert(0, shape.clone());
        doc.objects[2].scale_keys.insert(2, [1., 10.]);
        doc.objects[3].locked = true;
        let before = doc.objects.clone();
        assert_eq!(
            simplify_paths(&mut doc, &ids, 2),
            [(ids[0], 1), (ids[2], 0)]
        );
        assert_eq!(doc.objects[1], before[1]);
        assert_eq!(doc.objects[3], before[3]);
        let nodes = &doc.objects[2].shape.contours()[0];
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[1].point, [0.5, 0.406]);
        assert_ne!(nodes[1].incoming, nodes[1].point);
    }

    #[test]
    fn refits_a_dense_closed_outline_to_cubics_at_the_displayed_pose() {
        let nodes: Vec<_> = (0..128)
            .map(|i| {
                let (sin, cos) = (std::f32::consts::TAU * i as f32 / 128.).sin_cos();
                PathNode::corner([0.5 + 0.5 * cos, 0.5 + 0.5 * sin])
            })
            .collect();
        let mut doc = Document::default();
        let id = doc.add_shape(
            [17., 30.],
            [180., 95.],
            [255; 4],
            Shape::Path {
                nodes: nodes.clone(),
                closed: true,
            },
            1,
        );
        doc.objects[0].scale = [-1.2, 0.83];
        doc.objects[0].rotation = 19.;
        doc.objects[0].animated.skew = 14.;

        let result = simplify_paths(&mut doc, &[id], 0);
        let Shape::Path {
            nodes: fitted,
            closed: true,
        } = &doc.objects[0].shape
        else {
            panic!("the outline stays closed");
        };
        assert_eq!(result, [(id, nodes.len() - fitted.len())]);
        assert!(fitted.len() <= 16, "{} fitted anchors", fitted.len());
        assert_eq!(fitted[0].point, nodes[0].point);
        assert!(fitted.iter().any(|n| n.point != n.outgoing));
        let world = |p| doc.objects[0].local_to_world(p, 0).map(f64::from);
        let before: Vec<_> = nodes.iter().map(|n| world(n.point)).collect();
        let source_edges = before.iter().zip(before.iter().cycle().skip(1));
        for (&a, &b) in fitted.iter().zip(fitted.iter().cycle().skip(1)) {
            let curve = [a.point, a.outgoing, b.incoming, b.point].map(world);
            for i in 0..=32 {
                let point = split_cubic_at(curve, f64::from(i) / 32.)[0][3];
                assert!(source_edges
                    .clone()
                    .any(|(&a, &b)| { distance_sq(point, a, b) <= (MAX_ERROR + 0.001).powi(2) }));
            }
        }
        doc.validate().unwrap();
    }

    #[test]
    fn protects_a_hole_with_less_clearance_than_the_error_limit() {
        let outer = [
            [0., 0.1],
            [0.2, 0.095],
            [0.4, 0.1],
            [0.4, 0.4],
            [0.2, 0.4],
            [0., 0.4],
        ]
        .map(PathNode::corner);
        let hole = [[0.2, 0.097], [0.16, 0.19], [0.24, 0.19]].map(PathNode::corner);
        let mut doc = Document::default();
        let id = doc.add_shape(
            [0.; 2],
            [100.; 2],
            [255; 4],
            Shape::Compound {
                contours: vec![outer.to_vec(), hole.to_vec()],
            },
            1,
        );
        assert_eq!(doc.hit_test([20., 9.85], 0), None);
        assert_eq!(doc.hit_test([18.5, 9.85], 0), Some(id));
        assert_eq!(simplify_paths(&mut doc, &[id], 0), [(id, 1)]);
        let contours = doc.objects[0].shape.contours();
        assert_eq!(&contours[0][..3], &outer[..3]);
        assert_eq!(contours[1], hole);
        assert_eq!(doc.hit_test([20., 9.85], 0), None);
        assert_eq!(doc.hit_test([18.5, 9.85], 0), Some(id));
    }

    #[test]
    fn keeps_the_sides_of_a_narrow_notch_from_crossing_the_same_contour() {
        let nodes = [
            [0., 0.],
            [0.1, 0.006],
            [0.2, 0.],
            [0.2, -0.04],
            [0.15, -0.04],
            [0.15, 0.0015],
            [0.05, 0.0015],
            [0.05, -0.04],
            [0., -0.04],
        ]
        .map(PathNode::corner);
        let mut doc = Document::default();
        let id = doc.add_shape(
            [20.; 2],
            [100.; 2],
            [255; 4],
            Shape::Path {
                closed: true,
                nodes: nodes.to_vec(),
            },
            1,
        );
        assert_eq!(doc.hit_test([30., 20.35], 0), Some(id));
        assert_eq!(doc.hit_test([30., 20.05], 0), None);
        simplify_paths(&mut doc, &[id], 0);
        assert_eq!(&doc.objects[0].shape.contours()[0][..3], &nodes[..3]);
        assert_eq!(doc.hit_test([30., 20.35], 0), Some(id));
        assert_eq!(doc.hit_test([30., 20.05], 0), None);
    }

    #[test]
    fn rounds_the_start_of_a_closed_contour_without_moving_its_anchor() {
        let nodes = [[0., 0.], [0.1, 0.], [-0.1, 0.004]].map(PathNode::corner);
        let mut doc = Document::default();
        let id = doc.add_shape(
            [20.; 2],
            [100.; 2],
            [255; 4],
            Shape::Path {
                closed: true,
                nodes: nodes.to_vec(),
            },
            1,
        );
        assert_eq!(simplify_paths(&mut doc, &[id], 0), [(id, 0)]);
        let rounded = &doc.objects[0].shape.contours()[0];
        assert_eq!(rounded[0].point, nodes[0].point);
        assert_eq!(rounded[0].handle_style, HandleStyle::Smooth);
        assert_eq!(&rounded[1..], &nodes[1..]);
    }
}
