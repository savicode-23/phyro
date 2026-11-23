use super::*;
use crate::{geometry::path_contains, Document, PathNode, Shape};
use tiny_skia_path::PathVerb;

fn stroke(points: &[[f32; 2]], closed: bool) -> Document {
    let mut doc = Document::default();
    let nodes = points.iter().copied().map(PathNode::corner).collect();
    doc.add_shape([0.; 2], [1.; 2], [0; 4], Shape::Path { nodes, closed }, 1);
    doc.objects[0].stroke_width = 20.;
    doc
}

#[test]
fn pressure_peak_between_two_anchors_controls_picking_bounds_and_keys() {
    let mut doc = stroke(&[[10., 32.], [110., 32.]], false);
    let o = &mut doc.objects[0];
    o.stroke_profile = vec![[0., 0.], [0.5, 1.], [1., 0.]];
    o.animated.width_keys.insert(0, 20.);
    o.animated.width_keys.insert(4, 10.);
    assert!(o.contains([60., 41.], 0));
    assert!(!o.contains([15., 40.], 0));
    assert!(!o.contains([60., 41.], 4));
    let b = o.bounds(0);
    for (actual, expected) in b.into_iter().zip([10., 22., 110., 42.]) {
        assert!((actual - expected).abs() < 0.005);
    }
    assert!(o.bounds_within([9., 21., 111., 43.], 0));
    o.stroke_profile = vec![[0., 0.], [0.5, 0.], [1., 1.]];
    assert!(o.bounds_within([59., 21., 121., 43.], 0));
}

#[test]
fn compact_joins_match_a_swept_disk_oracle_on_bends_cusps_and_closed_seams() {
    type BrushCase<'a> = (&'a [[f32; 2]], &'a [f32], bool);
    let cases: &[BrushCase<'_>] = &[
        (
            &[[10., 20.], [37., 18.], [58., 28.], [72., 50.]],
            &[0.1, 0.8, 1., 0.35],
            false,
        ),
        (
            &[[8., 20.], [50., 20.], [28., 22.], [48., 21.], [52., 45.]],
            &[0., 0.8, 0.7, 0., 1.],
            false,
        ),
        (
            &[[12., 12.], [62., 12.], [62., 62.], [12., 62.]],
            &[0.45, 0.9, 0.6, 1., 0.45],
            true,
        ),
        (&[[15., 21.], [17., 21.], [43., 26.]], &[0., 1., 0.6], false),
    ];
    for &(points, factors, closed) in cases {
        let mut doc = stroke(points, closed);
        let o = &mut doc.objects[0];
        let mut points: Vec<Coord> = points.iter().map(|p| p.map(f64::from)).collect();
        if closed {
            points.push(points[0]);
        }
        let mut d = vec![0.];
        for p in points.windows(2) {
            d.push(d.last().unwrap() + (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]));
        }
        o.stroke_profile = d
            .iter()
            .zip(factors)
            .map(|(&s, &r)| [(s / d.last().unwrap()) as f32, r])
            .collect();
        let region = o.pressure_outline(&o.outline(0).unwrap(), 0).unwrap();
        if points.len() == 4 && !closed {
            assert_eq!(
                region
                    .verbs()
                    .iter()
                    .filter(|&&v| v == PathVerb::Move)
                    .count(),
                1
            );
        }
        for y in 0..74 {
            for x in 0..84 {
                let p = [f64::from(x) + 0.3, f64::from(y) + 0.7];
                let mut margin = f64::NEG_INFINITY;
                for (i, pair) in points.windows(2).enumerate() {
                    // Independent dense stamps along the segment; exclude the
                    // thin band in which sampling or hit-test flattening matters.
                    for j in 0..=50 {
                        let t = f64::from(j) / 50.;
                        let q = mix(pair[0], pair[1], t);
                        let r = 10.
                            * (f64::from(factors[i]) * (1. - t) + f64::from(factors[i + 1]) * t);
                        margin = margin.max(r - (q[0] - p[0]).hypot(q[1] - p[1]));
                    }
                }
                if margin.abs() > 0.25 {
                    assert_eq!(
                        path_contains(&region, p.map(|v| v as f32)),
                        margin > 0.,
                        "points={points:?} query={p:?} margin={margin}"
                    );
                }
            }
        }
    }
}

#[test]
fn pressure_profiles_reject_invalid_positions_factors_and_excessive_sample_counts() {
    let mut doc = stroke(&[[0.; 2], [10., 0.]], false);
    for invalid in [
        vec![[0., 1.]],
        vec![[0.1, 1.], [1., 1.]],
        vec![[0., 1.], [1., 1.1]],
        vec![[0., 1.], [0., 0.], [1., 1.]],
        vec![[0., 1.], [f32::NAN, 0.], [1., 1.]],
        (0..=MAX_STROKE_SAMPLES)
            .map(|i| [i as f32 / MAX_STROKE_SAMPLES as f32, 0.4])
            .collect(),
    ] {
        doc.objects[0].stroke_profile = invalid;
        assert!(doc.validate().unwrap_err().contains("pressure profile"));
    }
    doc.objects[0].stroke_profile = vec![[0., 0.], [0.4, 1.], [1., 0.]];
    assert!(doc.validate().is_ok());
}

#[test]
fn reversing_visible_centerlines_preserves_the_pressure_on_each_side_of_the_artwork() {
    for closed in [false, true] {
        let mut doc = stroke(&[[8., 8.], [46., 8.], [46., 55.], [10., 40.]], closed);
        let o = &mut doc.objects[0];
        o.stroke_profile = vec![[0., 0.3], [0.17, 0.9], [0.54, 0.1], [0.77, 1.], [1., 0.3]];
        let before = o.pressure_outline(&o.outline(0).unwrap(), 0).unwrap();
        o.reorder_pressure_profile(0, 0, 3, true);
        crate::path_edit::reverse(&mut o.shape.contours_mut()[0]);
        let after = o.pressure_outline(&o.outline(0).unwrap(), 0).unwrap();
        assert!(valid_profile(&o.stroke_profile));
        for y in 0..70 {
            for x in 0..62 {
                let at = [x as f32 + 0.34, y as f32 + 0.42];
                assert_eq!(
                    path_contains(&before, at),
                    path_contains(&after, at),
                    "{closed} {at:?}"
                );
            }
        }
    }
}
