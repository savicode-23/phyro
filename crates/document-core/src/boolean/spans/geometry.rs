use super::super::{
    flatten,
    planar::{Point, GRID},
};
use super::Span;

pub(super) type World = [f64; 2];
pub(super) type Edge = (Point, Point);

pub(super) fn snapped(point: World) -> Point {
    point.map(|v| (v * GRID).round() as i64)
}

pub(super) fn exact_grid_point(point: World) -> Option<Point> {
    let grid = snapped(point);
    (point == grid.map(|v| v as f64 / GRID)).then_some(grid)
}

pub(super) fn unordered((a, b): Edge) -> Edge {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Chord {
    pub edge: Edge,
    bounds: [i64; 4],
}

impl Chord {
    pub fn new(a: Point, b: Point) -> Self {
        Self {
            edge: (a, b),
            bounds: [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ],
        }
    }

    pub fn left(self) -> i64 {
        self.bounds[0]
    }
    pub fn right(self) -> i64 {
        self.bounds[2]
    }

    pub fn near(self, other: Self, margin: i64) -> bool {
        let [l, t, r, b] = self.bounds;
        let [ll, tt, rr, bb] = other.bounds;
        if rr + margin < l || r + margin < ll || bb + margin < t || b + margin < tt {
            return false;
        }
        let (a, b) = self.edge;
        let (c, d) = other.edge;
        // The integer coordinates are limited to ±2^27 during flattening.
        let side = |p: Point, q: Point, r: Point| {
            (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
        };
        let same_side = |a: i64, b: i64| a != 0 && a.signum() == b.signum();
        if !same_side(side(a, b, c), side(a, b, d)) && !same_side(side(c, d, a), side(c, d, b)) {
            return true;
        }
        // Disjoint segments attain their minimum distance at an endpoint.
        let [a, b, c, d] = [a, b, c, d].map(|p| p.map(|v| v as f64));
        let margin_sq = (margin as f64).powi(2);
        [
            flatten::distance_sq(a, c, d),
            flatten::distance_sq(b, c, d),
            flatten::distance_sq(c, a, b),
            flatten::distance_sq(d, a, b),
        ]
        .into_iter()
        .any(|distance| distance <= margin_sq)
    }
}

pub(super) fn transverse(a: World, junction: World, b: World) -> bool {
    let [x, y] = std::array::from_fn(|i| a[i] - junction[i]);
    let [u, v] = std::array::from_fn(|i| b[i] - junction[i]);
    // Avoid extrapolating at grazing contacts (less than ~20 degrees).
    (x * v - y * u).abs() >= 0.35 * x.hypot(y) * u.hypot(v)
}

pub(super) struct Cubic(pub [World; 4]);

impl Cubic {
    pub(super) fn at(&self, t: f64) -> World {
        let [a, b, c, d] = self.0;
        std::array::from_fn(|i| {
            (1. - t).powi(3) * a[i]
                + 3. * (1. - t).powi(2) * t * b[i]
                + 3. * (1. - t) * t * t * c[i]
                + t.powi(3) * d[i]
        })
    }

    // One third of the derivative. Multiplying it by a subspan's length
    // gives the offset from a split endpoint to its Bézier handle.
    pub fn handle_offset(&self, t: f64, span: f64) -> World {
        let [a, b, c, d] = self.0;
        std::array::from_fn(|i| {
            span * ((1. - t).powi(2) * (b[i] - a[i])
                + 2. * (1. - t) * t * (c[i] - b[i])
                + t * t * (d[i] - c[i]))
        })
    }

    fn control_steps(&self, span: Span) -> [World; 3] {
        let dt = span.end - span.start;
        let p = self.at(span.start);
        let q = self.at(span.end);
        let first = self.handle_offset(span.start, dt);
        let last = self.handle_offset(span.end, dt);
        let middle = std::array::from_fn(|i| q[i] - p[i] - first[i] - last[i]);
        [first, middle, last]
    }

    pub fn monotone_across_line(&self, span: Span, (a, b): Edge) -> bool {
        let dx = (b[0] - a[0]) as f64;
        let dy = (b[1] - a[1]) as f64;
        let steps = self.control_steps(span).map(|[x, y]| dx * y - dy * x);
        steps.iter().all(|&v| v >= 0.) || steps.iter().all(|&v| v <= 0.)
    }

    // Keep the solver's actual junction. Locate a source parameter by
    // projection along its flattened chord, rejecting ambiguous projections.
    pub fn clipped(&self, span: Span, edge: Edge, junction: World) -> Option<Span> {
        let [start, end] = [edge.0, edge.1].map(|p| p.map(|v| v as f64 / GRID));
        let dir: World = std::array::from_fn(|i| end[i] - start[i]);
        let dot = |v: World| v[0] * dir[0] + v[1] * dir[1];
        let projection = |v: World| dot(std::array::from_fn(|i| v[i] - start[i]));
        let length = dir[0].hypot(dir[1]);
        let target = projection(junction);
        let margin = 4. * flatten::TOLERANCE;
        // i_overlay intersects on a grid with 16 extra fractional bits. Its
        // rounded point can be about 1.5e-8 artboard pixels off the chord.
        if length <= 2. * margin
            || target < margin * length
            || target > (length - margin) * length
            || flatten::distance_sq(junction, start, end) > 1e-15
        {
            return None;
        }
        if self.control_steps(span).into_iter().any(|d| dot(d) < 0.) {
            return None;
        }
        let (mut from, mut to) = (span.start, span.end);
        for _ in 0..34 {
            let mid = 0.5 * (from + to);
            if projection(self.at(mid)) < target {
                from = mid
            } else {
                to = mid
            }
        }
        let end = 0.5 * (from + to);
        let [x, y] = self.at(end);
        ((x - junction[0]).hypot(y - junction[1]) <= flatten::TOLERANCE + 1. / GRID)
            .then_some(Span { end, ..span })
    }
}
