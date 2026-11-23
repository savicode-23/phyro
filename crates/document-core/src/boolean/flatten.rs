use super::planar::{Contour, Point, GRID, LIMIT};
use super::COMPLEXITY;
pub(super) use crate::bezier::{distance_sq, quadratic, split_cubic};
use tiny_skia_path::{Path, PathSegment};

pub(super) const TOLERANCE: f64 = 0.02;
type Coord = [f64; 2];
fn world(point: tiny_skia_path::Point) -> Coord {
    [point.x.into(), point.y.into()]
}

fn push(points: &mut Vec<Point>, p: Coord) -> Result<(), String> {
    if points.len() >= 4096 {
        return Err(COMPLEXITY.into());
    }
    let scaled = p.map(|c| (c * GRID).round());
    if scaled
        .iter()
        .any(|v| !v.is_finite() || v.abs() > LIMIT as f64)
    {
        return Err(
            "Path coordinates must lie within 131072 pixels of the artboard origin.".into(),
        );
    }
    let p = scaled.map(|c| c as i64);
    if points.last() != Some(&p) {
        points.push(p);
    }
    Ok(())
}

// The convex hull of a Bézier stays within the chord's tolerance capsule
// once every control point does. Straight edges retain their original vertices.
pub(super) fn sample_cubic(
    controls: [Coord; 4],
    mut emit: impl FnMut(Coord, f64) -> Result<(), String>,
) -> Result<(), String> {
    subdivide(controls, [0., 1.], 0, &mut emit)
}

fn subdivide(
    [a, b, c, d]: [Coord; 4],
    span: [f64; 2],
    depth: u32,
    emit: &mut impl FnMut(Coord, f64) -> Result<(), String>,
) -> Result<(), String> {
    const ERROR_SQ: f64 = TOLERANCE * TOLERANCE;
    if distance_sq(b, a, d) <= ERROR_SQ && distance_sq(c, a, d) <= ERROR_SQ {
        return emit(d, span[1]);
    }
    if depth >= 24 {
        return Err(COMPLEXITY.into());
    }
    let [left, right] = split_cubic([a, b, c, d]);
    let mid = (span[0] + span[1]) * 0.5;
    subdivide(left, [span[0], mid], depth + 1, emit)?;
    subdivide(right, [mid, span[1]], depth + 1, emit)
}

fn finish(
    result: &mut Vec<Contour>,
    points: &mut Vec<Point>,
    closed: bool,
    remaining: &mut usize,
) -> Result<(), String> {
    if points.is_empty() {
        return Ok(());
    }
    if closed && points.last() == points.first() {
        points.pop();
    }
    if points.len() < if closed { 3 } else { 2 } {
        return Err("A contour collapses at the boolean precision of 1/1024 pixel.".into());
    }
    *remaining = remaining.checked_sub(points.len()).ok_or(COMPLEXITY)?;
    result.push(Contour {
        points: std::mem::take(points),
        closed,
    });
    Ok(())
}

pub(super) fn contours(path: &Path, remaining: &mut usize) -> Result<Vec<Contour>, String> {
    let mut paths = Vec::new();
    let mut points = Vec::new();
    let mut last = [0.; 2];
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                finish(&mut paths, &mut points, false, remaining)?;
                last = world(p);
                push(&mut points, last)?;
            }
            PathSegment::LineTo(p) => {
                last = world(p);
                push(&mut points, last)?;
            }
            PathSegment::QuadTo(c, p) => {
                let (c, end) = (world(c), world(p));
                sample_cubic(quadratic(last, c, end), |p, _| push(&mut points, p))?;
                last = end;
            }
            PathSegment::CubicTo(b, c, p) => {
                let end = world(p);
                sample_cubic([last, world(b), world(c), end], |p, _| push(&mut points, p))?;
                last = end;
            }
            PathSegment::Close => finish(&mut paths, &mut points, true, remaining)?,
        }
    }
    finish(&mut paths, &mut points, false, remaining)?;
    if paths.is_empty() {
        return Err("No path geometry is present.".into());
    }
    Ok(paths)
}
