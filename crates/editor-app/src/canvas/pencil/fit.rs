use super::{simplify_indices, HandleStyle, PathNode, Pos2, Vec2};

// Fit the filtered samples by arc length. Knots share a tangent across smooth
// joins; sharp turns keep independent sides and the exact contact position.
struct Samples<'a> {
    points: &'a [Pos2],
    distance: Vec<f64>,
    closed: bool,
    support: f64,
    tolerance: f32,
    noise_floor: f32,
}

impl Samples<'_> {
    fn at_distance(&self, distance: f64) -> Pos2 {
        let last = self.points.len() - 1;
        let distance = if self.closed {
            distance.rem_euclid(self.distance[last])
        } else {
            distance.clamp(0., self.distance[last])
        };
        let i = self.distance.partition_point(|&d| d < distance);
        if i == 0 {
            return self.points[0];
        }
        let u = (distance - self.distance[i - 1]) / (self.distance[i] - self.distance[i - 1]);
        self.points[i - 1].lerp(self.points[i], u as f32)
    }

    fn directions(&self, index: usize) -> (Vec2, Vec2) {
        let point = self.points[index];
        let distance = self.distance[index];
        (
            (point - self.at_distance(distance - self.support)).normalized(),
            (self.at_distance(distance + self.support) - point).normalized(),
        )
    }

    fn tangent(&self, index: usize) -> Vec2 {
        let (a, b) = self.directions(index);
        if a != Vec2::ZERO && b != Vec2::ZERO && a.dot(b) < 0.35 {
            Vec2::ZERO
        } else {
            (a + b).normalized()
        }
    }

    fn span(&self, start: usize, end: usize, a: Vec2, b: Vec2) -> (Vec2, Vec2, Option<usize>) {
        let p = self.points[start];
        let chord = self.points[end] - p;
        let length = self.distance[end] - self.distance[start];
        let parameter = |i: usize| ((self.distance[i] - self.distance[start]) / length) as f32;
        if end == start + 1 {
            // No interior samples to constrain the curve. Keep its control
            // polygon inside a tolerance-wide strip along this measured edge.
            let axis = chord.normalized();
            let handle = |tangent: Vec2| {
                if tangent.dot(axis) <= 0. {
                    return Vec2::ZERO;
                }
                let sine = (tangent.x * axis.y - tangent.y * axis.x).abs();
                tangent * (length as f32 / 3.).min(self.tolerance / sine)
            };
            return (handle(a), -handle(b), None);
        }
        let basis = |t: f32| {
            let s = 1. - t;
            let (u, v) = (3. * s * s * t, 3. * s * t * t);
            (u, v, chord * (v + t * t * t))
        };
        // Least squares for the two handle lengths, keeping endpoint tangents fixed.
        // Weight by distance so a burst of tablet events cannot dominate a span.
        let (mut aa, mut ab, mut bb, mut ax, mut bx) = (0., 0., 0., 0., 0.);
        let axis = chord.normalized();
        let mut bend = 0_f32;
        for i in start + 1..end {
            let (u, v, base) = basis(parameter(i));
            let (x, y) = (u * a, -v * b);
            let offset = self.points[i] - p;
            bend = bend.max((offset.x * axis.y - offset.y * axis.x).abs());
            let error = offset - base;
            let w = ((self.distance[i + 1] - self.distance[i - 1]) / length) as f32;
            aa += w * x.length_sq();
            ab += w * x.dot(y);
            bb += w * y.length_sq();
            ax += w * x.dot(error);
            bx += w * y.dot(error);
        }
        let det = aa * bb - ab * ab;
        let fallback = length as f32 / 3.;
        let (left, right) = if det > 1e-6 * aa * bb {
            ((bb * ax - ab * bx) / det, (aa * bx - ab * ax) / det)
        } else {
            (
                if aa > 0. {
                    (ax - ab * fallback) / aa
                } else {
                    fallback
                },
                if bb > 0. {
                    (bx - ab * fallback) / bb
                } else {
                    fallback
                },
            )
        };
        let maximum = 0.75 * length as f32;
        let left = a * left.clamp(0., maximum);
        let right = -b * right.clamp(0., maximum);
        let mut worst = 0.;
        let mut split = start;
        let at = |t| {
            let (u, v, base) = basis(t);
            base + u * left + v * right
        };
        for i in start + 1..=end {
            let t = parameter(i);
            let mut error = (self.points[i] - p - at(t)).length_sq();
            let step = (self.distance[i] - self.distance[i - 1]) as f32;
            if step > 4. * self.tolerance {
                // Check the two interior controls of this subcurve against
                // the measured edge. The Bézier lies inside their convex hull.
                let tangent = |t: f32| {
                    let s = 1. - t;
                    s * s * left + 2. * s * t * (chord + right - left) - t * t * right
                };
                let t0 = parameter(i - 1);
                let from = self.points[i - 1] - p;
                let edge = self.points[i] - self.points[i - 1];
                for control in [
                    at(t0) + (t - t0) * tangent(t0),
                    at(t) - (t - t0) * tangent(t),
                ] {
                    let offset = control - from;
                    let fraction = (offset.dot(edge) / edge.length_sq()).clamp(0., 1.);
                    error = error.max((offset - fraction * edge).length_sq());
                }
            }
            if error > worst {
                worst = error;
                split = i.min(end - 1);
            }
        }
        // A long shallow bow needs a tighter relative error than a broad
        // sweep. Keep a floor so subpixel hand jitter does not create knots.
        let tolerance = (0.18 * bend).clamp(self.noise_floor, self.tolerance);
        (
            left,
            right,
            (worst > tolerance * tolerance).then_some(split),
        )
    }
}

pub(super) fn nodes(
    points: &[Pos2],
    closed: bool,
    tolerance: f32,
    scale: f32,
) -> Option<Vec<PathNode>> {
    // A closed input repeats the start point at the end for fitting the seam.
    if points.len() < if closed { 4 } else { 2 } {
        return None;
    }
    let last = points.len() - 1;
    let mut distance = Vec::with_capacity(points.len());
    distance.push(0.);
    for pair in points.windows(2) {
        distance.push(distance.last().unwrap() + f64::from(pair[0].distance(pair[1])));
    }
    let total = distance[last];
    if total <= 0. || !total.is_finite() {
        return None;
    }
    let samples = Samples {
        points,
        distance,
        closed,
        support: f64::from(1. / scale + tolerance).min(total / 8.),
        tolerance,
        noise_floor: tolerance.min(1.9 / scale),
    };
    // Detect sustained turns before fitting, so a cubic cannot cut across a
    // corner simply because the samples on either straight arm are sparse.
    let mut breaks: Vec<_> = simplify_indices(points, tolerance)
        .into_iter()
        .filter(|&i| i == 0 || i == last || samples.tangent(i) == Vec2::ZERO)
        .collect();
    if closed {
        // Native closed paths need three distinct anchors even for a loop
        // that two half-circle cubics would already fit within tolerance.
        while breaks.len() < 4 {
            let gap = breaks
                .windows(2)
                .enumerate()
                .filter(|(_, w)| w[1] > w[0] + 1)
                .max_by(|(_, a), (_, b)| {
                    let length = |w: &[usize]| samples.distance[w[1]] - samples.distance[w[0]];
                    length(a).total_cmp(&length(b))
                })?
                .0;
            let (a, b) = (breaks[gap], breaks[gap + 1]);
            let half = (samples.distance[a] + samples.distance[b]) * 0.5;
            let mid = samples
                .distance
                .partition_point(|&d| d < half)
                .clamp(a + 1, b - 1);
            breaks.insert(gap + 1, mid);
        }
    }
    let mut nodes = vec![PathNode::corner(points[0].into())];
    let mut spans: Vec<_> = breaks.windows(2).rev().map(|w| (w[0], w[1])).collect();
    while let Some((start, end)) = spans.pop() {
        let a = samples.tangent(start);
        let b = samples.tangent(end);
        let (left, right, split) = samples.span(start, end, a, b);
        if let Some(mut split) = split {
            // Like the acquisition simplifier, bound depth for adversarial scribbles.
            let width = end - start;
            if width > 96 && (split - start).min(end - split) < width / 8 {
                split = start + width / 2;
            }
            spans.push((split, end));
            spans.push((start, split));
        } else {
            let previous = nodes.last_mut().unwrap();
            previous.outgoing = (points[start] + left).into();
            if (closed || start > 0) && a != Vec2::ZERO {
                previous.handle_style = HandleStyle::Smooth;
            }
            let mut next = PathNode::corner(points[end].into());
            next.incoming = (points[end] + right).into();
            nodes.push(next);
        }
    }
    if closed {
        nodes[0].incoming = nodes.pop().unwrap().incoming;
    }
    Some(nodes)
}
