use crate::{BooleanOp, Document, FillRule, PathNode, Shape};

// Independently compare the rendered winding against the two source fills.
// Sample off the tolerance band around an input or newly clipped junction.
fn assert_fill_matches(source: &Document, result: &Document, op: BooleanOp) {
    let paths = |d: &Document| {
        d.objects
            .iter()
            .map(|o| (o.outline(0).unwrap(), o.fill_rule))
            .collect::<Vec<_>>()
    };
    let old = paths(source);
    let new = paths(result);
    let hit = |(path, rule): &(tiny_skia_path::Path, FillRule), p| {
        crate::geometry::path_contains_with_rule(path, p, *rule)
    };
    let source_at = |p| [hit(&old[0], p), hit(&old[1], p)];
    for y in 0..44 {
        for x in 0..70 {
            let p = [x as f32 * 3. + 0.37, y as f32 * 3. + 0.51];
            let [a, b] = source_at(p);
            if [[0.06, 0.], [-0.06, 0.], [0., 0.06], [0., -0.06]]
                .iter()
                .any(|q| source_at([p[0] + q[0], p[1] + q[1]]) != [a, b])
            {
                continue;
            }
            let expected = match op {
                BooleanOp::Union => a || b,
                BooleanOp::Difference => a && !b,
                BooleanOp::Intersection => a && b,
                BooleanOp::Exclusion => a ^ b,
                BooleanOp::Division => a,
                _ => unreachable!(),
            };
            assert_eq!(
                new.iter().filter(|outline| hit(outline, p)).count(),
                usize::from(expected),
                "{op:?} at {p:?}"
            );
        }
    }
}

#[test]
fn a_crossing_at_an_existing_anchor_and_an_implicitly_closed_curve_stay_editable() {
    let oval = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap();
    for open in [false, true] {
        let mut scene = Document::default();
        let mut shape = oval.clone();
        if let Shape::Path { closed, .. } = &mut shape {
            *closed = !open;
        }
        let a = scene.add_shape([8., 12.], [80., 68.], [255; 4], shape, 1);
        let b = scene.add_rectangle([48., 0.], [52., 106.], [255; 4]);
        for op in [
            BooleanOp::Union,
            BooleanOp::Difference,
            BooleanOp::Intersection,
            BooleanOp::Exclusion,
            BooleanOp::Division,
        ] {
            let mut d = scene.clone();
            d.boolean(&[a, b], 0, op).unwrap();
            d.validate().unwrap();
            for ring in d.objects.iter().flat_map(|o| o.shape.contours()) {
                assert!(ring.len() <= 7, "open {open} / {op:?}: {}", ring.len());
            }
            if matches!(op, BooleanOp::Difference | BooleanOp::Intersection) {
                let nodes = &d.objects[0].shape.contours()[0];
                assert_eq!(nodes.len(), 3);
                assert!(nodes
                    .iter()
                    .any(|n| n.incoming != n.point || n.outgoing != n.point));
            }
            assert_fill_matches(&scene, &d, op);
        }
    }
}

#[test]
fn dividing_an_oval_with_an_open_line_leaves_two_exact_halves() {
    let mut scene = Document::default();
    let oval = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap();
    let a = scene.add_shape([8., 12.], [80., 68.], [255; 4], oval, 1);
    let b = scene
        .add_artboard_path(
            [[48., 0.], [48., 106.]].map(PathNode::corner).to_vec(),
            false,
            1,
            [255; 4],
        )
        .unwrap();
    let mut d = scene.clone();
    assert_eq!(d.boolean(&[a, b], 0, BooleanOp::Division).unwrap().len(), 2);
    for object in &d.objects {
        assert_eq!(object.shape.contours()[0].len(), 3);
    }
    assert_fill_matches(&scene, &d, BooleanOp::Division);
}

#[test]
fn crossing_ovals_keep_both_curved_boundaries_when_excluding_and_dividing() {
    let oval = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap();
    let mut scene = Document::default();
    let a = scene.add_shape([32.; 2], [96.; 2], [255; 4], oval.clone(), 1);
    let b = scene.add_shape([65., 48.], [96.; 2], [255; 4], oval, 1);
    for op in [BooleanOp::Exclusion, BooleanOp::Division] {
        let mut d = scene.clone();
        d.boolean(&[a, b], 0, op).unwrap();
        let contours: Vec<_> = d.objects.iter().flat_map(|o| o.shape.contours()).collect();
        assert_eq!(contours.len(), 2);
        for ring in contours {
            assert_eq!(ring.len(), 6);
            assert!(ring
                .iter()
                .all(|n| n.point != n.incoming && n.point != n.outgoing));
        }
        assert_fill_matches(&scene, &d, op);
    }
}

#[test]
fn rotated_copies_of_a_concave_pen_path_keep_their_long_curved_spans() {
    // The top cubic sweeps left before turning right. Its first control
    // edge runs opposite to the chord, but the path never crosses itself.
    let mut nodes = [
        [15., 20.],
        [91., 20.],
        [91., 76.],
        [65., 68.],
        [65., 39.],
        [16., 54.],
    ]
    .map(PathNode::corner);
    nodes[0].outgoing = [-14., 8.];
    nodes[1].incoming = [54., -9.];
    nodes[1].outgoing = [128., -2.];
    nodes[2].incoming = [111., 42.];
    nodes[2].outgoing = [99., 95.];
    nodes[3].incoming = [79., 89.];
    nodes[4].outgoing = [46., 27.];
    nodes[5].incoming = [22., 63.];
    for (angle, offset) in [(0., [23., 8.]), (27., [11., 19.])] {
        let mut scene = Document::default();
        let [a, b] = [0, 1].map(|_| {
            scene
                .add_artboard_path(nodes.to_vec(), true, 1, [255; 4])
                .unwrap()
        });
        scene.objects[0].rotation = angle;
        scene.objects[1].rotation = angle - 74.;
        for (value, delta) in scene.objects[1].position.iter_mut().zip(offset) {
            *value += delta;
        }
        let mut d = scene.clone();
        d.boolean(&[a, b], 0, BooleanOp::Union).unwrap();
        d.validate().unwrap();
        let rings = d.objects[0].shape.contours();
        let knots: usize = rings.iter().map(Vec::len).sum();
        assert!(knots <= 24, "{angle}°: {knots} knots");
        assert!(
            rings
                .iter()
                .flatten()
                .filter(|n| n.incoming != n.point || n.outgoing != n.point)
                .count()
                >= 5
        );
        assert_fill_matches(&scene, &d, BooleanOp::Union);
    }
}

#[test]
fn concave_outlines_keep_their_corners_and_curves_through_consecutive_operations() {
    let mut nodes = [
        [0., 0.12],
        [0.8, 0.],
        [1., 0.7],
        [0.65, 0.7],
        [0.65, 0.25],
        [0.35, 0.25],
        [0.35, 0.8],
        [0., 0.9],
    ]
    .map(PathNode::corner);
    nodes[0].outgoing = [0.12, 0.];
    nodes[1].incoming = [0.7, 0.];
    nodes[1].outgoing = [1., 0.];
    nodes[2].incoming = [1., 0.4];
    nodes[6].outgoing = [0.35, 1.];
    nodes[7].incoming = [0.12, 1.];
    nodes[7].outgoing = [0., 0.8];
    let oval = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap();
    for disjoint in [false, true] {
        let mut scene = Document::default();
        let a = scene.add_shape(
            [12., 21.],
            [97., 89.],
            [255; 4],
            Shape::Path {
                closed: true,
                nodes: nodes.to_vec(),
            },
            1,
        );
        let b = scene.add_shape(
            if disjoint { [128., 21.] } else { [56., 38.] },
            [70., 70.],
            [255; 4],
            oval.clone(),
            1,
        );
        let mut d = scene.clone();
        d.boolean(&[a, b], 0, BooleanOp::Union).unwrap();
        let rings = d.objects[0].shape.contours();
        assert!(rings
            .iter()
            .all(|r| r.iter().any(|n| n.incoming != n.point)));
        if disjoint {
            let mut counts: Vec<_> = rings.iter().map(Vec::len).collect();
            counts.sort_unstable();
            assert_eq!(counts, [4, 8]);
        } else {
            assert_eq!(rings.len(), 1);
            assert!(rings[0].len() <= 16, "{} nodes", rings[0].len());
        }
        let corner = scene.objects[0].local_to_world(nodes[5].point, 0);
        assert!(rings.iter().flatten().any(|n| {
            let p = d.objects[0].local_to_world(n.point, 0);
            (p[0] - corner[0]).hypot(p[1] - corner[1]) < 0.001
                && n.incoming == n.point
                && n.outgoing == n.point
        }));
        assert_fill_matches(&scene, &d, BooleanOp::Union);
        let c = d.add_rectangle([30., 0.], [8., 126.], [255; 4]);
        let before = d.clone();
        d.boolean(&[a, c], 0, BooleanOp::Difference).unwrap();
        d.validate().unwrap();
        assert!(d.objects[0].shape.contours().iter().all(|r| r.len() <= 20));
        assert_fill_matches(&before, &d, BooleanOp::Difference);
    }
}
