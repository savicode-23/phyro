type Coord = [f64; 2];

pub(crate) fn interpolate(a: Coord, b: Coord, t: f64) -> Coord {
    std::array::from_fn(|i| (1. - t) * a[i] + t * b[i])
}

pub(crate) fn split_cubic(curve: [Coord; 4]) -> [[Coord; 4]; 2] {
    split_cubic_at(curve, 0.5)
}

pub(crate) fn split_cubic_at([a, b, c, d]: [Coord; 4], t: f64) -> [[Coord; 4]; 2] {
    let (ab, bc, cd) = (
        interpolate(a, b, t),
        interpolate(b, c, t),
        interpolate(c, d, t),
    );
    let (abc, bcd) = (interpolate(ab, bc, t), interpolate(bc, cd, t));
    let center = interpolate(abc, bcd, t);
    [[a, ab, abc, center], [center, bcd, cd, d]]
}

pub(crate) fn distance_sq(p: Coord, a: Coord, b: Coord) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx * dx + dy * dy;
    let t = if len == 0. {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0., 1.)
    };
    (p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)
}

pub(crate) fn quadratic(a: Coord, c: Coord, end: Coord) -> [Coord; 4] {
    let b = std::array::from_fn(|i| (a[i] + 2. * c[i]) / 3.);
    let d = std::array::from_fn(|i| (end[i] + 2. * c[i]) / 3.);
    [a, b, d, end]
}
