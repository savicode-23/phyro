use crate::{BooleanOp, Document, FillRule, PathNode, Shape, VectorObject};

// Match every evaluated anchor and both cubic handles across coordinate frames.
// Reversing a closed contour swaps incoming and outgoing handles at each anchor.
fn reversed(
    source: &VectorObject,
    contour: &[PathNode],
    result: &VectorObject,
    frame: u32,
) -> bool {
    let same = |p, q| {
        let a = result.local_to_world(p, frame);
        let b = source.local_to_world(q, frame);
        (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) < 0.00005
    };
    let mut orientation = None;
    for node in contour {
        let matched = result
            .shape
            .contours()
            .iter()
            .flatten()
            .find(|n| same(n.point, node.point))
            .expect("source anchor survives at its evaluated coordinates");
        let forward =
            same(matched.incoming, node.incoming) && same(matched.outgoing, node.outgoing);
        let reverse =
            same(matched.incoming, node.outgoing) && same(matched.outgoing, node.incoming);
        assert!(forward ^ reverse);
        assert_eq!(*orientation.get_or_insert(reverse), reverse);
    }
    orientation.unwrap()
}

#[test]
fn preserved_rings_match_operand_fills_and_hole_winding_in_each_operation() {
    for primitive in [false, true] {
        for inside in [true, false] {
            let shape = if primitive {
                Shape::Ellipse
            } else {
                crate::path_edit::primitive_path(&Shape::Ellipse).unwrap()
            };
            let mut original = Document::default();
            let a =
                original.add_shape([10., 12.], [80., 65.], [17, 88, 195, 255], shape.clone(), 1);
            let b = original.add_shape(
                if inside { [31., 34.] } else { [105., 20.] },
                [17., 12.],
                [45; 4],
                shape.clone(),
                1,
            );
            original.objects[0].rotation_keys.insert(2, 12.);
            original.objects[0].scale = [1.13, 0.84];
            original.objects[0].fill_rule = FillRule::EvenOdd;
            original.objects[1].rotation = -23.;
            original.objects[1].animated.skew = 13.;
            for op in [
                BooleanOp::Union,
                BooleanOp::Difference,
                BooleanOp::Exclusion,
                BooleanOp::Intersection,
                BooleanOp::Division,
            ] {
                let mut doc = original.clone();
                let out = doc.boolean(&[b, a], 2, op);
                if !inside && op == BooleanOp::Division {
                    assert!(out.is_err());
                    assert_eq!(doc, original);
                    continue;
                }
                let ids = out.unwrap();
                doc.validate().unwrap();
                if !ids.is_empty() {
                    assert_eq!(ids[0], a);
                }
                for o in &doc.objects {
                    assert_eq!(o.fill, original.objects[0].fill);
                    assert!(o.rotation_keys.is_empty());
                }
                let total: usize = doc
                    .objects
                    .iter()
                    .flat_map(|o| o.shape.contours())
                    .map(Vec::len)
                    .sum();
                assert!(total <= if primitive { 96 } else { 12 }, "{op:?}: {total}");
                for y in 0..47 {
                    for x in 0..67 {
                        let p = [2. * x as f32 + 0.37, 2. * y as f32 + 0.51];
                        let hits = |p| {
                            [&original.objects[0], &original.objects[1]].map(|o| o.contains(p, 2))
                        };
                        let old = hits(p);
                        if [[0.05, 0.], [-0.05, 0.], [0., 0.05], [0., -0.05]]
                            .into_iter()
                            .any(|q| hits([p[0] + q[0], p[1] + q[1]]) != old)
                        {
                            continue;
                        }
                        let expected = match op {
                            BooleanOp::Union => old[0] || old[1],
                            BooleanOp::Difference => old[0] && !old[1],
                            BooleanOp::Intersection => old[0] && old[1],
                            BooleanOp::Exclusion => old[0] ^ old[1],
                            BooleanOp::Division => old[0],
                            _ => unreachable!(),
                        };
                        assert_eq!(
                            doc.objects.iter().any(|o| o.contains(p, 2)),
                            expected,
                            "{op:?} {p:?}"
                        );
                    }
                }
                if !primitive && inside && op == BooleanOp::Difference {
                    let result = &doc.objects[0];
                    let orientation =
                        |o: &VectorObject| reversed(o, &o.shape.contours()[0], result, 2);
                    assert_ne!(
                        orientation(&original.objects[0]),
                        orientation(&original.objects[1])
                    );
                }
            }
        }
    }
}

#[test]
fn tangent_and_near_contact_keep_the_integer_overlay_result() {
    for x in [20., 20.01] {
        let mut d = Document::default();
        let a = d.add_shape([0.; 2], [20.; 2], [255; 4], Shape::Ellipse, 1);
        let b = d.add_shape([x, 0.], [20.; 2], [255; 4], Shape::Ellipse, 1);
        d.boolean(&[a, b], 0, BooleanOp::Union).unwrap();
        assert!(
            d.objects
                .iter()
                .flat_map(|o| o.shape.contours())
                .flatten()
                .all(|n| n.point == n.incoming && n.point == n.outgoing),
            "x={x}"
        );
    }
}

#[test]
fn union_preserves_each_unchanged_curve_among_three_operands() {
    let ellipse = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap();
    for second_x in [75., 35.] {
        let mut doc = Document::default();
        let ids = [10., second_x, 165.]
            .map(|x| doc.add_shape([x, 12.], [42., 35.], [155; 4], ellipse.clone(), 1));
        doc.objects[2].rotation = 22.;
        let source = doc.clone();
        assert_eq!(doc.boolean(&ids, 0, BooleanOp::Union).unwrap(), [ids[0]]);
        let result = &doc.objects[0];
        if second_x == 75. {
            assert_eq!(
                result
                    .shape
                    .contours()
                    .iter()
                    .map(Vec::len)
                    .collect::<Vec<_>>(),
                [4, 4, 4]
            );
            for object in &source.objects {
                reversed(object, &object.shape.contours()[0], result, 0);
            }
        } else {
            assert_eq!(result.shape.contours().len(), 2);
            assert!(result.shape.contours().iter().any(|c| c.len() == 4));
            reversed(&source.objects[2], &ellipse.contours()[0], result, 0);
        }
    }
}

#[test]
fn exact_duplicates_keep_the_original_controls_in_the_back_paint_slot() {
    for ellipse in [
        Shape::Ellipse,
        crate::path_edit::primitive_path(&Shape::Ellipse).unwrap(),
    ] {
        let mut doc = Document::default();
        let back = doc.add_shape(
            [12., 8.],
            [47., 31.],
            [92, 64, 175, 255],
            ellipse.clone(),
            1,
        );
        let front = doc.add_shape([12., 8.], [47., 31.], [198; 4], ellipse.clone(), 1);
        for o in &mut doc.objects {
            o.rotation = 17.;
            o.animated.skew = 9.;
        }
        let source = doc.objects[0].clone();
        assert_eq!(
            doc.boolean(&[front, back], 0, BooleanOp::Union).unwrap(),
            [back]
        );
        let result = &doc.objects[0];
        assert_eq!(result.fill, source.fill);
        let outlines = crate::geometry::editable_contours(&source.outline(0).unwrap(), |p| {
            source.world_to_local([p.x, p.y], 0)
        })
        .unwrap();
        assert_eq!(result.shape.contours()[0].len(), outlines[0].len());
        reversed(&source, &outlines[0], result, 0);
    }
}

#[test]
fn crossing_authored_curves_keep_handles_and_exposed_corners() {
    let mut left = PathNode::corner([0., 0.8]);
    left.outgoing = [0., 0.25];
    let top = PathNode {
        handle_style: Default::default(),
        point: [0.5, 0.02],
        incoming: [0.22, 0.02],
        outgoing: [0.81, 0.02],
    };
    let mut right = PathNode::corner([1., 0.8]);
    right.incoming = [1., 0.26];
    let shape = Shape::Path {
        closed: true,
        nodes: vec![left, top, right],
    };
    let mut original = Document::default();
    let a = original.add_shape([13., 22.], [58., 48.], [125; 4], shape.clone(), 1);
    let b = original.add_shape([37., 17.], [55., 45.], [255; 4], shape, 1);
    original.objects[0].rotation = 9.;
    original.objects[1].animated.skew = -12.;
    for op in [
        BooleanOp::Union,
        BooleanOp::Difference,
        BooleanOp::Intersection,
    ] {
        let mut result = original.clone();
        assert_eq!(result.boolean(&[b, a], 0, op).unwrap(), [a]);
        let output = &result.objects[0];
        assert_eq!(output.shape.contours().len(), 1);
        let nodes = &output.shape.contours()[0];
        assert!(nodes.len() <= 16, "{op:?}: {} authored knots", nodes.len());
        assert!(nodes
            .iter()
            .any(|n| n.point != n.outgoing && n.point != n.incoming));
        if op == BooleanOp::Union {
            let corner = original.objects[0].local_to_world(left.point, 0);
            assert!(
                nodes.iter().any(|n| {
                    let p = output.local_to_world(n.point, 0);
                    (p[0] - corner[0]).hypot(p[1] - corner[1]) < 0.001
                        && (n.incoming == n.point || n.outgoing == n.point)
                }),
                "the exposed endpoint of the straight base must remain a corner"
            );
        }
    }
}

#[test]
fn moving_and_cutting_a_compound_keeps_each_existing_curved_boundary() {
    for (primitive, shape) in [
        (
            false,
            crate::path_edit::primitive_path(&Shape::Ellipse).unwrap(),
        ),
        (true, Shape::Ellipse),
    ] {
        let mut d = Document::default();
        let base = d.add_shape(
            [10., 10.],
            [95., 70.],
            [80, 100, 170, 222],
            shape.clone(),
            1,
        );
        let first = d.add_shape([36., 35.], [12., 14.], [255; 4], shape.clone(), 1);
        let hole_center = [42., 42.];
        d.boolean(&[base, first], 0, BooleanOp::Difference).unwrap();
        let back = &mut d.objects[0];
        let first_local = back.world_to_local(hole_center, 0);
        back.position[0] += 8.25;
        back.position[1] += 11.5;
        back.rotation = 17.3;
        back.animated.skew = 8.5;
        back.scale = [1.18, 0.93];
        let old_hole = back.local_to_world(first_local, 0);
        let new_hole = back.local_to_world([0.70, 0.51], 0);
        let second = d.add_shape(
            [new_hole[0] - 6., new_hole[1] - 6.],
            [12., 12.],
            [255; 4],
            shape,
            1,
        );
        let old = d.clone();
        let ids = d
            .boolean(&[second, base], 0, BooleanOp::Difference)
            .unwrap();
        assert_eq!(ids, [base]);
        d.validate().unwrap();
        let result = &d.objects[0];
        let nodes: Vec<_> = result.shape.contours().iter().map(Vec::len).collect();
        assert_eq!(nodes.len(), 3);
        assert!(nodes.iter().all(|&n| n <= 32));
        if !primitive {
            assert_eq!(nodes, [4, 4, 4]);
        }
        assert!(d.hit_test(old_hole, 0).is_none());
        assert!(d.hit_test(new_hole, 0).is_none());
        for contour in old.objects[0].shape.contours() {
            assert!(!reversed(&old.objects[0], contour, result, 0));
        }
    }
}

#[test]
fn an_existing_compounds_fill_rule_controls_which_boundary_is_a_hole() {
    for (rule, old_hole) in [(FillRule::EvenOdd, true), (FillRule::NonZero, false)] {
        let Shape::Path { nodes, .. } = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap()
        else {
            unreachable!()
        };
        let mut inner = nodes.clone();
        for node in &mut inner {
            for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                p[0] = 0.27 + 0.14 * p[0];
                p[1] = 0.43 + 0.14 * p[1];
            }
        }
        // Both wind in the same direction. Even-odd removes the inner disk;
        // nonzero keeps it filled and the overlay omits its redundant boundary.
        let mut doc = Document::default();
        let a = doc.add_shape(
            [0.; 2],
            [100.; 2],
            [255; 4],
            Shape::Compound {
                contours: vec![nodes, inner],
            },
            1,
        );
        doc.objects[0].fill_rule = rule;
        let b = doc.add_shape([61., 44.], [12.; 2], [255; 4], Shape::Ellipse, 1);
        doc.boolean(&[a, b], 0, BooleanOp::Difference).unwrap();
        assert_eq!(doc.hit_test([34., 50.], 0).is_none(), old_hole);
        assert!(doc.hit_test([67., 50.], 0).is_none());
        let contours = doc.objects[0].shape.contours();
        assert_eq!(contours.len(), if old_hole { 3 } else { 2 });
        assert!(contours
            .iter()
            .all(|c| c.len() <= 8 && c.iter().any(|n| n.point != n.incoming)));
    }
}

#[test]
fn ambiguous_and_large_contours_use_the_planar_output() {
    let Shape::Path { nodes: circle, .. } =
        crate::path_edit::primitive_path(&Shape::Ellipse).unwrap()
    else {
        unreachable!()
    };
    let mut large = circle.clone();
    for _ in 0..6 {
        for i in (0..large.len()).rev() {
            crate::path_edit::subdivide(&mut large, true, i);
        }
    }
    let mut star: Vec<_> = (0..5)
        .map(|i| {
            let angle = (2 * i % 5) as f32 * std::f32::consts::TAU / 5.;
            PathNode::corner([0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin()])
        })
        .collect();
    // Curve one of the five crossing edges so an accidental reconstruction
    // would be visible as handles on one of the punched-out boundaries.
    star[0].outgoing = [0.64, 0.62];
    assert!(!super::simple(&star));
    for nodes in [large, star] {
        let mut doc = Document::default();
        let a = doc.add_shape([5.; 2], [85.; 2], [255; 4], Shape::Ellipse, 1);
        let b = doc.add_shape(
            [37.; 2],
            [18.; 2],
            [255; 4],
            Shape::Path {
                nodes,
                closed: true,
            },
            1,
        );
        doc.boolean(&[a, b], 0, BooleanOp::Difference).unwrap();
        let rings = doc.objects[0].shape.contours();
        assert!(rings.len() >= 2);
        assert!(rings[0].iter().any(|n| n.point != n.incoming));
        assert!(rings[1..]
            .iter()
            .flatten()
            .all(|n| n.incoming == n.point && n.outgoing == n.point));
    }
}

#[test]
fn an_uncut_sixty_four_anchor_contour_keeps_its_drawn_handles() {
    let Shape::Path { mut nodes, .. } = crate::path_edit::primitive_path(&Shape::Ellipse).unwrap()
    else {
        unreachable!()
    };
    for _ in 0..4 {
        for i in (0..nodes.len()).rev() {
            crate::path_edit::subdivide(&mut nodes, true, i);
        }
    }
    let mut d = Document::default();
    let a = d.add_shape(
        [4.; 2],
        [96.; 2],
        [255; 4],
        Shape::Path {
            nodes,
            closed: true,
        },
        1,
    );
    let b = d.add_rectangle([120., 8.], [30.; 2], [255; 4]);
    let original = d.objects[0].clone();
    d.boolean(&[a, b], 0, BooleanOp::Union).unwrap();
    let contours = d.objects[0].shape.contours();
    assert!(contours.iter().any(|c| c.len() == 64));
    reversed(&original, &original.shape.contours()[0], &d.objects[0], 0);
}

mod crossings;

#[test]
fn subdivision_rejects_a_loop_inside_one_cubic_and_a_sub_tolerance_notch() {
    let mut looped = [[0., 0.], [20., 0.], [20., -20.], [0., -20.]].map(PathNode::corner);
    looped[0].outgoing = [90., 60.];
    looped[1].incoming = [-90., 60.];
    assert!(!super::simple(&looped));
    let notch = [
        [0., 0.],
        [9., 0.],
        [9., 9.],
        [5.003, 9.],
        [5.003, 3.],
        [4.997, 3.],
        [4.997, 9.],
        [0., 9.],
    ]
    .map(PathNode::corner);
    assert!(!super::simple(&notch));
}
