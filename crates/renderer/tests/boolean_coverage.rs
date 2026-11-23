//! Differential coverage against tiny-skia's independent nonzero scan converter.
//! Rasterize each original operand separately, compose the binary masks, and
//! compare to rasterization of the editable result. No boolean predicates are
//! reused by the oracle.
use document_core::{BooleanOp, Document, PathNode, Shape};
use tiny_skia::{FillRule, Mask, Transform};

const SCALE: u32 = 4;
const WIDTH: u32 = 48 * SCALE;
type Rings = Vec<Vec<[f32; 2]>>;

fn add(doc: &mut Document, rings: Rings) -> u64 {
    let contours = rings
        .into_iter()
        .map(|r| r.into_iter().map(PathNode::corner).collect())
        .collect();
    doc.add_shape([0.; 2], [1.; 2], [255; 4], Shape::Compound { contours }, 1)
}

fn mask(doc: &Document, ids: &[u64]) -> Vec<u8> {
    let mut mask = Mask::new(WIDTH, WIDTH).unwrap();
    for obj in doc.objects.iter().filter(|o| ids.contains(&o.id)) {
        mask.fill_path(
            &obj.outline(0).unwrap(),
            match obj.fill_rule {
                document_core::FillRule::NonZero => FillRule::Winding,
                document_core::FillRule::EvenOdd => FillRule::EvenOdd,
            },
            false,
            Transform::from_scale(SCALE as f32, SCALE as f32),
        );
    }
    mask.data().to_vec()
}

fn polygon(seed: &mut u32) -> Vec<[f32; 2]> {
    let mut coord = || {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        1. + (((*seed >> 8) % 704) as f32) / 16.
    };
    (0..8).map(|_| [coord(), coord()]).collect()
}

fn square(x: f32, y: f32, w: f32, h: f32) -> Vec<[f32; 2]> {
    vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

fn near_source_edge(doc: &Document, index: usize) -> bool {
    let stride = WIDTH as usize;
    let p = [(index % stride) as f64 + 0.5, (index / stride) as f64 + 0.5]
        .map(|v| v / f64::from(SCALE));
    doc.objects
        .iter()
        .flat_map(|o| o.shape.contours())
        .any(|ring| {
            ring.iter().zip(ring.iter().cycle().skip(1)).any(|(a, b)| {
                let a = a.point.map(f64::from);
                let b = b.point.map(f64::from);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let t =
                    (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
                (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy) <= 0.5 / f64::from(SCALE)
            })
        })
}
#[test]
fn mask_algebra_matches_independent_rasterization() {
    let mut seed = 1_539_731;
    let mut cases = vec![];
    for _ in 0..48 {
        cases.push((vec![polygon(&mut seed)], vec![polygon(&mut seed)]));
    }
    let mut hole = square(10., 10., 24., 24.);
    hole.reverse();
    let nested = vec![square(4., 4., 36., 36.), hole, square(17., 17., 10., 10.)];
    cases.extend([
        (nested.clone(), vec![square(15., 2., 10., 42.)]), // Split a ring and an island.
        (nested.clone(), nested),                          // Identical compound operands.
        (
            vec![square(4., 4., 18., 18.)],
            vec![square(22., 22., 18., 18.)],
        ), // Point touch.
        (
            vec![square(4., 4., 18., 18.)],
            vec![square(22., 10., 18., 10.)],
        ), // Shared edge.
        (
            vec![vec![
                [2., 6.],
                [20., 6.001],
                [44., 6.],
                [40., 39.],
                [3., 35.],
            ]],
            vec![vec![[5., 6.002], [42., 5.999], [16., 33.]]],
        ), // Near-collinear intersections.
    ]);
    for (case, (left, right)) in cases.into_iter().enumerate() {
        let mut doc = Document::default();
        let a = add(&mut doc, left);
        let b = add(&mut doc, right);
        if case % 3 == 0 {
            doc.objects[0].fill_rule = document_core::FillRule::EvenOdd;
        }
        if case % 4 == 0 {
            doc.objects[1].fill_rule = document_core::FillRule::EvenOdd;
        }
        let (ma, mb) = (mask(&doc, &[a]), mask(&doc, &[b]));
        for op in [
            BooleanOp::Union,
            BooleanOp::Difference,
            BooleanOp::Intersection,
            BooleanOp::Exclusion,
        ] {
            let mut result = doc.clone();
            let ids = result
                .boolean(&[a, b], 0, op)
                .unwrap_or_else(|e| panic!("case {case}, {op:?}: {e}"));
            let actual = mask(&result, &ids);
            let mut mismatches = 0;
            for (i, (&a, &b)) in ma.iter().zip(&mb).enumerate() {
                let expected = match op {
                    BooleanOp::Union => a | b,
                    BooleanOp::Difference => a & !b,
                    BooleanOp::Intersection => a & b,
                    BooleanOp::Exclusion => a ^ b,
                    _ => unreachable!(),
                };
                if actual[i] != expected {
                    // Scan conversion rounds to subpixel coordinates; edges with new
                    // intersection endpoints may round differently at sample centers.
                    assert!(
                        near_source_edge(&doc, i),
                        "interior mismatch, case {case} {op:?}, pixel {i}"
                    );
                    mismatches += 1;
                }
            }
            assert!(
                mismatches <= WIDTH as usize * WIDTH as usize / 500,
                "{mismatches} subpixel mismatches, case {case} {op:?}"
            );
        }
    }
}
