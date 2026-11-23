use super::{Pos2, Rect};

type Point = [f64; 2];

fn lerp(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn sample([a, b, c, d]: [Point; 4], t: f64) -> Point {
    let ab = lerp(a, b, t);
    let bc = lerp(b, c, t);
    let cd = lerp(c, d, t);
    lerp(lerp(ab, bc, t), lerp(bc, cd, t), t)
}

pub(super) fn on_cubic(
    controls: [Pos2; 4],
    point: Pos2,
    radius: f32,
    tolerance: f32,
) -> Option<(f32, f32)> {
    let [a, b, c, d] = controls;
    let bounds = Rect::from_min_max(a.min(b).min(c).min(d), a.max(b).max(c).max(d));
    if !bounds.expand(radius).contains(point) || !controls.iter().all(|p| p.is_finite()) {
        return None;
    }
    // Subdivide relative to the pointer in double precision. Adjacent handles
    // can be far apart on the pasteboard even while the artist is zoomed in.
    let controls = controls.map(|p| {
        [
            f64::from(p.x) - f64::from(point.x),
            f64::from(p.y) - f64::from(point.y),
        ]
    });
    let mut best = (f64::from(radius), 0.);
    search(controls, [0., 1.], f64::from(tolerance), &mut best);
    if best.0 >= f64::from(radius) {
        return None;
    }
    // Splitting uses the representable authored parameter.
    let t = best.1 as f32;
    let [x, y] = sample(controls, f64::from(t));
    let distance = x.hypot(y);
    (distance < f64::from(radius)).then_some((distance as f32, t))
}

// The squared distance from the pointer is a degree-six Bézier polynomial.
// Its seven coefficients bound the entire curve, including tangent near misses.
fn outside_radius([a, b, c, d]: [Point; 4], radius: f64) -> bool {
    let dot = |p: Point, q: Point| p[0] * q[0] + p[1] * q[1];
    let [aa, bb, cc, dd] = [a, b, c, d].map(|p| dot(p, p));
    let squared = radius * radius;
    let limit = squared + 32. * f64::EPSILON * aa.max(bb).max(cc).max(dd).max(squared);
    aa > limit
        && dot(a, b) > limit
        && 2. * dot(a, c) + 3. * bb > 5. * limit
        && dot(a, d) + 9. * dot(b, c) > 10. * limit
        && 2. * dot(b, d) + 3. * cc > 5. * limit
        && dot(c, d) > limit
        && dd > limit
}

fn search(controls: [Point; 4], [start, end]: [f64; 2], tolerance: f64, best: &mut (f64, f64)) {
    let [a, b, c, d] = controls;
    // A cubic stays in its control hull. Skip intervals that cannot improve the best hit.
    if (0..2)
        .any(|i| controls.iter().all(|p| p[i] > best.0) || controls.iter().all(|p| p[i] < -best.0))
    {
        return;
    }
    // A chord with these two interpolated controls has exactly linear timing.
    // Bounding both deviations bounds error at the same curve parameter, even
    // on a straight edge whose handles coincide with its endpoints.
    let first = lerp(a, d, 1. / 3.);
    let last = lerp(a, d, 2. / 3.);
    let flat = (0..2).all(|i| (b[i] - first[i]).abs().max((c[i] - last[i]).abs()) <= tolerance);
    let terminal = end - start <= f64::from(f32::EPSILON) / 8.;
    if flat || terminal {
        let (x, y) = (d[0] - a[0], d[1] - a[1]);
        let length_sq = x * x + y * y;
        let u = if length_sq > 0. {
            (-(a[0] * x + a[1] * y) / length_sq).clamp(0., 1.)
        } else {
            0.
        };
        let chord_point = lerp(a, d, u);
        let ab = lerp(a, b, u);
        let bc = lerp(b, c, u);
        let cd = lerp(c, d, u);
        let from = lerp(ab, bc, u);
        let to = lerp(bc, cd, u);
        let [x, y] = lerp(from, to, u);
        let mut distance = x.hypot(y);
        let mut parameter = u;
        // A projection onto the chord can just miss a click on the boundary
        // of the hit radius. Refine it using the tangent of the actual curve.
        let (dx, dy) = (3. * (to[0] - from[0]), 3. * (to[1] - from[1]));
        let speed_sq = dx * dx + dy * dy;
        if speed_sq > 0. {
            let refined = (u - (x * dx + y * dy) / speed_sq).clamp(0., 1.);
            let [rx, ry] = sample(controls, refined);
            let refined_distance = rx.hypot(ry);
            if refined_distance < distance {
                distance = refined_distance;
                parameter = refined;
            }
        }
        if distance < best.0 {
            *best = (distance, start + parameter * (end - start));
            return;
        }
        // At the edge of the hit radius a flat interval can still bow toward
        // the pointer. Refine only if its chord-error bound permits a closer hit.
        let error = (b[0] - first[0])
            .hypot(b[1] - first[1])
            .max((c[0] - last[0]).hypot(c[1] - last[1]));
        if terminal
            || error <= tolerance / 65_536.
            || chord_point[0].hypot(chord_point[1]) - error >= best.0
            || outside_radius(controls, best.0)
        {
            return;
        }
    }
    let ab = lerp(a, b, 0.5);
    let bc = lerp(b, c, 0.5);
    let cd = lerp(c, d, 0.5);
    let abc = lerp(ab, bc, 0.5);
    let bcd = lerp(bc, cd, 0.5);
    let middle = lerp(abc, bcd, 0.5);
    let half = (start + end) * 0.5;
    search([a, ab, abc, middle], [start, half], tolerance, best);
    search([middle, bcd, cd, d], [half, end], tolerance, best);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bowed_curves_and_collapsed_handles_accept_clicks_just_inside_the_radius() {
        let curved = [[0., 0.], [0., 60.], [60., 0.], [60., 60.]].map(Pos2::from);
        let straight = [[0., 0.], [0., 0.], [32., 0.], [32., 0.]].map(Pos2::from);
        for (curve, point) in [
            (curved, Pos2::new(-6.394402, 15.3505335)),
            (straight, Pos2::new(0.041743297, 7.99992)),
        ] {
            let (distance, t) = on_cubic(curve, point, 8., 0.125).unwrap();
            assert!(distance < 8. && t > 0. && t < 1.);
        }
        assert!(on_cubic(straight, Pos2::new(2., 8.001), 8., 0.125).is_none());
    }

    #[test]
    fn grazing_a_quarter_circle_respects_the_actual_curve_distance() {
        let tangent = 4. * (2.0_f32.sqrt() - 1.) / 3.;
        for radius in [7.99999, 8.00001] {
            let handle = tangent * radius;
            let controls = [
                [radius, 0.],
                [radius, handle],
                [handle, radius],
                [0., radius],
            ]
            .map(Pos2::from);
            assert_eq!(
                on_cubic(controls, Pos2::ZERO, 8., 0.125).is_some(),
                radius < 8.
            );
        }
    }
}
