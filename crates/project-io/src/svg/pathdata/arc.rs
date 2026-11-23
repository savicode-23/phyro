use super::{corner, finite, PathNode};
use std::{
    f64::consts::{FRAC_PI_2, TAU},
    io,
};

/// SVG endpoint-to-center conversion (SVG 2, implementation notes B.2).
/// Represent each quarter-turn or shorter arc with one editable cubic.
pub(super) fn append(
    nodes: &mut Vec<PathNode>,
    radii: [f32; 2],
    rotation: f32,
    large: bool,
    sweep: bool,
    end: [f32; 2],
) -> io::Result<()> {
    let start = nodes.last().unwrap().point;
    if start == end {
        return Ok(());
    }
    if radii.contains(&0.) {
        nodes.push(corner(end));
        return Ok(());
    }
    let [mut rx, mut ry] = radii.map(|v| f64::from(v).abs());
    let (sin, cos) = f64::from(rotation).to_radians().sin_cos();
    let [x0, y0] = start.map(f64::from);
    let [x1, y1] = end.map(f64::from);
    let (dx, dy) = ((x0 - x1) / 2., (y0 - y1) / 2.);
    let mut u = (cos * dx + sin * dy) / rx;
    let mut v = (-sin * dx + cos * dy) / ry;
    let chord = u.hypot(v);
    let offset = if chord >= 1. {
        rx *= chord;
        ry *= chord;
        u /= chord;
        v /= chord;
        0.
    } else {
        let sign = if large == sweep { -1. } else { 1. };
        sign * (1. - chord * chord).sqrt() / chord
    };
    let (cx, cy) = (offset * v, -offset * u);
    let first = (v - cy).atan2(u - cx);
    let short_arc = 2. * chord.min(1.).asin();
    let magnitude = if large { TAU - short_arc } else { short_arc };
    let angle = if sweep { magnitude } else { -magnitude };
    // Floating-point noise around exact quarter-turns should not add anchors.
    let parts = (magnitude / FRAC_PI_2 - 1e-12).ceil().max(1.) as usize;
    let step = angle / parts as f64;
    let handle = (4. / 3.) * (step / 4.).tan();
    let map = |x: f64, y: f64| [cos * rx * x - sin * ry * y, sin * rx * x + cos * ry * y];
    for i in 0..parts {
        let (start_sin, start_cos) = (first + step * i as f64).sin_cos();
        let (end_sin, end_cos) = (first + step * (i + 1) as f64).sin_cos();
        let point = if i + 1 == parts {
            end
        } else {
            let [ex, ey] = map(end_cos + cx, end_sin + cy);
            [
                finite(((x0 + x1) / 2. + ex) as f32)?,
                finite(((y0 + y1) / 2. + ey) as f32)?,
            ]
        };
        let prior = nodes.last_mut().unwrap();
        let outgoing = map(-start_sin * handle, start_cos * handle);
        let incoming = map(end_sin * handle, -end_cos * handle);
        for (axis, delta) in outgoing.into_iter().enumerate() {
            prior.outgoing[axis] = finite((f64::from(prior.point[axis]) + delta) as f32)?;
        }
        let mut node = corner(point);
        for (axis, delta) in incoming.into_iter().enumerate() {
            node.incoming[axis] = finite((f64::from(point[axis]) + delta) as f32)?;
        }
        nodes.push(node);
    }
    Ok(())
}
