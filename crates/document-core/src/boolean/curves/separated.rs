//! Certify that a Bézier outline has no self-intersections. Subdivision
//! tightens the bounds around curls and concavities, without adding nodes to
//! the result. Only consecutive intervals may meet, at their shared endpoint.
use super::super::flatten::split_cubic;
use crate::PathNode;

type Point = [f64; 2];
type Hull = [Point; 4];

fn delta(a: Point, b: Point) -> Point {
    [b[0] - a[0], b[1] - a[1]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn extent(hull: &Hull, axis: Point) -> (f64, f64) {
    let mut lo = dot(hull[0], axis);
    let mut hi = lo;
    for p in &hull[1..] {
        let v = dot(*p, axis);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    (lo, hi)
}

fn separate(a: &Hull, b: &Hull, adjacent: bool, margin: f64) -> bool {
    let separated_on = |axis| {
        let (lo, hi) = extent(a, axis);
        let (l, h) = extent(b, axis);
        if adjacent {
            // Strictly separated far endpoints plus weakly separated controls
            // put the open cubic interiors on opposite sides for 0 < t < 1.
            let start = dot(a[0], axis);
            let end = dot(b[3], axis);
            hi <= l && start < end || h <= lo && end < start
        } else {
            let gap = (l - hi).max(lo - h);
            gap > 0. && gap * gap > margin * margin * dot(axis, axis)
        }
    };
    if separated_on([1., 0.]) || separated_on([0., 1.]) {
        return true;
    }
    // A separating axis can be normal to an edge of either convex hull.
    // Trying every control-point pair avoids constructing the hulls.
    for hull in [a, b] {
        let chord = delta(hull[0], hull[3]);
        if adjacent && separated_on(chord) {
            return true;
        }
        for (i, &p) in hull.iter().enumerate() {
            for &q in &hull[i + 1..] {
                let [dx, dy] = delta(p, q);
                if separated_on([-dy, dx]) {
                    return true;
                }
            }
        }
    }
    false
}

// A projection that increases along a control polygon increases along its
// cubic. Curves whose controls turn back can still be injective piecewise.
fn monotone(hull: &Hull) -> bool {
    let chord = delta(hull[0], hull[3]);
    chord != [0.; 2] && hull.windows(2).all(|p| dot(delta(p[0], p[1]), chord) >= 0.)
}

struct Certificate {
    margin: f64,
    splits_left: usize,
}

impl Certificate {
    fn split(&mut self, hull: &Hull, depth: u8) -> Option<[Hull; 2]> {
        if depth >= 20 {
            return None;
        }
        self.splits_left = self.splits_left.checked_sub(1)?;
        Some(split_cubic(*hull))
    }

    fn injective(&mut self, hull: &Hull, depth: u8) -> bool {
        if monotone(hull) {
            return true;
        }
        let Some([left, right]) = self.split(hull, depth) else {
            return false;
        };
        self.injective(&left, depth + 1)
            && self.injective(&right, depth + 1)
            && self.disjoint(&left, &right, true, depth + 1)
    }

    fn disjoint(&mut self, a: &Hull, b: &Hull, adjacent: bool, depth: u8) -> bool {
        if separate(a, b, adjacent, self.margin) {
            return true;
        }
        let diameter = |h: &Hull| {
            let (left, right) = extent(h, [1., 0.]);
            let (top, bottom) = extent(h, [0., 1.]);
            (right - left).max(bottom - top)
        };
        // Split only the less tightly bounded interval. When a and b meet
        // at a[3] == b[0], only the halves containing that anchor may touch.
        if diameter(a) >= diameter(b) {
            let Some([left, right]) = self.split(a, depth) else {
                return false;
            };
            self.disjoint(&left, b, false, depth + 1)
                && self.disjoint(&right, b, adjacent, depth + 1)
        } else {
            let Some([left, right]) = self.split(b, depth) else {
                return false;
            };
            self.disjoint(a, &left, adjacent, depth + 1)
                && self.disjoint(a, &right, false, depth + 1)
        }
    }
}

pub(super) fn simple(nodes: &[PathNode]) -> bool {
    let mut hulls = Vec::with_capacity(nodes.len());
    let max = nodes
        .iter()
        .flat_map(|n| [n.point, n.incoming, n.outgoing])
        .flatten()
        .map(|v| f64::from(v).abs())
        .fold(0.0, f64::max);
    let mut certificate = Certificate {
        margin: 2. * super::TOLERANCE + 2. / super::GRID + 32. * f64::from(f32::EPSILON) * max,
        splits_left: 2_000,
    };
    for (a, b) in nodes.iter().zip(nodes.iter().cycle().skip(1)) {
        let hull = [a.point, a.outgoing, b.incoming, b.point].map(|p| p.map(f64::from));
        if !certificate.injective(&hull, 0) {
            return false;
        }
        for (i, prev) in hulls.iter().enumerate() {
            let separated = if i == 0 && hulls.len() + 1 == nodes.len() {
                certificate.disjoint(&hull, prev, true, 0)
            } else {
                certificate.disjoint(prev, &hull, i + 1 == hulls.len(), 0)
            };
            if !separated {
                return false;
            }
        }
        hulls.push(hull);
    }
    true
}
