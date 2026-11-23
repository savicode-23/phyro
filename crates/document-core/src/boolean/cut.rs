//! Split the back object's outline at isolated intersections with the cutter.
//! Filled overlays can discard interior edges, so Cut Path instead follows the
//! original contour order and retains both sides of each crossing.
use super::planar::{Contour, Point, GRID};
use super::spans::{CurveSpans, Span};
use super::{Ring, COMPLEXITY};
use crate::VectorObject;
use std::cmp::Ordering;
use tiny_skia_path::Path;

fn delta(a: Point, b: Point) -> Point {
    [b[0] - a[0], b[1] - a[1]]
}
fn cross(a: Point, b: Point) -> i128 {
    i128::from(a[0]) * i128::from(b[1]) - i128::from(a[1]) * i128::from(b[0])
}

#[derive(Clone, Copy)]
struct Fraction(i128, i128);
impl Fraction {
    fn new(n: i128, d: i128) -> Self {
        if d < 0 {
            Self(-n, -d)
        } else {
            Self(n, d)
        }
    }
    fn cmp(self, b: Self) -> Ordering {
        (self.0 * b.1).cmp(&(b.0 * self.1))
    }
    fn in_unit(self) -> bool {
        self.0 >= 0 && self.0 <= self.1
    }
    fn point(self, a: Point, b: Point) -> [f64; 2] {
        if self.0 == 0 {
            return a.map(|v| v as f64 / GRID);
        }
        if self.0 == self.1 {
            return b.map(|v| v as f64 / GRID);
        }
        std::array::from_fn(|i| {
            (i128::from(a[i]) * self.1 + i128::from(b[i] - a[i]) * self.0) as f64
                / self.1 as f64
                / GRID
        })
    }
}

fn intersection(a: Point, b: Point, c: Point, d: Point) -> Result<Option<Fraction>, String> {
    let (r, s, v) = (delta(a, b), delta(c, d), delta(a, c));
    let determinant = cross(r, s);
    if determinant != 0 {
        let t = Fraction::new(cross(v, s), determinant);
        let u = Fraction::new(cross(v, r), determinant);
        return Ok((t.in_unit() && u.in_unit()).then_some(t));
    }
    if cross(v, r) != 0 {
        return Ok(None);
    }
    let axis = usize::from(r[1].abs() > r[0].abs());
    let start = a[axis].min(b[axis]).max(c[axis].min(d[axis]));
    let end = a[axis].max(b[axis]).min(c[axis].max(d[axis]));
    match start.cmp(&end) {
        Ordering::Greater => Ok(None),
        Ordering::Equal => Ok(Some(Fraction::new(
            i128::from(start - a[axis]),
            i128::from(r[axis]),
        ))),
        Ordering::Less => {
            Err("Cut path needs isolated intersections. The selected paths share an edge.".into())
        }
    }
}

pub(super) fn edges(path: &Contour) -> impl Iterator<Item = (Point, Point)> + '_ {
    path.points
        .iter()
        .copied()
        .zip(path.points.iter().copied().cycle().skip(1))
        .take(if path.closed {
            path.points.len()
        } else {
            path.points.len() - 1
        })
}

#[derive(Clone, Copy)]
struct Vertex {
    point: [f64; 2],
    cut: bool,
    incoming: Option<Span>,
}

pub(super) fn objects(
    base: &VectorObject,
    frame: u32,
    source: &[Contour],
    cutter: &[Contour],
    outline: &Path,
) -> Result<Vec<VectorObject>, String> {
    let cutters: Vec<_> = cutter.iter().flat_map(edges).collect();
    let spans = CurveSpans::for_cut(outline, source);
    let make_piece = |vertices: &[Vertex], closed: bool| {
        let ring: Ring = vertices[..vertices.len() - usize::from(closed)]
            .iter()
            .map(|v| v.point)
            .collect();
        let mut object = super::path_object(base, frame, vec![ring], closed, |ring| {
            let spans = spans.as_ref()?;
            let edges: Vec<_> = vertices[1..].iter().map(|v| v.incoming).collect();
            spans.nodes_from_edges(ring, &edges, closed)
        })?;
        // Cut Path exposes centerlines. Give fill-only artwork a visible
        // hairline using its own color, so every piece can be selected.
        if !object.has_stroke_at(0) {
            object.no_stroke = false;
            object.stroke = object
                .gradient
                .as_ref()
                .and_then(|g| g.stops.first())
                .map_or(object.fill, |s| s.color);
            object.stroke_width = 1.;
            object.stroke_gradient = None;
        }
        object.no_fill = true;
        object.gradient = None;
        Ok::<_, String>(object)
    };
    let mut result = Vec::new();
    let mut any_cut = false;
    let mut cut_count = 0;
    for contour in source {
        let mut walk: Vec<Vertex> = Vec::new();
        for (a, b) in edges(contour) {
            let mut marks = vec![(Fraction(0, 1), false), (Fraction(1, 1), false)];
            for &(c, d) in &cutters {
                if let Some(t) = intersection(a, b, c, d)? {
                    marks.push((t, true));
                    cut_count += 1;
                    if cut_count > 65_536 {
                        return Err(COMPLEXITY.into());
                    }
                }
            }
            marks.sort_by(|a, b| a.0.cmp(b.0));
            marks.dedup_by(|a, b| {
                if a.0.cmp(b.0).is_eq() {
                    b.1 |= a.1;
                    true
                } else {
                    false
                }
            });
            let mut previous = None;
            for (t, cut) in marks {
                let point = t.point(a, b);
                if let Some(last) = walk.last_mut().filter(|v| v.point == point) {
                    last.cut |= cut;
                } else {
                    let incoming =
                        previous.and_then(|p| spans.as_ref()?.cut_span((a, b), p, point));
                    walk.push(Vertex {
                        point,
                        cut,
                        incoming,
                    });
                }
                previous = Some(point);
            }
        }
        if contour.closed {
            let last = walk.pop().expect("validated closed path");
            walk[0].cut |= last.cut;
            walk[0].incoming = last.incoming;
            if let Some(start) = walk.iter().position(|v| v.cut) {
                walk.rotate_left(start);
            }
            walk.push(walk[0]);
        }
        let mut first = 0;
        let mut cut_here = contour.closed && walk[0].cut;
        for (index, vertex) in walk.iter().enumerate().skip(1) {
            if vertex.cut && index + 1 < walk.len() {
                cut_here = true;
                result.push(make_piece(&walk[first..=index], false)?);
                first = index;
            }
        }
        any_cut |= cut_here;
        result.push(make_piece(&walk[first..], contour.closed && !cut_here)?);
    }
    if !any_cut {
        return Err("The cutter does not cross the selected outline.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
