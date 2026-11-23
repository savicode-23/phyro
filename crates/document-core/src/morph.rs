//! Correspondence between two individually authored Bézier outlines.
//! Arc length selects matching points; all output geometry is cut from the original cubics.
use crate::{path_edit::HandleStyle, PathNode, Shape, MAX_PATH_NODES};
use serde::{Deserialize, Serialize};

type Point = [f64; 2];
type Cubic = [Point; 4];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphPoint {
    pub segment: usize,
    pub t: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphGuide {
    pub from: MorphPoint,
    pub to: MorphPoint,
}

impl MorphPoint {
    pub fn position(self, shape: &Shape) -> Option<[f32; 2]> {
        let (nodes, closed) = outline(shape)?;
        if self.segment >= nodes.len() - usize::from(!closed) || !(0.0..=1.0).contains(&self.t) {
            return None;
        }
        Some(at(controls(nodes, self.segment), self.t.into()).map(|x| x as f32))
    }
}

fn outline(shape: &Shape) -> Option<(&[PathNode], bool)> {
    match shape {
        Shape::Path { nodes, closed } if (2..=MAX_PATH_NODES).contains(&nodes.len()) => {
            Some((nodes, *closed))
        }
        _ => None,
    }
}

fn controls(nodes: &[PathNode], i: usize) -> Cubic {
    let a = &nodes[i];
    let b = &nodes[(i + 1) % nodes.len()];
    [a.point, a.outgoing, b.incoming, b.point].map(|p| p.map(f64::from))
}

fn mix(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| (1. - t) * a[i] + t * b[i])
}

fn casteljau([a, b, c, d]: Cubic, t: f64) -> [Point; 2] {
    let mid = mix(b, c, t);
    [mix(mix(a, b, t), mid, t), mix(mid, mix(c, d, t), t)]
}

fn at(curve: Cubic, t: f64) -> Point {
    let [a, b] = casteljau(curve, t);
    mix(a, b, t)
}

// Bézier blossoming: repeated parameters give the two endpoints; the two
// mixed evaluations give their exact handles, also when traversing backwards.
fn subcurve(curve: Cubic, start: f64, end: f64) -> Cubic {
    let [a, b] = casteljau(curve, start);
    let [c, d] = casteljau(curve, end);
    [
        mix(a, b, start),
        mix(a, b, end),
        mix(c, d, start),
        mix(c, d, end),
    ]
}

const STEPS: usize = 16;
const PROBES: usize = 32;
const ROUND_OFF: f64 = 64. * f64::EPSILON;

struct Outline<'a> {
    nodes: &'a [PathNode],
    closed: bool,
    winding: Option<i8>,
    // Monotone cumulative fraction at each equal-parameter sample.
    length: Vec<f64>,
}

impl<'a> Outline<'a> {
    fn new(shape: &'a Shape) -> Result<Self, String> {
        let (nodes, closed) =
            outline(shape).ok_or("Choose a single editable path for each pose.")?;
        let segments = nodes.len() - usize::from(!closed);
        let samples = segments * STEPS;
        let mut length = Vec::with_capacity(samples + 1);
        length.push(0.);
        let mut total = 0.;
        let origin = nodes[0].point.map(f64::from);
        let mut area = 0.;
        for i in 0..segments {
            let curve = controls(nodes, i);
            let mut prev = curve[0];
            for step in 1..=STEPS {
                let p = at(curve, step as f64 / STEPS as f64);
                total += (p[0] - prev[0]).hypot(p[1] - prev[1]);
                area += (prev[0] - origin[0]) * (p[1] - origin[1])
                    - (prev[1] - origin[1]) * (p[0] - origin[0]);
                length.push(total);
                prev = p;
            }
        }
        if !total.is_finite() {
            return Err("Morph outlines must have finite coordinates.".into());
        }
        // Give coincident knots a tiny amount of parameter space. This keeps
        // inverse lookups single-valued, even for a completely collapsed pose.
        for (i, value) in length.iter_mut().enumerate() {
            let uniform = i as f64 / samples as f64;
            *value = if total == 0. {
                uniform
            } else {
                (1. - 1e-6) * (*value / total) + 1e-6 * uniform
            };
        }
        Ok(Self {
            nodes,
            closed,
            winding: (closed && area.abs() > total * total * 1e-6).then_some(area.signum() as i8),
            length,
        })
    }

    fn segments(&self) -> usize {
        (self.length.len() - 1) / STEPS
    }

    fn fraction(&self, point: MorphPoint) -> Result<f64, String> {
        if point.segment >= self.segments() || !(0.0..=1.0).contains(&point.t) {
            return Err("A morph guide is outside its outline.".into());
        }
        let sample = f64::from(point.t) * STEPS as f64;
        if point.t == 1. {
            return Ok(self.length[(point.segment + 1) * STEPS]);
        }
        let step = (sample as usize).min(STEPS - 1);
        let i = point.segment * STEPS + step;
        Ok(self.length[i] + (sample - step as f64) * (self.length[i + 1] - self.length[i]))
    }

    // Return a continuous segment index. A full turn on a closed outline
    // advances it by segments(), so samples on opposite sides of its seam
    // can be subdivided exactly like samples on any other segment.
    fn parameter(&self, distance: f64) -> f64 {
        let (s, turn) = if self.closed {
            let turn = distance.floor();
            (distance - turn, turn)
        } else {
            (distance.clamp(0., 1.), 0.)
        };
        let i = self
            .length
            .partition_point(|&v| v <= s)
            .saturating_sub(1)
            .min(self.length.len() - 2);
        // Adding or removing a contour turn can round an exact authored knot.
        let t = if (s - self.length[i]).abs() <= ROUND_OFF {
            0.
        } else if (s - self.length[i + 1]).abs() <= ROUND_OFF {
            1.
        } else {
            (s - self.length[i]) / (self.length[i + 1] - self.length[i])
        };
        turn * self.segments() as f64 + (i as f64 + t) / STEPS as f64
    }

    fn position(&self, distance: f64) -> Point {
        let q = self.parameter(distance);
        let n = self.segments();
        let i = (q.floor() as isize).rem_euclid(n as isize) as usize;
        if !self.closed && q >= n as f64 {
            return self.nodes[n].point.map(f64::from);
        }
        at(controls(self.nodes, i), q - q.floor())
    }

    fn address(&self, distance: f64) -> MorphPoint {
        let q = self.parameter(distance);
        if !self.closed && q >= self.segments() as f64 {
            return MorphPoint {
                segment: self.segments() - 1,
                t: 1.,
            };
        }
        MorphPoint {
            segment: (q.floor() as isize).rem_euclid(self.segments() as isize) as usize,
            t: (q - q.floor()) as f32,
        }
    }

    fn piece(&self, start: f64, end: f64) -> Cubic {
        let a = self.parameter(start);
        let b = self.parameter(end);
        let seg = ((a + b) * 0.5).floor();
        let i = (seg as isize).rem_euclid(self.segments() as isize) as usize;
        subcurve(
            controls(self.nodes, i),
            (a - seg).clamp(0., 1.),
            (b - seg).clamp(0., 1.),
        )
    }
}

// Convert an authored arc fraction to increasing travel from the chosen seam.
fn travel(s: f64, seam: f64, direction: f64, closed: bool) -> f64 {
    let d = (s - seam) * direction;
    if closed {
        d.rem_euclid(1.)
    } else {
        d
    }
}

fn interpolate_map(pairs: &[[f64; 2]], from: usize, value: f64) -> f64 {
    let i = pairs
        .partition_point(|p| p[from] <= value)
        .clamp(1, pairs.len() - 1);
    let (a, b) = (pairs[i - 1], pairs[i]);
    if value == b[from] {
        return b[1 - from];
    }
    let t = (value - a[from]) / (b[from] - a[from]);
    let to = 1 - from;
    a[to] + t * (b[to] - a[to])
}

struct Match {
    from_seam: f64,
    to_seam: f64,
    direction: f64,
    // Increasing distances along both oriented contours, including both ends.
    pairs: Vec<[f64; 2]>,
}

impl Match {
    fn with_guides(
        from: &Outline,
        pins: &[[f64; 2]],
        direction: f64,
        to_seam: f64,
    ) -> Option<Self> {
        let from_seam = if from.closed {
            pins.first().map_or(0., |p| p[0])
        } else {
            0.
        };
        let mut pairs = Vec::with_capacity(pins.len() + 2);
        for &[x, y] in pins {
            pairs.push([
                travel(x, from_seam, 1., from.closed),
                travel(y, to_seam, direction, from.closed),
            ]);
        }
        pairs.sort_by(|a, b| a[0].total_cmp(&b[0]));
        if pairs
            .windows(2)
            .any(|p| p[1][0] - p[0][0] <= ROUND_OFF || p[1][1] - p[0][1] <= ROUND_OFF)
        {
            return None;
        }
        if pairs.first() != Some(&[0., 0.]) {
            pairs.insert(0, [0., 0.]);
        }
        if pairs.last() != Some(&[1., 1.]) {
            pairs.push([1., 1.]);
        }
        if pairs
            .windows(2)
            .any(|p| p[1][0] <= p[0][0] || p[1][1] <= p[0][1])
        {
            return None;
        }
        Some(Self {
            from_seam,
            to_seam,
            direction,
            pairs,
        })
    }

    fn score(&self, from: &Outline, to: &Outline) -> f64 {
        let mut cost = 0.;
        let mut mean = [0.; 2];
        for i in 0..=PROBES - usize::from(from.closed) {
            let t = i as f64 / PROBES as f64;
            let a = from.position(self.from_seam + t);
            let b = to.position(self.to_seam + self.direction * interpolate_map(&self.pairs, 0, t));
            // A pose can move across the page. Score variation in travel so
            // translating either outline does not change correspondence.
            for axis in 0..2 {
                let shift = b[axis] - a[axis];
                let delta = shift - mean[axis];
                mean[axis] += delta / (i + 1) as f64;
                cost += delta * (shift - mean[axis]);
            }
        }
        cost
    }

    fn automatic_seam(from: &Outline, to: &Outline, direction: f64) -> Self {
        let mut mapping = Self {
            from_seam: 0.,
            to_seam: 0.,
            direction,
            pairs: vec![[0., 0.], [1., 1.]],
        };
        let mut best = (0., f64::INFINITY);
        let mut inspect = |seam: f64, best: &mut (f64, f64)| {
            mapping.to_seam = seam.rem_euclid(1.);
            let cost = mapping.score(from, to);
            if cost < best.1 {
                *best = (mapping.to_seam, cost);
            }
        };
        for i in 0..PROBES {
            inspect(i as f64 / PROBES as f64, &mut best);
        }
        let mut step = 1. / PROBES as f64;
        for _ in 0..10 {
            step *= 0.5;
            let phase = best.0;
            inspect(phase - step, &mut best);
            inspect(phase + step, &mut best);
        }
        // Prefer exact authored seams when redrawn contours only differ in
        // their first node. Scoring all shifts would be quadratic on long paths.
        let closest = to.parameter(best.0) as usize;
        if to.segments() <= PROBES {
            for i in 0..to.segments() {
                inspect(to.length[i * STEPS], &mut best);
            }
        } else {
            for i in [closest, (closest + 1) % to.segments()] {
                inspect(to.length[i * STEPS], &mut best);
            }
        }
        mapping.to_seam = best.0;
        mapping
    }
}

#[derive(Clone, Debug)]
pub struct MorphPlan {
    closed: bool,
    from: Vec<PathNode>,
    to: Vec<PathNode>,
    guides: Vec<MorphPoint>,
    knots: Vec<(MorphPoint, MorphPoint)>,
}

impl MorphPlan {
    pub fn build(from: &Shape, to: &Shape, guides: &[MorphGuide]) -> Result<Self, String> {
        if guides.len() > MAX_PATH_NODES {
            return Err("Too many morph guides.".into());
        }
        let from = Outline::new(from)?;
        let to = Outline::new(to)?;
        if from.closed != to.closed {
            return Err("Both morph poses must be open or both closed.".into());
        }
        let pins = guides
            .iter()
            .map(|g| Ok([from.fraction(g.from)?, to.fraction(g.to)?]))
            .collect::<Result<Vec<_>, String>>()?;
        // On a filled contour, taking the shorter route to a landmark can turn
        // the entire interior inside out. Maintain winding whenever the artist's
        // ordered landmarks allow it. Open and near-zero-area paths use distance.
        let preferred = from.winding.zip(to.winding).map(|(a, b)| f64::from(a * b));
        let first = preferred.unwrap_or(1.);
        let mut best = None;
        for direction in [first, -first] {
            let candidate = if from.closed && pins.is_empty() {
                Match::automatic_seam(&from, &to, direction)
            } else {
                let seam = if from.closed {
                    pins[0][1]
                } else if direction > 0. {
                    0.
                } else {
                    1.
                };
                let Some(candidate) = Match::with_guides(&from, &pins, direction, seam) else {
                    continue;
                };
                candidate
            };
            if preferred == Some(direction) {
                best = Some((0., candidate));
                break;
            }
            let score = candidate.score(&from, &to);
            if best.as_ref().is_none_or(|(old, _)| score < *old) {
                best = Some((score, candidate));
            }
        }
        let (_, mapping) = best.ok_or("Guide pairs must be distinct, follow the outline in order, and pair an open endpoint with an endpoint.")?;
        let Match {
            from_seam,
            to_seam,
            direction,
            pairs,
        } = mapping;
        let mut breaks = pairs.clone();
        for (i, path, seam, sign) in [(0, &from, from_seam, 1.), (1, &to, to_seam, direction)] {
            for &s in path.length.iter().step_by(STEPS) {
                let d = travel(s, seam, sign, from.closed);
                let other = interpolate_map(&pairs, i, d);
                breaks.push(if i == 0 { [d, other] } else { [other, d] });
            }
        }
        breaks.sort_by(|a, b| a[0].total_cmp(&b[0]));
        breaks
            .dedup_by(|a, b| (a[0] - b[0]).abs() <= ROUND_OFF && (a[1] - b[1]).abs() <= ROUND_OFF);
        let count = breaks.len() - usize::from(from.closed);
        if count > MAX_PATH_NODES {
            return Err("The matched morph exceeds the editable path node limit.".into());
        }
        let nodes = |path: &Outline, index: usize, seam: f64, sign: f64| {
            let mut result = vec![PathNode::corner([0.; 2]); count];
            for (i, b) in breaks.windows(2).enumerate() {
                let [p, c, d, q] = path
                    .piece(seam + sign * b[0][index], seam + sign * b[1][index])
                    .map(|p| p.map(|v| v as f32));
                let next = (i + 1) % count;
                result[i].point = p;
                result[i].outgoing = c;
                result[next].incoming = d;
                result[next].point = q;
            }
            if !path.closed {
                result[0].incoming = result[0].point;
                result[count - 1].outgoing = result[count - 1].point;
            }
            result
        };
        let guides = pins
            .iter()
            .map(|p| {
                let d = travel(p[0], from_seam, 1., from.closed);
                let i = breaks.partition_point(|b| b[0] < d - ROUND_OFF);
                if !from.closed && i >= count - 1 {
                    MorphPoint {
                        segment: count - 2,
                        t: 1.,
                    }
                } else {
                    MorphPoint {
                        segment: i % count,
                        t: 0.,
                    }
                }
            })
            .collect();
        let knots = breaks[..count]
            .iter()
            .map(|p| {
                (
                    from.address(from_seam + p[0]),
                    to.address(to_seam + direction * p[1]),
                )
            })
            .collect();
        Ok(Self {
            closed: from.closed,
            from: nodes(&from, 0, from_seam, 1.),
            to: nodes(&to, 1, to_seam, direction),
            guides,
            knots,
        })
    }

    pub fn sample(&self, t: f64) -> Shape {
        let interp = |a, b| crate::properties::pair(a, b, t);
        Shape::Path {
            closed: self.closed,
            nodes: self
                .from
                .iter()
                .zip(&self.to)
                .map(|(a, b)| PathNode {
                    point: interp(a.point, b.point),
                    incoming: interp(a.incoming, b.incoming),
                    outgoing: interp(a.outgoing, b.outgoing),
                    handle_style: HandleStyle::Corner,
                })
                .collect(),
        }
    }

    /// Landing places of the authored guides on an interpolated breakdown pose.
    pub fn sample_guides(&self) -> Vec<MorphPoint> {
        self.guides.clone()
    }

    /// Position on each authored pose of every node in the interpolated outline.
    pub fn knots(&self) -> &[(MorphPoint, MorphPoint)] {
        &self.knots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(points: &[[f32; 2]], closed: bool) -> Shape {
        Shape::Path {
            nodes: points.iter().copied().map(PathNode::corner).collect(),
            closed,
        }
    }

    fn point(segment: usize, t: f32) -> MorphPoint {
        MorphPoint { segment, t }
    }

    fn close(a: [f32; 2], b: [f32; 2]) {
        assert!(
            (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) < 0.0001,
            "{a:?} != {b:?}"
        );
    }

    #[test]
    fn restrictions_are_exact_cubics_in_either_direction() {
        let c = [[3., 1.], [-5., 10.], [13., 5.], [7., 1.]];
        for (start, end) in [(0., 0.34), (0.21, 0.91), (0.91, 0.21), (1., 0.)] {
            let sub = subcurve(c, start, end);
            for i in 0..=64 {
                let t = i as f64 / 64.;
                let a = at(sub, t);
                let b = at(c, start + t * (end - start));
                assert!(a.iter().zip(b).all(|(x, y)| (*x - y).abs() < 1e-12));
            }
        }
    }

    #[test]
    fn reversed_redrawn_open_pose_preserves_endpoints_and_corners() {
        let a = path(&[[0., 0.], [1., 0.], [2., 3.]], false);
        let b = path(&[[12., 5.], [11., 2.], [10.5, 2.], [10., 2.]], false);
        let plan = MorphPlan::build(&a, &b, &[]).unwrap();
        let mid = plan.sample(0.5);
        let nodes = &mid.contours()[0];
        close(nodes[0].point, [5., 1.]);
        close(nodes.last().unwrap().point, [7., 4.]);
        assert!(nodes
            .iter()
            .any(|n| (n.point[0] - 6.).abs() < 0.001 && (n.point[1] - 1.).abs() < 0.001));
        assert!(nodes.len() <= 6);
    }

    #[test]
    fn closed_seam_matches_rotated_reversed_outline() {
        let a = path(&[[0., 0.], [2., 0.], [3., 2.], [1., 5.]], true);
        let b = path(&[[13., 5.], [12., 3.], [10., 3.], [11., 8.]], true);
        let plan = MorphPlan::build(&a, &b, &[]).unwrap();
        assert_eq!(plan.knots().len(), 4);
        for (n, orig) in plan.sample(0.5).contours()[0].iter().zip(&a.contours()[0]) {
            close(n.point, [orig.point[0] + 5., orig.point[1] + 1.5]);
        }
    }

    #[test]
    fn a_single_off_node_guide_preserves_a_filled_outlines_winding() {
        let a = path(&[[0.1, 0.8], [0.35, 0.1], [0.9, 0.7]], true);
        let b = path(
            &[
                [0., 0.5],
                [0.5, 0.36],
                [0.72, 0.03],
                [1., 0.4],
                [0.76, 0.92],
                [0.25, 0.9],
            ],
            true,
        );
        let guide = MorphGuide {
            from: point(0, 0.35),
            to: point(1, 0.6),
        };
        let plan = MorphPlan::build(&a, &b, &[guide]).unwrap();
        let start = Outline::new(&a).unwrap().winding;
        for t in [0.3, 0.5, 1.] {
            assert_eq!(Outline::new(&plan.sample(t)).unwrap().winding, start);
        }

        // The artist may explicitly pair landmarks in the opposite direction.
        let reversed = [0, 2, 1]
            .into_iter()
            .enumerate()
            .map(|(i, j)| MorphGuide {
                from: point(i, 0.),
                to: point(j, 0.),
            })
            .collect::<Vec<_>>();
        let forced = MorphPlan::build(&a, &a, &reversed).unwrap();
        assert_eq!(
            Outline::new(&forced.sample(1.)).unwrap().winding,
            start.map(|v| -v)
        );
    }

    #[test]
    fn interior_guides_land_exactly_and_knots_survive_splitting_a_span() {
        let mut a = path(&[[0., 0.], [4., 2.], [6., 0.]], false);
        a.contours_mut()[0][0].outgoing = [2., 4.];
        a.contours_mut()[0][1].incoming = [1., -2.];
        let mut b = path(&[[7., 3.], [6., 1.], [4., 2.], [1., 4.]], false);
        b.contours_mut()[0][2].outgoing = [7., 6.];
        let guides = [
            MorphGuide {
                from: point(1, 0.7),
                to: point(0, 0.31),
            },
            MorphGuide {
                from: point(0, 0.44),
                to: point(2, 0.62),
            },
        ];
        let plan = MorphPlan::build(&a, &b, &guides).unwrap();
        let mid = plan.sample(0.5);
        for (i, g) in plan.sample_guides().iter().enumerate() {
            let p = guides[i].from.position(&a).unwrap();
            let q = guides[i].to.position(&b).unwrap();
            close(
                g.position(&mid).unwrap(),
                [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5],
            );
        }
        let knots = plan.knots();
        let left: Vec<_> = knots
            .iter()
            .enumerate()
            .map(|(i, &(source, _))| MorphGuide {
                from: source,
                to: if i + 1 == knots.len() {
                    point(i - 1, 1.)
                } else {
                    point(i, 0.)
                },
            })
            .collect();
        let split = MorphPlan::build(&a, &mid, &left).unwrap();
        let quarter = plan.sample(0.25);
        let rebuilt = split.sample(0.5);
        assert_eq!(quarter.contours()[0].len(), rebuilt.contours()[0].len());
        for (x, y) in quarter.contours()[0].iter().zip(&rebuilt.contours()[0]) {
            close(x.point, y.point);
            close(x.incoming, y.incoming);
            close(x.outgoing, y.outgoing);
        }
    }

    #[test]
    fn conflicting_out_of_range_and_crossing_guides_are_rejected() {
        let closed = path(&[[0., 0.], [1., 0.], [1., 1.], [0., 2.]], true);
        let open = path(&[[0., 0.], [0., 1.]], false);
        let crossing = [0, 2, 1, 3]
            .into_iter()
            .enumerate()
            .map(|(i, j)| MorphGuide {
                from: point(i, 0.),
                to: point(j, 0.),
            })
            .collect::<Vec<_>>();
        assert!(MorphPlan::build(&closed, &closed, &crossing).is_err());
        assert!(MorphPlan::build(&closed, &open, &[]).is_err());
        for (a, b) in [
            (point(0, 0.), point(0, 0.5)),
            (point(3, 0.2), point(0, 0.2)),
            (point(0, f32::NAN), point(0, 0.3)),
        ] {
            assert!(MorphPlan::build(&open, &open, &[MorphGuide { from: a, to: b }]).is_err());
        }
        assert!(MorphPlan::build(&closed, &closed, &[crossing[0], crossing[0]]).is_err());
    }

    #[test]
    fn collapsed_pose_is_finite_and_accepts_guides() {
        let a = path(&[[0., 0.]; 4], true);
        let b = path(&[[0., 0.], [1., 0.], [1., 1.]], true);
        let guide = MorphGuide {
            from: point(2, 0.5),
            to: point(1, 0.7),
        };
        let plan = MorphPlan::build(&a, &b, &[guide]).unwrap();
        for n in &plan.sample(0.5).contours()[0] {
            assert!([n.point, n.incoming, n.outgoing]
                .into_iter()
                .flatten()
                .all(f32::is_finite));
        }
    }
}
