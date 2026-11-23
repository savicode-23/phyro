//! Reattach whole, uncut source contours only when the integer overlay returns
//! exactly their boundary and the source curves are simple and safely separated.
use super::{
    flatten::TOLERANCE,
    planar::{Contour, Point, GRID},
    Piece, Ring,
};
use crate::{geometry::editable_contours, PathNode};
use tiny_skia_path::{Path, PathVerb};
mod separated;

pub(super) const MAX_CONTOURS: usize = 32;
const MAX_ANCHORS: usize = 128;

pub(super) struct Original<'a> {
    grid: &'a [Point],
    nodes: Vec<PathNode>,
}

// Certify simplicity by sweeping once around the mean of the anchors. Each
// segment's control points must occur in angular order in a sector < 180°.
// A ray inside that sector crosses the control polygon at most once, hence
// crosses the Bézier at most once by its variation-diminishing property.
fn radial(nodes: &[PathNode]) -> bool {
    let center: [f64; 2] = std::array::from_fn(|i| {
        nodes.iter().map(|n| f64::from(n.point[i])).sum::<f64>() / nodes.len() as f64
    });
    let relative = |p: [f32; 2]| [f64::from(p[0]) - center[0], f64::from(p[1]) - center[1]];
    let edges = nodes.iter().zip(nodes.iter().cycle().skip(1));
    let area: f64 = edges
        .clone()
        .map(|(a, b)| {
            let [ax, ay] = relative(a.point);
            let [bx, by] = relative(b.point);
            ax * by - ay * bx
        })
        .sum();
    if area == 0. {
        return false;
    }
    let angle = |p| {
        let [x, y] = relative(p);
        (x != 0. || y != 0.).then(|| area.signum() * y.atan2(x))
    };
    let mut wraps = 0;
    for (a, b) in edges {
        let [Some(start), Some(first), Some(second), Some(end)] =
            [a.point, a.outgoing, b.incoming, b.point].map(angle)
        else {
            return false;
        };
        let from_start = |v: f64| (v - start).rem_euclid(std::f64::consts::TAU);
        let (span, t, u) = (from_start(end), from_start(first), from_start(second));
        if span == 0. || span >= std::f64::consts::PI || t > u || u > span {
            return false;
        }
        wraps += usize::from(end < start);
    }
    wraps == 1
}

pub(super) fn simple(nodes: &[PathNode]) -> bool {
    (3..=MAX_ANCHORS).contains(&nodes.len()) && (radial(nodes) || separated::simple(nodes))
}

// Bounds in integer flattening coordinates. Source curves lie in 0.02 px
// capsules around the unsnapped chords. Include both capsules, endpoint grid
// rounding and enough room to store reconstructed handles as f32s.
fn bounds(points: &[Point]) -> [i64; 4] {
    let [x, y] = points[0];
    points.iter().fold([x, y, x, y], |[l, t, r, b], &[x, y]| {
        [l.min(x), t.min(y), r.max(x), b.max(y)]
    })
}

fn misses(contour: &Contour, [l, t, r, b]: [i64; 4], margin: i64) -> bool {
    contour
        .points
        .iter()
        .zip(contour.points.iter().cycle().skip(1))
        .all(|(a, z)| {
            a[0].max(z[0]) + margin < l
                || a[0].min(z[0]) - margin > r
                || a[1].max(z[1]) + margin < t
                || a[1].min(z[1]) - margin > b
        })
}

pub(super) fn filled_nodes(path: &Path, input: &[Contour]) -> Option<Vec<Vec<PathNode>>> {
    if input.iter().any(|c| !c.closed) || path.verbs().len() > input.len() * (MAX_ANCHORS + 2) {
        return None;
    }
    let mut contours = editable_contours(path, |p| [p.x, p.y])?;
    for nodes in &mut contours {
        // Filled open paths have an implicit straight closing edge. Their
        // first incoming and last outgoing handles do not describe a segment.
        if nodes.len() > 1 && nodes.first()?.point == nodes.last()?.point {
            nodes[0].incoming = nodes.pop()?.incoming;
        }
    }
    (contours.len() == input.len()).then_some(contours)
}

pub(super) fn originals<'a>(
    outlines: &[Path],
    inputs: &'a [Vec<Contour>],
    pieces: &[Piece],
) -> Vec<Original<'a>> {
    // Crossing edits normally change every ring's vertex count. Skip the
    // separation and source-geometry checks as soon as a match is impossible.
    if outlines.len() != inputs.len()
        || inputs.iter().map(Vec::len).sum::<usize>() > MAX_CONTOURS
        || !pieces
            .iter()
            .flat_map(|(rings, _)| rings)
            .any(|r| inputs.iter().flatten().any(|c| c.points.len() == r.len()))
        || !outlines.iter().any(|p| {
            p.verbs()
                .iter()
                .any(|v| matches!(v, PathVerb::Quad | PathVerb::Cubic))
        })
    {
        return Vec::new();
    }
    struct Boundary<'a> {
        source: usize,
        index: usize,
        contour: &'a Contour,
        bounds: [i64; 4],
    }
    let boundaries: Vec<_> = inputs
        .iter()
        .enumerate()
        .flat_map(|(source, contours)| {
            contours
                .iter()
                .enumerate()
                .map(move |(index, contour)| Boundary {
                    source,
                    index,
                    contour,
                    bounds: bounds(&contour.points),
                })
        })
        .collect();
    let max_coord = boundaries
        .iter()
        .flat_map(|b| b.bounds)
        .map(|v| v.abs())
        .max()
        .unwrap_or(0);
    let margin = (2. * TOLERANCE * GRID + 2. + 32. * f64::from(f32::EPSILON) * max_coord as f64)
        .ceil() as i64;
    let mut isolated = vec![true; boundaries.len()];
    for (i, a) in boundaries.iter().enumerate() {
        for (j, b) in boundaries.iter().enumerate().skip(i + 1) {
            // Exact copies have the same continuous boundary. Comparing the
            // evaluated paths also checks their controls, beyond the snap grid.
            if a.index == b.index && outlines[a.source] == outlines[b.source] {
                continue;
            }
            if !misses(a.contour, b.bounds, margin) && !misses(b.contour, a.bounds, margin) {
                isolated[i] = false;
                isolated[j] = false;
            }
        }
    }
    let mut originals = Vec::new();
    let mut remaining = isolated.as_slice();
    for (path, input) in outlines.iter().zip(inputs) {
        let (isolated, rest) = remaining.split_at(input.len());
        remaining = rest;
        let Some(contours) = filled_nodes(path, input) else {
            continue;
        };
        for ((nodes, c), &isolated) in contours.into_iter().zip(input).zip(isolated) {
            if !isolated {
                continue;
            }
            if nodes
                .iter()
                .any(|n| n.incoming != n.point || n.outgoing != n.point)
                && simple(&nodes)
            {
                originals.push(Original {
                    grid: &c.points,
                    nodes,
                });
            }
        }
    }
    originals
}

pub(super) fn orientation(grid: &[Point], ring: &Ring) -> Option<bool> {
    let n = grid.len();
    if n != ring.len() {
        return None;
    }
    let equal =
        |grid: Point, p: [f64; 2]| p[0] * GRID == grid[0] as f64 && p[1] * GRID == grid[1] as f64;
    let start = grid.iter().position(|&g| equal(g, ring[0]))?;
    let forward = (0..n).all(|i| equal(grid[(start + i) % n], ring[i]));
    if !forward && !(0..n).all(|i| equal(grid[(start + n - i) % n], ring[i])) {
        return None;
    }
    Some(forward)
}

impl Original<'_> {
    pub(super) fn nodes_for(&self, ring: &Ring) -> Option<Vec<PathNode>> {
        let forward = orientation(self.grid, ring)?;
        let mut nodes = self.nodes.clone();
        if !forward {
            crate::path_edit::reverse(&mut nodes);
        }
        Some(nodes)
    }
}

#[cfg(test)]
mod tests;
