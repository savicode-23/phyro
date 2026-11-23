use super::{BooleanOp, Piece, COMPLEXITY};
use i_overlay::core::overlay::Overlay;
use i_overlay::core::{fill_rule::FillRule, overlay_rule::OverlayRule, single::SingleIntOverlay};
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::int::shape::{IntShape, IntShapes};
use i_overlay::string::slice::IntSlice;

// Snap inputs consistently. Leave 16 fractional bits below the snap grid for
// intersection vertices and intermediate results when folding multiple objects.
pub(super) type Point = [i64; 2];
pub(super) const GRID: f64 = 1024.;
pub(super) const LIMIT: i64 = 1 << 27;
const SUBGRID: i64 = 1 << 16;

pub(super) struct Contour {
    pub points: Vec<Point>,
    pub closed: bool,
}

fn integer_paths(paths: &[Contour]) -> IntShape<i64> {
    paths
        .iter()
        .map(|c| {
            c.points
                .iter()
                .map(|p| IntPoint::new(p[0] * SUBGRID, p[1] * SUBGRID))
                .collect()
        })
        .collect()
}

/// Normalize a parity-filled operand independently: the overlay backend accepts
/// one common fill rule per operation, while each source object has its own rule.
fn region(paths: &[Contour], rule: crate::FillRule) -> IntShape<i64> {
    let raw = integer_paths(paths);
    if rule == crate::FillRule::EvenOdd {
        Overlay::from_subj(&raw)
            .overlay(OverlayRule::Subject, FillRule::EvenOdd)
            .into_iter()
            .flatten()
            .collect()
    } else {
        raw
    }
}

fn check_size(shapes: &IntShapes<i64>) -> Result<(), String> {
    if shapes.iter().flatten().map(Vec::len).sum::<usize>() > crate::MAX_PATH_NODES {
        Err(COMPLEXITY.into())
    } else {
        Ok(())
    }
}

// A few thousand long crisscrossing segments can otherwise yield millions of
// intersections before the output node limit can be checked. Count overlapping
// segment boxes in an X sweep to bound work and memory for a synchronous edit.
pub(super) fn check_arrangement_budget(inputs: &[Vec<Contour>]) -> Result<(), String> {
    let mut boxes = Vec::new();
    for c in inputs.iter().flatten() {
        let edge_count = c.points.len() - usize::from(!c.closed);
        for (a, b) in c
            .points
            .iter()
            .zip(c.points.iter().cycle().skip(1))
            .take(edge_count)
        {
            boxes.push([
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ]);
        }
    }
    boxes.sort_unstable_by_key(|b| b[0]);
    let mut overlaps = 0;
    for (i, b) in boxes.iter().enumerate() {
        for c in &boxes[i + 1..] {
            if c[0] > b[2] {
                break;
            }
            if c[1] <= b[3] && b[1] <= c[3] {
                overlaps += 1;
                if overlaps > 65_536 {
                    return Err(COMPLEXITY.into());
                }
            }
        }
    }
    Ok(())
}
pub(super) fn combine(
    inputs: &[Vec<Contour>],
    rules: &[crate::FillRule],
    op: BooleanOp,
) -> Result<Vec<Piece>, String> {
    check_arrangement_budget(inputs)?;
    let source = region(&inputs[0], rules[0]);
    let shapes = if op == BooleanOp::Division {
        let parts = if inputs[1].iter().all(|c| c.closed) {
            let cutter = region(&inputs[1], rules[1]);
            let mut outside = source.overlay(&cutter, OverlayRule::Difference, FillRule::NonZero);
            outside.extend(source.overlay(&cutter, OverlayRule::Intersect, FillRule::NonZero));
            outside
        } else {
            source.slice_by_paths(&integer_paths(&inputs[1]), FillRule::NonZero)
        };
        let old_count = source
            .overlay(&source, OverlayRule::Subject, FillRule::NonZero)
            .len();
        if parts.len() <= old_count {
            return Err("The cutter does not divide the back fill into separate regions.".into());
        }
        parts
    } else {
        let rule = match op {
            BooleanOp::Union => OverlayRule::Union,
            BooleanOp::Exclusion => OverlayRule::Xor,
            BooleanOp::Intersection => OverlayRule::Intersect,
            BooleanOp::Difference => OverlayRule::Difference,
            _ => unreachable!(),
        };
        let mut current = vec![source];
        for (operand, &fill) in inputs[1..].iter().zip(&rules[1..]) {
            current = current.overlay(&region(operand, fill), rule, FillRule::NonZero);
            check_size(&current)?;
        }
        current
    };
    check_size(&shapes)?;
    let to_ring = |ring: Vec<IntPoint<i64>>| {
        ring.into_iter()
            .map(|p| [p.x, p.y].map(|v| v as f64 / (GRID * SUBGRID as f64)))
            .collect()
    };
    if op == BooleanOp::Division {
        Ok(shapes
            .into_iter()
            .map(|shape| (shape.into_iter().map(to_ring).collect(), true))
            .collect())
    } else if shapes.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![(
            shapes.into_iter().flatten().map(to_ring).collect(),
            true,
        )])
    }
}
