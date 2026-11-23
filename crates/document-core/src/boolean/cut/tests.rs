use crate::{BooleanOp, Document, PathNode, Shape};

fn vertical_cutter(doc: &mut Document, x: f32) -> u64 {
    doc.add_shape(
        [x, -10.],
        [1., 120.],
        [255; 4],
        Shape::Path {
            nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([0., 1.])],
            closed: false,
        },
        1,
    )
}

#[test]
fn cutting_at_authored_anchors_retains_the_original_handles_on_both_halves() {
    let mut doc = Document::default();
    let oval = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap();
    let a = doc.add_shape([0.; 2], [96.; 2], [22, 33, 44, 255], oval, 1);
    let original = doc.objects[0].clone();
    let b = vertical_cutter(&mut doc, 48.);
    let pieces = doc.boolean(&[a, b], 0, BooleanOp::CutPath).unwrap();
    assert_eq!(pieces.len(), 2);
    for piece in &doc.objects {
        let Shape::Path {
            nodes,
            closed: false,
        } = &piece.shape
        else {
            panic!("expected an open half-circle")
        };
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].incoming, nodes[0].point);
        assert_eq!(nodes[2].outgoing, nodes[2].point);
        assert_eq!(piece.stroke, original.fill);
        assert!(piece.no_fill);
        assert_eq!(piece.fill, original.fill);
        let world = piece.local_to_world_at(0);
        let original_world = original.local_to_world_at(0);
        let Shape::Path { nodes: old, .. } = &original.shape else {
            unreachable!()
        };
        for (i, n) in nodes.iter().enumerate() {
            let source = old
                .iter()
                .find(|s| original_world(s.point) == world(n.point))
                .unwrap();
            for (actual, expected) in [
                (
                    n.incoming,
                    if i == 0 {
                        source.point
                    } else {
                        source.incoming
                    },
                ),
                (
                    n.outgoing,
                    if i == 2 {
                        source.point
                    } else {
                        source.outgoing
                    },
                ),
            ] {
                let [x, y] = world(actual);
                let [u, v] = original_world(expected);
                assert!((x - u).hypot(y - v) < 0.001);
            }
        }
    }
    doc.validate().unwrap();
}

#[test]
fn cutting_inside_an_open_cubic_retains_two_curves_and_the_solver_endpoint() {
    let mut doc = Document::default();
    let mut start = PathNode::corner([0., 0.]);
    start.outgoing = [1. / 3., 1.];
    let mut end = PathNode::corner([1., 0.]);
    end.incoming = [2. / 3., 1.];
    let a = doc.add_shape(
        [0.; 2],
        [96.; 2],
        [255; 4],
        Shape::Path {
            nodes: vec![start, end],
            closed: false,
        },
        1,
    );
    let b = vertical_cutter(&mut doc, 37.25);
    assert_eq!(
        doc.boolean(&[a, b], 0, BooleanOp::CutPath).unwrap().len(),
        2
    );
    let mut endpoints = Vec::new();
    for piece in &doc.objects {
        let Shape::Path {
            nodes,
            closed: false,
        } = &piece.shape
        else {
            panic!("expected an open cubic")
        };
        assert_eq!(nodes.len(), 2);
        let world = piece.local_to_world_at(0);
        let [a, b, c, d] = [
            nodes[0].point,
            nodes[0].outgoing,
            nodes[1].incoming,
            nodes[1].point,
        ]
        .map(world);
        endpoints.extend([a, d]);
        for i in 0..=32 {
            let t = i as f32 / 32.;
            let u = 1. - t;
            let [x, y]: [f32; 2] = std::array::from_fn(|j| {
                u.powi(3) * a[j] + 3. * u * u * t * b[j] + 3. * u * t * t * c[j] + t.powi(3) * d[j]
            });
            let source_t = x / 96.;
            assert!((y - 288. * source_t * (1. - source_t)).abs() < 0.022);
        }
    }
    assert_eq!(endpoints[1], endpoints[2]);
    assert_eq!(endpoints[1][0], 37.25);
    doc.validate().unwrap();
}

#[test]
fn retraced_cubic_keeps_the_ordered_polyline_for_each_ambiguous_chord() {
    let mut doc = Document::default();
    let mut first = PathNode::corner([0., 0.]);
    first.outgoing = [0., 1.];
    let mut turn = PathNode::corner([1., 0.]);
    turn.incoming = [1., 1.];
    turn.outgoing = turn.incoming;
    let mut last = PathNode::corner(first.point);
    last.incoming = first.outgoing;
    let a = doc.add_shape(
        [0.; 2],
        [96.; 2],
        [255; 4],
        Shape::Path {
            nodes: vec![first, turn, last],
            closed: false,
        },
        1,
    );
    let b = vertical_cutter(&mut doc, 48.);
    assert_eq!(
        doc.boolean(&[a, b], 0, BooleanOp::CutPath).unwrap().len(),
        3
    );
    for p in &doc.objects {
        let Shape::Path {
            nodes,
            closed: false,
        } = &p.shape
        else {
            panic!("expected open path")
        };
        assert!(nodes.len() > 2);
        assert!(nodes
            .iter()
            .all(|n| n.point == n.incoming && n.point == n.outgoing));
    }
    doc.validate().unwrap();
}

#[test]
fn cutting_one_compound_contour_leaves_the_other_closed_and_editable() {
    let Shape::Path { nodes, .. } = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap()
    else {
        unreachable!()
    };
    let mut second = nodes.clone();
    for n in &mut second {
        for p in [&mut n.point, &mut n.incoming, &mut n.outgoing] {
            p[0] += 2.;
        }
    }
    let mut doc = Document::default();
    let a = doc.add_shape(
        [0.; 2],
        [96.; 2],
        [255; 4],
        Shape::Compound {
            contours: vec![nodes, second],
        },
        1,
    );
    let b = vertical_cutter(&mut doc, 48.);
    assert_eq!(
        doc.boolean(&[a, b], 0, BooleanOp::CutPath).unwrap().len(),
        3
    );
    let Shape::Path {
        nodes,
        closed: true,
    } = &doc.objects[2].shape
    else {
        panic!("expected an uncut closed circle")
    };
    assert_eq!(nodes.len(), 4);
    assert!(nodes
        .iter()
        .all(|n| n.point != n.incoming && n.point != n.outgoing));
    doc.validate().unwrap();
}
