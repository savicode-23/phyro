use super::{segment, Point};
use crate::{bezier::split_cubic, PathNode};

struct SpanBounds {
    contour: usize,
    span: usize,
    min: Point,
    max: Point,
    chord: Point,
    start: f64,
    end: f64,
}

impl SpanBounds {
    fn is_local_to(
        &self,
        other: &Self,
        total: f64,
        spans: usize,
        closed: bool,
        margin: f64,
    ) -> bool {
        if self.contour != other.contour {
            return false;
        }
        let (first, second) = if self.span < other.span {
            (self, other)
        } else {
            (other, self)
        };
        if second.span - first.span == 1 || closed && first.span == 0 && second.span + 1 == spans {
            return true;
        }
        let gap = second.start - first.end;
        let gap = if closed {
            gap.min(total - second.end + first.start)
        } else {
            gap
        };
        // Nearby samples heading in the same direction are part of the local
        // edge. A narrow return can reverse direction in very little distance.
        gap <= 2. * margin
            && first.chord[0] * second.chord[0] + first.chord[1] * second.chord[1] > 0.
    }
}

/// Keep nearby, nonlocal boundaries in place. The fits can move both
/// boundaries by the error budget, so test twice that distance.
pub(super) fn protected_spans(
    contours: &[Vec<PathNode>],
    closed: bool,
    display: &impl Fn(Point) -> Point,
    tolerance: f64,
) -> Option<Vec<Vec<bool>>> {
    let margin = 2. * tolerance;
    if !margin.is_finite() || margin < 0. {
        return None;
    }
    let span_count = |nodes: &[PathNode]| nodes.len().saturating_sub(usize::from(!closed));
    let mut protected: Vec<_> = contours
        .iter()
        .map(|nodes| vec![false; span_count(nodes)])
        .collect();
    let mut bounds = Vec::with_capacity(protected.iter().map(Vec::len).sum());
    let mut lengths = Vec::with_capacity(contours.len());
    let mut extent_min = [f64::INFINITY; 2];
    let mut extent_max = [f64::NEG_INFINITY; 2];
    for (contour, nodes) in contours.iter().enumerate() {
        let mut distance = 0.;
        for (span, (&a, &b)) in nodes
            .iter()
            .zip(nodes.iter().cycle().skip(1))
            .take(span_count(nodes))
            .enumerate()
        {
            let curve = segment(a, b).map(display);
            if !curve.into_iter().flatten().all(f64::is_finite) {
                return None;
            }
            // Subdivision tightens the envelope around large handles and
            // still encloses the whole curve, including between anchors.
            let mut min = curve[0];
            let mut max = curve[0];
            for point in split_cubic(curve).into_iter().flatten() {
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            for axis in 0..2 {
                extent_min[axis] = extent_min[axis].min(min[axis]);
                extent_max[axis] = extent_max[axis].max(max[axis]);
            }
            let chord = [curve[3][0] - curve[0][0], curve[3][1] - curve[0][1]];
            let end = distance + chord[0].hypot(chord[1]);
            if !end.is_finite() {
                return None;
            }
            bounds.push(SpanBounds {
                contour,
                span,
                min,
                max,
                chord,
                start: distance,
                end,
            });
            distance = end;
        }
        lengths.push(distance);
    }
    let axis = usize::from(extent_max[1] - extent_min[1] > extent_max[0] - extent_min[0]);
    let other_axis = 1 - axis;
    bounds.sort_unstable_by(|a, b| a.min[axis].total_cmp(&b.min[axis]));
    // Overlapping projections can still make a sweep quadratic. The caller
    // retains the artwork if there is insufficient time to certify clearance.
    let mut comparisons_left: usize = 2_000_000;
    for (i, first) in bounds.iter().enumerate() {
        for second in bounds[i + 1..]
            .iter()
            .take_while(|second| second.min[axis] - first.max[axis] <= margin)
        {
            comparisons_left = comparisons_left.checked_sub(1)?;
            if second.min[other_axis] - first.max[other_axis] <= margin
                && first.min[other_axis] - second.max[other_axis] <= margin
                && !first.is_local_to(
                    second,
                    lengths[first.contour],
                    protected[first.contour].len(),
                    closed,
                    margin,
                )
            {
                protected[first.contour][first.span] = true;
                protected[second.contour][second.span] = true;
            }
        }
    }
    Some(protected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_a_closing_span_near_the_interior_of_another_contours_curve() {
        let mut outer = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]].map(PathNode::corner);
        outer[0].outgoing = [2., 2.];
        outer[1].incoming = [8., 2.];
        let hole = [[3., 2.4], [4., 5.], [5., 2.4]].map(PathNode::corner);
        let contours = [outer.to_vec(), hole.to_vec()];
        let protected = protected_spans(&contours, true, &|[x, y]| [-x, 0.5 * y], 0.25).unwrap();
        assert_eq!(protected[0], [true, false, false, false]);
        assert!(protected[1][2]);
        assert!(protected_spans(&contours, true, &|_| [f64::NAN, 0.], 0.25).is_none());
    }

    #[test]
    fn protects_the_opposite_edge_of_a_narrow_return_on_an_open_contour() {
        let nodes = [[0., 0.], [5., 0.], [5., 0.5], [0., 0.5]].map(PathNode::corner);
        let protected = protected_spans(&[nodes.to_vec()], false, &|p| p, 0.25).unwrap();
        assert_eq!(protected[0], [true, false, true]);
    }

    #[test]
    fn sweeps_dense_vertical_neighbors_without_protecting_them() {
        let nodes = (0..4_000)
            .map(|i| PathNode::corner([0., i as f32 * 0.05]))
            .collect();
        let protected = protected_spans(&[nodes], false, &|p| p, 0.25).unwrap();
        assert_eq!(protected[0].len(), 3_999);
        assert!(protected[0].iter().all(|&span| !span));
    }
}
