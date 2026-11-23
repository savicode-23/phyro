//! Reconnect surviving integer-overlay edges to intervals of the source Béziers.
//! The overlay still decides the rings, winding, corner order and junctions.
mod geometry;

use super::{
    curves, flatten,
    planar::{Contour, Point, GRID},
    Piece, Ring,
};
use crate::PathNode;
use geometry::{exact_grid_point, snapped, transverse, unordered, Chord, Cubic, Edge, World};
use std::collections::{HashMap, HashSet};
use tiny_skia_path::{Path, PathSegment, PathVerb};

#[derive(Clone, Copy)]
pub(super) struct Span {
    curve: usize,
    start: f64,
    end: f64,
}
impl Span {
    fn follows(self, other: Self) -> bool {
        self.curve == other.curve && self.start == other.end
    }
    fn reversed(self) -> Self {
        Self {
            start: self.end,
            end: self.start,
            ..self
        }
    }
}

struct CrossingChord {
    end: Point,
    span: Span,
    neighbor: Edge,
}
struct ClippedSpan {
    span: Span,
    source: Edge,
    neighbor: Edge,
}

// Only nearby chords get entries. A missing entry is clear, while None
// means that several other boundaries approached this chord.
type Contacts = HashMap<Edge, Option<Edge>>;
fn contacts(contours: &[&Contour], margin: i64) -> Contacts {
    let mut chords: Vec<_> = contours
        .iter()
        .enumerate()
        .flat_map(|(owner, c)| {
            c.points
                .iter()
                .zip(c.points.iter().cycle().skip(1))
                .take(c.points.len() - usize::from(!c.closed))
                .map(move |(&a, &b)| (owner, Chord::new(a, b)))
        })
        .collect();
    chords.sort_unstable_by_key(|(_, c)| c.left());
    let mut result = HashMap::new();
    let mut record = |edge, neighbor| {
        result
            .entry(unordered(edge))
            .and_modify(|prior| {
                if *prior != Some(unordered(neighbor)) {
                    *prior = None
                }
            })
            .or_insert(Some(unordered(neighbor)));
    };
    for (i, (owner, first)) in chords.iter().enumerate() {
        for (other, second) in chords[i + 1..]
            .iter()
            .take_while(|(_, c)| c.left() <= first.right() + margin)
        {
            if owner != other && first.near(*second, margin) {
                record(first.edge, second.edge);
                record(second.edge, first.edge);
            }
        }
    }
    result
}
#[derive(Default)]
pub(super) struct CurveSpans<'a> {
    curves: Vec<Cubic>,
    clear_edges: HashMap<Edge, Span>,
    crossings: HashMap<Point, Vec<CrossingChord>>,
    straight_edges: HashSet<Edge>,
    contours: Vec<&'a Contour>,
}

impl<'a> CurveSpans<'a> {
    pub(super) fn collect(
        outlines: &[Path],
        inputs: &'a [Vec<Contour>],
        pieces: &[Piece],
    ) -> Option<Self> {
        if outlines.len() != inputs.len()
            || !outlines.iter().any(|path| {
                path.verbs()
                    .iter()
                    .any(|v| matches!(v, PathVerb::Cubic | PathVerb::Quad))
            })
        {
            return None;
        }
        let contours: Vec<_> = inputs.iter().flatten().collect();
        // Clearance checks can compare every source chord to every other.
        // Keep expensive or uncertified geometry on the existing polygon path.
        if contours.len() > curves::MAX_CONTOURS
            || contours.iter().map(|c| c.points.len()).sum::<usize>() > 2000
        {
            return None;
        }
        let mut result = Self {
            contours,
            ..Default::default()
        };
        if !pieces
            .iter()
            .flat_map(|(rings, _)| rings)
            .any(|ring| result.changed_ring(ring))
        {
            return None;
        }
        let max = result
            .contours
            .iter()
            .flat_map(|c| c.points.iter().flatten())
            .map(|v| v.abs())
            .max()?;
        let margin =
            (2. * flatten::TOLERANCE * GRID + 2. + 32. * f64::from(f32::EPSILON) * max as f64)
                .ceil() as i64;
        let neighbors = contacts(&result.contours, margin);
        for (path, input) in outlines.iter().zip(inputs) {
            result.record_straight_edges(path, input);
            if !curves::filled_nodes(path, input)
                .is_some_and(|contours| contours.iter().all(|nodes| curves::simple(nodes)))
            {
                continue;
            }
            result.trace_path(path, &neighbors)?;
        }
        Some(result)
    }

    fn changed_ring(&self, ring: &Ring) -> bool {
        // A cut can meet existing anchors at both ends (e.g. bisecting an
        // ellipse). Even with no new vertices, the output then follows a
        // different boundary. Unchanged rings use the isolated-contour check.
        !self
            .contours
            .iter()
            .any(|c| c.closed && curves::orientation(&c.points, ring).is_some())
    }

    fn record_straight_edges(&mut self, path: &Path, input: &[Contour]) {
        let mut contours = input.iter();
        let mut previous = [0; 2];
        let mut close_to = None;
        for segment in path.segments() {
            let (p, straight) = match segment {
                PathSegment::MoveTo(p) => {
                    if let Some(first) = close_to.take() {
                        self.straight_edges.insert(unordered((previous, first)));
                    }
                    previous = snapped([p.x.into(), p.y.into()]);
                    close_to = contours.next().filter(|c| c.closed).map(|_| previous);
                    continue;
                }
                PathSegment::LineTo(p) => (p, true),
                PathSegment::QuadTo(_, p) | PathSegment::CubicTo(_, _, p) => (p, false),
                PathSegment::Close => continue,
            };
            let next = snapped([p.x.into(), p.y.into()]);
            if straight {
                self.straight_edges.insert(unordered((previous, next)));
            }
            previous = next;
        }
        if let Some(first) = close_to {
            self.straight_edges.insert(unordered((previous, first)));
        }
    }

    fn trace_path(&mut self, path: &Path, neighbors: &Contacts) -> Option<()> {
        let coord = |p: tiny_skia_path::Point| [f64::from(p.x), f64::from(p.y)];
        let mut previous = [0.; 2];
        for segment in path.segments() {
            let controls = match segment {
                PathSegment::MoveTo(p) | PathSegment::LineTo(p) => {
                    previous = coord(p);
                    continue;
                }
                PathSegment::Close => continue,
                PathSegment::QuadTo(c, p) => flatten::quadratic(previous, coord(c), coord(p)),
                PathSegment::CubicTo(c, d, p) => [previous, coord(c), coord(d), coord(p)],
            };
            previous = controls[3];
            let curve = self.curves.len();
            self.curves.push(Cubic(controls));
            let mut point = snapped(controls[0]);
            let mut start = 0.;
            // Use the very same subdivision decisions that fed the integer solver.
            flatten::sample_cubic(controls, |p, end| {
                let next = snapped(p);
                if point != next {
                    let edge = (point, next);
                    self.record(
                        edge,
                        Span { curve, start, end },
                        neighbors.get(&unordered(edge)),
                    );
                }
                start = end;
                point = next;
                Ok(())
            })
            .ok()?;
        }
        Some(())
    }

    fn record(&mut self, (a, b): Edge, span: Span, contact: Option<&Option<Edge>>) {
        if let Some(&contact) = contact {
            let Some(neighbor) = contact else { return };
            if neighbor == unordered((a, b)) {
                return;
            }
            self.crossings.entry(a).or_default().push(CrossingChord {
                end: b,
                span,
                neighbor,
            });
            self.crossings.entry(b).or_default().push(CrossingChord {
                end: a,
                span: span.reversed(),
                neighbor,
            });
        } else {
            self.clear_edges.insert((a, b), span);
            self.clear_edges.insert((b, a), span.reversed());
        }
    }

    fn clipped_span(&self, from: World, junction: World) -> Option<ClippedSpan> {
        let start = exact_grid_point(from)?;
        let mut matches = self.crossings.get(&start)?.iter().filter_map(|chord| {
            let edge = (start, chord.end);
            let span = if exact_grid_point(junction) == Some(chord.end)
                && self.curves[chord.span.curve].at(chord.span.end) == junction
            {
                chord.span
            } else {
                self.curves[chord.span.curve].clipped(chord.span, edge, junction)?
            };
            Some(ClippedSpan {
                span,
                source: unordered(edge),
                neighbor: chord.neighbor,
            })
        });
        let found = matches.next()?;
        matches.next().is_none().then_some(found)
    }

    fn match_edges(&self, ring: &Ring) -> Vec<Option<Span>> {
        let n = ring.len();
        let mut edges: Vec<_> = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(&a, &b)| {
                self.clear_edges
                    .get(&(exact_grid_point(a)?, exact_grid_point(b)?))
                    .copied()
            })
            .collect();
        for (i, &junction) in ring.iter().enumerate() {
            let previous = (i + n - 1) % n;
            if edges[previous].is_some() || edges[i].is_some() {
                continue;
            }
            let a = ring[previous];
            let b = ring[(i + 1) % n];
            if !transverse(a, junction, b) {
                continue;
            }
            let (left, right) = (
                self.clipped_span(a, junction),
                self.clipped_span(b, junction),
            );
            // For a curve meeting a line, the outgoing straight section must
            // lie on the one contacting source line and the incoming curve
            // must travel monotonically across it. This also handles a cut
            // exactly at an authored anchor without a residual straight chord.
            let on_line = |clipped: &ClippedSpan, adjacent| {
                let neighbor = clipped.neighbor;
                let [start, end] = [neighbor.0, neighbor.1].map(|p| p.map(|v| v as f64 / GRID));
                self.straight_edges.contains(&neighbor)
                    && self.curves[clipped.span.curve].monotone_across_line(clipped.span, neighbor)
                    && [adjacent, junction]
                        .into_iter()
                        .all(|p| flatten::distance_sq(p, start, end) < 1e-15)
            };
            match (left, right) {
                (Some(left), Some(right))
                    if left.source == right.neighbor && right.source == left.neighbor =>
                {
                    edges[previous] = Some(left.span);
                    edges[i] = Some(right.span.reversed());
                }
                (Some(left), None) if on_line(&left, b) => edges[previous] = Some(left.span),
                (None, Some(right)) if on_line(&right, a) => edges[i] = Some(right.span.reversed()),
                _ => (),
            }
        }
        edges
    }

    pub(super) fn nodes_for(&self, ring: &Ring) -> Option<Vec<PathNode>> {
        if !self.changed_ring(ring) {
            return None;
        }
        self.nodes_from_edges(ring, &self.match_edges(ring), true)
    }

    pub(super) fn for_cut(path: &Path, input: &[Contour]) -> Option<Self> {
        if !path
            .verbs()
            .iter()
            .any(|v| matches!(v, PathVerb::Cubic | PathVerb::Quad))
        {
            return None;
        }
        // A retraced chord could belong to a straight edge or a different cubic.
        let mut seen = HashSet::new();
        let mut repeated = Contacts::new();
        for contour in input {
            for (a, b) in super::cut::edges(contour) {
                let edge = unordered((a, b));
                if !seen.insert(edge) {
                    repeated.insert(edge, None);
                }
            }
        }
        let mut spans = Self::default();
        spans.trace_path(path, &repeated)?;
        Some(spans)
    }

    pub(super) fn cut_span(&self, edge: Edge, from: World, to: World) -> Option<Span> {
        let span = *self.clear_edges.get(&edge)?;
        let parameter = |p, endpoint, t| {
            if exact_grid_point(p) == Some(endpoint) {
                Some(t)
            } else {
                Some(self.curves[span.curve].clipped(span, edge, p)?.end)
            }
        };
        Some(Span {
            start: parameter(from, edge.0, span.start)?,
            end: parameter(to, edge.1, span.end)?,
            ..span
        })
    }

    pub(super) fn nodes_from_edges(
        &self,
        ring: &Ring,
        edges: &[Option<Span>],
        closed: bool,
    ) -> Option<Vec<PathNode>> {
        if edges.iter().all(Option::is_none) {
            return None;
        }
        let n = ring.len();
        let count = edges.len();
        debug_assert_eq!(count, n - usize::from(!closed));
        let joins = |i: usize, j: usize| {
            edges[i]
                .zip(edges[j])
                .is_some_and(|(left, right)| right.follows(left))
        };
        // The solver may begin anywhere on the ring, including mid-Bézier.
        let start = if closed {
            (0..n).find(|&i| !joins((i + n - 1) % n, i))?
        } else {
            0
        };
        let mut nodes = Vec::new();
        let mut next_incoming = None;
        let mut offset = 0;
        while offset < count {
            let from = (start + offset) % n;
            let mut end = offset + 1;
            while end < count && joins((start + end - 1) % n, (start + end) % n) {
                end += 1
            }
            let p = ring[from];
            let mut node = PathNode::corner(p.map(|v| v as f32));
            if let Some(handle) = next_incoming.take() {
                node.incoming = handle
            }
            if end > offset + 1 || !closed && edges[from].is_some() {
                let first = edges[from]?;
                let last = edges[(start + end - 1) % n]?;
                let curve = &self.curves[first.curve];
                let dt = last.end - first.start;
                let out = curve.handle_offset(first.start, dt);
                let incoming = curve.handle_offset(last.end, dt);
                let q = ring[(start + end) % n];
                node.outgoing = std::array::from_fn(|i| (p[i] + out[i]) as f32);
                next_incoming = Some(std::array::from_fn(|i| (q[i] - incoming[i]) as f32));
            }
            nodes.push(node);
            offset = end;
        }
        if closed {
            if let Some(handle) = next_incoming {
                nodes[0].incoming = handle
            }
        } else {
            let mut last = PathNode::corner(ring[n - 1].map(|v| v as f32));
            if let Some(handle) = next_incoming {
                last.incoming = handle;
            }
            nodes.push(last);
        }
        (!closed || nodes.len() >= 3 && nodes.len() < n).then_some(nodes)
    }
}
