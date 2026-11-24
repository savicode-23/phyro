use super::{delta, length, line, within_error, Cubic, Point};
use crate::bezier::{interpolate, split_cubic_at};

const CERTIFICATION_DEPTH: u8 = 4;
const CURVED_SPAN_SAMPLES: usize = 4;
const REFINEMENTS: usize = 2;
const MIN_SUSTAINED_BEND: f64 = 0.08;

struct Interval {
    source: Cubic,
    end: f64,
}

fn dot([x, y]: Point, [u, v]: Point) -> f64 {
    x * u + y * v
}

fn cross([x, y]: Point, [u, v]: Point) -> f64 {
    x * v - y * u
}

fn forward(curve: &Cubic, chord: Point, display: &impl Fn(Point) -> Point) -> bool {
    curve
        .windows(2)
        .all(|pair| dot(display(delta(pair[0], pair[1])), chord) >= 0.)
}

fn line_fits(
    source: &[Cubic],
    first: Point,
    chord: Point,
    chord_sq: f64,
    display: &impl Fn(Point) -> Point,
    limit_sq: f64,
) -> bool {
    source.iter().all(|span| {
        // Project the controls onto the chord. The resulting cubics stay on
        // that segment and together cover its full length.
        let difference = span.map(|point| {
            let offset = display(delta(first, point));
            let t = (dot(offset, chord) / chord_sq).clamp(0., 1.);
            std::array::from_fn(|axis| offset[axis] - chord[axis] * t)
        });
        within_error(difference, limit_sq, CERTIFICATION_DEPTH)
    })
}

fn sustained_bend(
    source: &[Cubic],
    first: Point,
    chord: Point,
    display: &impl Fn(Point) -> Point,
) -> bool {
    let min_bend = MIN_SUSTAINED_BEND * length(chord);
    if source.len() < 4
        || !source[..source.len() - 1]
            .iter()
            .any(|span| cross(chord, display(delta(first, span[3]))).abs() > min_bend)
    {
        return false;
    }

    let direction = |span: &Cubic| display(delta(span[0], span[3]));
    let mut previous = direction(&source[0]);
    let mut sign = None;
    let mut turns = 0;
    for span in &source[1..] {
        let next = direction(span);
        let turn = cross(previous, next);
        if turn.abs() > 1e-4 * length(previous) * length(next) {
            let current_sign = turn.is_sign_positive();
            if sign.is_some_and(|previous_sign| previous_sign != current_sign) {
                return false;
            }
            sign = Some(current_sign);
            turns += 1;
        }
        previous = next;
    }
    turns >= 3
}

fn sample(source: &[Cubic], display: &impl Fn(Point) -> Point) -> Option<Vec<Interval>> {
    let mut intervals = Vec::with_capacity(source.len());
    let mut distance = 0.;
    for &span in source {
        let count = if span == line(span[0], span[3]) {
            1
        } else {
            CURVED_SPAN_SAMPLES
        };
        let mut remainder = span;
        for left in (1..=count).rev() {
            let section = if left == 1 {
                remainder
            } else {
                let [section, rest] = split_cubic_at(remainder, 1. / left as f64);
                remainder = rest;
                section
            };
            distance += length(display(delta(section[0], section[3])));
            intervals.push(Interval {
                source: section,
                end: distance,
            });
        }
    }
    if !(distance > 0. && distance.is_finite()) {
        return None;
    }
    for interval in &mut intervals {
        interval.end /= distance;
    }
    Some(intervals)
}

fn least_squares(first: Point, last: Point, intervals: &[Interval]) -> Option<Cubic> {
    // Solve the two free control points against the cubic Bernstein basis.
    // Relative coordinates avoid subtracting large endpoint contributions.
    let chord = delta(first, last);
    let (mut aa, mut ab, mut bb) = (0., 0., 0.);
    let (mut ax, mut bx) = ([0.; 2], [0.; 2]);
    for interval in intervals {
        let t = interval.end;
        let u = 1. - t;
        let (a, b) = (3. * u * u * t, 3. * u * t * t);
        let target = delta(first, interval.source[3]);
        aa += a * a;
        ab += a * b;
        bb += b * b;
        for axis in 0..2 {
            let residual = target[axis] - chord[axis] * t * t * t;
            ax[axis] += a * residual;
            bx[axis] += b * residual;
        }
    }
    let determinant = aa * bb - ab * ab;
    if determinant <= 1e-10 * aa * bb {
        return None;
    }
    let point = |a: Point, b: Point, diagonal| {
        std::array::from_fn(|axis| {
            let value = first[axis] + (diagonal * a[axis] - ab * b[axis]) / determinant;
            // Certify the same handle coordinates that PathNode will store.
            f64::from(value as f32)
        })
    };
    Some([first, point(ax, bx, bb), point(bx, ax, aa), last])
}

fn fits(
    candidate: Cubic,
    intervals: &[Interval],
    display: &impl Fn(Point) -> Point,
    limit_sq: f64,
) -> bool {
    let mut start = 0.;
    let mut remainder = candidate;
    intervals.iter().all(|interval| {
        let end = interval.end;
        let section = if end == 1. {
            let section = remainder;
            remainder = [candidate[3]; 4];
            section
        } else {
            let [section, rest] = split_cubic_at(remainder, (end - start) / (1. - start));
            remainder = rest;
            section
        };
        start = end;
        let difference = std::array::from_fn(|i| display(delta(section[i], interval.source[i])));
        within_error(difference, limit_sq, CERTIFICATION_DEPTH)
    })
}

fn refine(intervals: &mut [Interval], curve: Cubic, display: &impl Fn(Point) -> Point) {
    let [a, b, c, d] = curve;
    let [v0, v1, v2] = [delta(a, b), delta(b, c), delta(c, d)].map(display);
    let [a0, a1] = [delta(v0, v1), delta(v1, v2)];
    let mut previous = 0.;
    for i in 0..intervals.len() - 1 {
        let interval = &intervals[i];
        let t = interval.end;
        let source = interval.source[3];
        let error = display(delta(source, split_cubic_at(curve, t)[0][3]));
        let velocity = interpolate(interpolate(v0, v1, t), interpolate(v1, v2, t), t);
        let acceleration = interpolate(a0, a1, t);
        let denominator = 3. * dot(velocity, velocity) + 2. * dot(error, acceleration);
        if denominator > 0. {
            // A closest-point step adjusts speed along the curve while keeping
            // each sample in order. Certification still checks every interval.
            let refined =
                (t - dot(error, velocity) / denominator).clamp(previous, intervals[i + 1].end);
            let new_error = display(delta(source, split_cubic_at(curve, refined)[0][3]));
            if dot(new_error, new_error) < dot(error, error) {
                intervals[i].end = refined;
            }
        }
        previous = intervals[i].end;
    }
}

/// Fit an ordered run to one segment, fixing its first and last anchor.
/// The caller limits the number of source spans and splits at authored corners.
pub(super) fn cubic(
    source: &[Cubic],
    display: &impl Fn(Point) -> Point,
    tolerance: f64,
) -> Option<Cubic> {
    let first = source.first()?[0];
    let last = source.last()?[3];
    let chord = display(delta(first, last));
    let chord_sq = dot(chord, chord);
    if chord_sq == 0. || !source.iter().all(|span| forward(span, chord, display)) {
        return None;
    }
    let limit_sq = tolerance * tolerance;
    let straight =
        line_fits(source, first, chord, chord_sq, display, limit_sq).then(|| line(first, last));
    if straight.is_some() && !sustained_bend(source, first, chord, display) {
        return straight;
    }

    let Some(mut intervals) = sample(source, display) else {
        return straight;
    };
    for step in 0..=REFINEMENTS {
        let Some(candidate) = least_squares(first, last, &intervals) else {
            break;
        };
        if forward(&candidate, chord, display) && fits(candidate, &intervals, display, limit_sq) {
            return Some(candidate);
        }
        if step < REFINEMENTS {
            refine(&mut intervals, candidate, display);
        }
    }
    straight
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_an_arc_of_short_straight_spans_without_overshooting() {
        let points: [Point; 17] = std::array::from_fn(|i| {
            let angle = std::f64::consts::FRAC_PI_2 * i as f64 / 16.;
            [angle.cos(), angle.sin()]
        });
        let source: Vec<_> = points.windows(2).map(|p| line(p[0], p[1])).collect();
        let fitted = cubic(&source, &|p| p.map(|c| c * 100.), 0.75).unwrap();
        assert_eq!((fitted[0], fitted[3]), (points[0], points[16]));
        assert_ne!(fitted, line(points[0], points[16]));
        for i in 0..=32 {
            let p = split_cubic_at(fitted, f64::from(i) / 32.)[0][3];
            assert!((100. * (length(p) - 1.)).abs() < 0.75, "{p:?}");
        }
    }

    #[test]
    fn prefers_a_straight_segment_and_preserves_traversal_direction() {
        let identity = |p| p;
        let a = [0., 0.];
        let b = [25., 0.2];
        let c = [100., 0.];
        assert_eq!(
            cubic(&[line(a, b), line(b, c)], &identity, 0.75),
            Some(line(a, c)),
        );
        let reversal = [line(a, b), line(b, a), line(a, c)];
        assert!(cubic(&reversal, &identity, 0.75).is_none());
    }

    #[test]
    fn preserves_a_shallow_sustained_arc_and_straightens_small_alternating_jitter() {
        let points: [Point; 33] = std::array::from_fn(|i| {
            let angle = 0.2 * i as f64 / 32. - 0.1;
            [100. * angle.cos(), 100. * angle.sin()]
        });
        let source: Vec<_> = points.windows(2).map(|p| line(p[0], p[1])).collect();
        let (first, last) = (points[0], points[32]);
        let chord = delta(first, last);
        let identity = |p| p;
        assert!(line_fits(
            &source,
            first,
            chord,
            dot(chord, chord),
            &identity,
            0.75_f64.powi(2)
        ));
        let smooth = cubic(&source, &identity, 0.75).unwrap();
        assert_ne!(smooth, line(first, last));
        assert!((split_cubic_at(smooth, 0.5)[0][3][0] - 100.).abs() < 0.05);

        let points = [[0., 0.], [10., 0.2], [20., -0.2], [30., 0.2], [40., 0.]];
        let jitter: Vec<_> = points.windows(2).map(|p| line(p[0], p[1])).collect();
        assert_eq!(
            cubic(&jitter, &identity, 0.75),
            Some(line(points[0], points[4]))
        );
    }
}
