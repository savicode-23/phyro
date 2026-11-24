use super::*;

mod responsiveness;

fn rectangles(a: [f32; 4], b: [f32; 4]) -> (Document, [u64; 2]) {
    let mut doc = Document::default();
    let ids = [a, b].map(|[x, y, w, h]| doc.add_rectangle([x, y], [w, h], [255; 4]));
    (doc, ids)
}

#[test]
fn operations_match_the_original_fills_across_overlaps_contacts_and_nesting() {
    for b in [
        [5., 3., 10., 10.],
        [10., 0., 5., 10.],
        [10., 10., 5., 5.],
        [12., 3., 5., 4.],
        [2., 2., 6., 6.],
        [0., 0., 10., 10.],
        [2., 0., 6., 10.],
        [-3., -3., 30., 30.],
    ] {
        let (doc, ids) = rectangles([0., 0., 10., 10.], b);
        for op in [
            BooleanOp::Union,
            BooleanOp::Difference,
            BooleanOp::Intersection,
            BooleanOp::Exclusion,
        ] {
            let mut result = doc.clone();
            result.boolean(&ids, 0, op).unwrap();
            result.validate().unwrap();
            for x in -4..25 {
                for y in -4..25 {
                    let p = [x as f32 + 0.37, y as f32 + 0.59];
                    let (a, b) = (doc.objects[0].contains(p, 0), doc.objects[1].contains(p, 0));
                    let expected = match op {
                        BooleanOp::Union => a || b,
                        BooleanOp::Difference => a && !b,
                        BooleanOp::Intersection => a && b,
                        _ => a ^ b,
                    };
                    assert_eq!(result.hit_test(p, 0).is_some(), expected, "{op:?} {p:?}");
                }
            }
        }
    }
}

#[test]
fn division_returns_independent_islands_and_preserves_z_order_and_ids() {
    let (mut doc, [a, b]) = rectangles([0., 0., 20., 10.], [8., -2., 4., 16.]);
    let front = doc.add_rectangle([30., 0.], [2., 2.], [255; 4]);
    doc.objects[0].fill = [1, 2, 3, 255];
    let ids = doc.boolean(&[b, a], 0, BooleanOp::Division).unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], a);
    assert_eq!(doc.objects.last().unwrap().id, front);
    assert_eq!(
        doc.objects
            .iter()
            .filter(|o| o.fill == [1, 2, 3, 255])
            .count(),
        3
    );
    let hits: HashSet<_> = [4., 10., 16.]
        .map(|x| doc.hit_test([x, 5.], 0).unwrap())
        .into_iter()
        .collect();
    assert_eq!(hits.len(), 3);
    doc.validate().unwrap();
}

#[test]
fn sparse_operands_keep_the_back_layer_slot_and_wrap_ids_around_unrelated_art() {
    let mut original = Document::default();
    original.layers.push(crate::Layer {
        id: 2,
        name: "Cutters".into(),
        visible: true,
        locked: false,
        blend_mode: Default::default(),
    });
    let cutter = original.add_shape([8., -2.], [4., 16.], [197; 4], Shape::Rectangle, 2);
    let below = original.add_rectangle([90.; 2], [3.; 2], [101; 4]);
    let base = original.add_rectangle([0.; 2], [20., 10.], [17, 82, 155, 254]);
    let above = original.add_rectangle([95.; 2], [4.; 2], [102; 4]);
    original.next_id = u64::MAX;
    for op in [
        BooleanOp::Union,
        BooleanOp::Difference,
        BooleanOp::Intersection,
        BooleanOp::Exclusion,
        BooleanOp::Division,
        BooleanOp::CutPath,
    ] {
        let mut doc = original.clone();
        let result = doc.boolean(&[cutter, base], 0, op).unwrap();
        assert_eq!(result[0], base);
        assert_eq!(doc.objects[0], original.objects[1]);
        assert_eq!(doc.objects.last(), original.objects.last());
        assert_eq!(
            doc.objects.iter().map(|o| o.id).collect::<Vec<_>>(),
            [vec![below], result.clone(), vec![above]].concat()
        );
        for o in &doc.objects[1..doc.objects.len() - 1] {
            assert_eq!(o.layer, original.objects[2].layer);
            assert_eq!(o.fill[..3], original.objects[2].fill[..3]);
        }
        if result.len() > 1 {
            assert_eq!(result[1], u64::MAX);
            assert_eq!(doc.next_id, above + result.len() as u64 - 1);
        } else {
            assert_eq!(doc.next_id, u64::MAX);
        }
        doc.validate().unwrap();
    }
    original.objects[0].position = [60.; 2];
    let result = original
        .boolean(&[base, cutter], 0, BooleanOp::Intersection)
        .unwrap();
    assert!(result.is_empty());
    assert_eq!(
        original.objects.iter().map(|o| o.id).collect::<Vec<_>>(),
        [below, above]
    );
    assert_eq!(original.next_id, u64::MAX);
}

#[test]
fn division_attaches_the_hole_to_its_enclosing_piece() {
    let (mut doc, ids) = rectangles([0., 0., 20., 20.], [5., 5., 10., 10.]);
    assert_eq!(doc.boolean(&ids, 0, BooleanOp::Division).unwrap().len(), 2);
    assert_eq!(
        doc.objects
            .iter()
            .filter(|o| o.contains([10., 10.], 0))
            .count(),
        1
    );
    assert_eq!(
        doc.objects
            .iter()
            .filter(|o| o.contains([3., 3.], 0))
            .count(),
        1
    );
    assert!(doc
        .objects
        .iter()
        .any(|o| matches!(&o.shape,Shape::Compound{contours} if contours.len()==2)));
}

#[test]
fn cut_accepts_open_cutters_and_rejects_coincident_centerlines_atomically() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([0., 0.], [10., 10.], [255; 4]);
    let b = doc.add_shape(
        [-2., 5.],
        [14., 1.],
        [0; 4],
        Shape::Path {
            nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([1., 0.])],
            closed: false,
        },
        1,
    );
    let mut coincident = doc.clone();
    coincident.objects[1].position[1] = 0.;
    let before = coincident.clone();
    assert!(coincident.boolean(&[a, b], 0, BooleanOp::CutPath).is_err());
    assert_eq!(coincident, before);
    let out = doc.boolean(&[a, b], 0, BooleanOp::CutPath).unwrap();
    assert_eq!(out.len(), 2);
    assert!(doc
        .objects
        .iter()
        .all(|o| matches!(o.shape, Shape::Path { closed: false, .. })
            && o.no_fill
            && o.stroke_width > 0.));
    let e: Vec<_> = doc
        .objects
        .iter()
        .map(|o| {
            let ns = &o.shape.contours()[0];
            [
                o.local_to_world(ns.first().unwrap().point, 0),
                o.local_to_world(ns.last().unwrap().point, 0),
            ]
        })
        .collect();
    assert_eq!(e[0][0], e[1][1]);
    assert_eq!(e[0][1], e[1][0]);
}

#[test]
fn compound_self_intersections_and_transformed_curves_use_evaluated_geometry() {
    let mut doc = Document::default();
    let bow = doc.add_shape(
        [0., 0.],
        [18., 18.],
        [255; 4],
        Shape::Path {
            nodes: [[0., 0.], [1., 1.], [0., 1.], [1., 0.]]
                .map(PathNode::corner)
                .to_vec(),
            closed: true,
        },
        1,
    );
    let ellipse = doc.add_shape([5., 3.], [20., 9.], [255; 4], Shape::Ellipse, 1);
    doc.objects[1].rotation_keys.insert(2, 35.);
    let before = doc.clone();
    doc.boolean(&[ellipse, bow], 2, BooleanOp::Intersection)
        .unwrap();
    for x in -2..25 {
        for y in -2..23 {
            let p = [x as f32 + 0.12, y as f32 + 0.51];
            let expected = before.objects.iter().all(|o| o.contains(p, 2));
            // Exclude samples within the explicitly bounded curve flattening error.
            if [[0.05, 0.], [0., 0.05], [-0.05, 0.], [0., -0.05]]
                .into_iter()
                .all(|d| {
                    before
                        .objects
                        .iter()
                        .all(|o| o.contains([p[0] + d[0], p[1] + d[1]], 2))
                        == expected
                })
            {
                assert_eq!(doc.hit_test(p, 2).is_some(), expected, "{p:?}");
            }
        }
    }
}

#[test]
fn rejects_open_locked_and_out_of_range_geometry_before_mutation() {
    let (mut doc, ids) = rectangles([0., 0., 10., 10.], [1., 1., 3., 3.]);
    for stage in 0..3 {
        match stage {
            0 => doc.layers[0].locked = true,
            1 => {
                doc.layers[0].locked = false;
                doc.objects[0].position[0] = 200000.;
            }
            _ => {
                doc.objects[0].position[0] = 0.;
                doc.objects[0].shape = Shape::Path {
                    nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([1., 1.])],
                    closed: false,
                };
            }
        }
        let old = doc.clone();
        assert!(doc.boolean(&ids, 0, BooleanOp::Union).is_err());
        assert_eq!(doc, old);
    }
}

#[test]
fn sub_grid_subject_reports_collapse_while_a_grid_wide_sliver_survives() {
    let (mut doc, ids) = rectangles([1., 1., 0.0004, 12.], [0., 0., 2., 14.]);
    let old = doc.clone();
    assert!(doc.boolean(&ids, 0, BooleanOp::Intersection).is_err());
    assert_eq!(doc, old);
    doc.objects[0].size[0] = 1. / 1024.;
    let out = doc.boolean(&ids, 0, BooleanOp::Intersection).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(doc.objects[0].size[0], 1. / 1024.);
}

#[test]
fn static_symbols_and_boolean_results_keep_the_visible_skin_pose() {
    let mut doc = Document::default();
    let parent = doc.add_bone("Arm", None, [10., 10.], [20., 10.]).unwrap();
    let child = doc
        .add_bone("Hand", Some(parent), [20., 10.], [30., 10.])
        .unwrap();
    let shape = crate::path_edit::primitive_path(&Shape::Rectangle).unwrap();
    let id = doc.add_shape([10., 8.], [20., 4.], [255; 4], shape, 1);
    doc.bind_skin_at(id, child, 0).unwrap();
    doc.set_bone_rotation_key(child, 0, 0.).unwrap();
    doc.set_bone_rotation_key(child, 2, 65.).unwrap();
    let posed = doc.objects[0].clone();
    let (symbol, origin) = crate::SymbolDefinition::capture("Bent", &doc, 2, &[id]).unwrap();
    let snapshot = &symbol.artwork.objects[0];
    let local = snapshot.shape.contours();
    let bent = posed.skinned_shape_at(2);
    for (node, source) in local[0].iter().zip(&bent.contours()[0]) {
        let captured = snapshot.local_to_world(node.point, 0);
        let expected = posed.local_to_world(source.point, 2);
        for axis in 0..2 {
            assert!((captured[axis] + origin[axis] - expected[axis]).abs() < 0.001);
        }
    }
    assert!(!symbol.artwork.uses_rig());
    let cover = doc.add_rectangle([0.; 2], [100.; 2], [255; 4]);
    doc.boolean(&[id, cover], 2, BooleanOp::Intersection)
        .unwrap();
    doc.validate().unwrap();
    let baked = &doc.objects[0];
    assert!(baked.skin.is_none());
    for (a, b) in baked.bounds(0).into_iter().zip(posed.bounds(2)) {
        assert!((a - b).abs() < 0.01);
    }
}

#[test]
fn results_bake_layout_effects_and_animated_geometry_without_rebinding() {
    let mut doc = Document::default();
    let text = crate::typography::TextStyle {
        content: "O".into(),
        ..Default::default()
    }
    .insert(&mut doc, 1, [3., 3.], [255; 4])
    .unwrap();
    let cover = doc.add_rectangle([0., 0.], [36., 36.], [255; 4]);
    let before = doc.clone();
    doc.boolean(&[text, cover], 0, BooleanOp::Intersection)
        .unwrap();
    assert!(doc.objects[0].text.is_none() && doc.objects[0].symbol.is_none());
    assert!(doc.objects[0].shape.contours().len() >= 2);
    let mut ink_hits = 0;
    for y in 3..25 {
        for x in 3..20 {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let original = before.objects[0].contains(p, 0);
            if [[0.05, 0.], [-0.05, 0.], [0., 0.05], [0., -0.05]]
                .into_iter()
                .all(|d| before.objects[0].contains([p[0] + d[0], p[1] + d[1]], 0) == original)
            {
                assert_eq!(doc.objects[0].contains(p, 0), original);
            }
            if original {
                ink_hits += 1;
            }
        }
    }
    assert!(ink_hits > 10);
    let mut doc = Document::default();
    let id = doc.add_shape(
        [2., 2.],
        [40., 20.],
        [255; 4],
        Shape::Path {
            nodes: [[0., 0.], [0.5, 0.], [0., 1.]]
                .map(PathNode::corner)
                .to_vec(),
            closed: true,
        },
        1,
    );
    doc.objects[0]
        .effects
        .push(crate::PathEffect::Mirror { horizontal: true });
    doc.objects[0].keys.insert(2, [4., 6.]);
    doc.objects[0].opacity_keys.insert(2, 0.6);
    let clip = doc.add_rectangle([0., 0.], [70., 40.], [255; 4]);
    doc.boolean(&[id, clip], 2, BooleanOp::Intersection)
        .unwrap();
    let out = &doc.objects[0];
    assert!(out.effects.is_empty() && out.keys.is_empty() && out.opacity_keys.is_empty());
    assert_eq!(out.opacity, 0.6);
    assert!(out.contains([40., 8.], 2));
    assert!(!out.contains([8., 8.], 2));
    doc.validate().unwrap();
}

#[test]
fn symbol_instances_and_stencils_are_rejected_before_replacing_their_art() {
    let (mut doc, ids) = rectangles([0., 0., 18., 18.], [4., 4., 8., 8.]);
    doc.objects[0].symbol = Some(crate::SymbolInstance::new(
        1,
        std::sync::Arc::new(Document::default()),
    ));
    let before = doc.clone();
    assert!(doc.boolean(&ids, 0, BooleanOp::Difference).is_err());
    assert_eq!(doc, before);
    doc.objects[0].symbol = None;
    doc.objects[0].stencil = Some(crate::Stencil {
        fill_rule: Default::default(),
        shape: Shape::Ellipse,
        origin: [0.; 2],
        size: [1.; 2],
        alpha: None,
    });
    let before = doc.clone();
    assert!(doc.boolean(&ids, 0, BooleanOp::Difference).is_err());
    assert_eq!(doc, before);
}

#[test]
fn division_splits_by_an_open_path_only_if_it_separates_the_fill() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([0., 0.], [10., 10.], [255; 4]);
    let b = doc.add_shape(
        [-1., 5.],
        [12., 1.],
        [0; 4],
        Shape::Path {
            nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([1., 0.])],
            closed: false,
        },
        1,
    );
    let mut miss = doc.clone();
    miss.objects[1].position[1] = 15.;
    let old = miss.clone();
    assert!(miss.boolean(&[a, b], 0, BooleanOp::Division).is_err());
    assert_eq!(old, miss);
    let ids = doc.boolean(&[a, b], 0, BooleanOp::Division).unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(doc.hit_test([4., 4.], 0), doc.hit_test([4., 7.], 0));
}

#[test]
fn fill_operations_close_an_open_triangle_as_the_canvas_does() {
    let (mut doc, ids) = rectangles([0., 0., 10., 10.], [0., 0., 10., 10.]);
    doc.objects[0].shape = Shape::Path {
        closed: false,
        nodes: [[0., 0.], [1., 0.], [0., 1.]]
            .map(PathNode::corner)
            .to_vec(),
    };
    doc.boolean(&ids, 0, BooleanOp::Intersection).unwrap();
    assert!(doc.hit_test([2., 2.], 0).is_some());
    assert!(doc.hit_test([9., 9.], 0).is_none());
}
