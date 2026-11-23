use document_core::{Document, FillRule, Layer, PathNode, Shape, Stencil};

fn setup() -> (Document, [u64; 3]) {
    let mut doc = Document::default();
    let a = doc.add_rectangle([0., 1.], [45., 40.], [155, 0, 90, 255]);
    let b = doc.add_rectangle([18., 3.], [34., 36.], [90, 180, 0, 255]);
    doc.objects[0].group = Some(17);
    doc.objects[1].group = Some(17);
    doc.objects[1].rotation = -18.;
    let c = doc.add_shape(
        [12., 8.],
        [23., 18.],
        [231, 40, 80, 193],
        Shape::Path {
            nodes: [[0., 0.], [1., 0.35], [0.24, 1.]]
                .map(PathNode::corner)
                .to_vec(),
            closed: true,
        },
        1,
    );
    let o = doc.objects.last_mut().unwrap();
    o.name = "Cutout".into();
    o.stroke = [88, 17, 153, 255];
    o.stroke_width = 2.75;
    o.fill_rule = FillRule::EvenOdd;
    o.rotation = 14.;
    (doc, [a, b, c])
}

#[test]
fn clip_group_retains_selection_layers_stacking_and_recovers_the_original_path_exactly() {
    let (mut doc, [a, b, c]) = setup();
    let _front = doc.add_rectangle([100.; 2], [5.; 2], [230; 4]);
    doc.objects[2]
        .keys
        .extend([(0, [10., 18.]), (3, [16., 19.])]);
    let initial = doc.clone();
    assert_eq!(doc.set_clip(&[c, a], 2, None).unwrap(), [a, b]);
    assert!(doc.objects[0].shares_clip_mask(&doc.objects[1], 2));
    let restored = doc.release_clip(&[b], 2).unwrap();
    assert_eq!(restored, [a, b, c]);
    assert_eq!(doc, initial);
    doc.validate().unwrap();
}

#[test]
fn scaled_rotated_group_and_independently_moving_members_release_at_the_displayed_pose() {
    let (mut doc, [a, b, c]) = setup();
    let original = doc.objects[2].clone();
    doc.set_clip(&[a, c], 0, Some([75, 205])).unwrap();
    // Both members receive the same uniform scale around the world origin and translation.
    let map = |p: [f32; 2]| [p[0] * 1.5 + 31., p[1] * 1.5 - 9.];
    for o in &mut doc.objects {
        let center = map(o.local_to_world([0.5, 0.5], 0));
        o.set_scale_at(0, [1.5, 1.5]);
        o.set_position_at(
            0,
            [center[0] - o.size[0] * 0.5, center[1] - o.size[1] * 0.5],
        );
    }
    assert!(doc.objects[0].shares_clip_mask(&doc.objects[1], 0));
    let left = doc.objects[0].stencil_alpha_space(0).unwrap();
    let right = doc.objects[1].stencil_alpha_space(0).unwrap();
    assert!((left.tx - right.tx).abs() < 0.001 && (left.sx - right.sx).abs() < 0.001);
    let mut separate = doc.clone();
    separate.objects[1].position[0] += 8.;
    assert!(!separate.objects[0].shares_clip_mask(&separate.objects[1], 0));
    let restored = separate.release_clip(&[a, b], 0).unwrap();
    assert_eq!(
        restored.len(),
        4,
        "distinct current silhouettes remain available"
    );
    assert_eq!(doc.release_clip(&[a], 0).unwrap(), [a, b, c]);
    for p in [[0., 0.], [1., 0.], [0.24, 1.], [0.5, 0.5]] {
        let got = doc.objects[2].local_to_world(p, 0);
        let expected = map(original.local_to_world(p, 0));
        assert!((got[0] - expected[0]).abs() < 0.002 && (got[1] - expected[1]).abs() < 0.002);
    }
    assert_eq!(doc.objects[2].shape, original.shape);
}

#[test]
fn independent_copies_on_another_layer_restore_distinct_ids_in_their_current_layer() {
    let (mut doc, [a, _, c]) = setup();
    doc.objects[0].group = None;
    doc.objects[1].group = None;
    doc.set_clip(&[a, c], 0, None).unwrap();
    let mut duplicate = doc.objects[0].clone();
    duplicate.id = 90;
    duplicate.layer = 2;
    duplicate.position[0] += 48.;
    doc.layers.push(Layer {
        id: 2,
        ..Layer::default()
    });
    doc.objects.push(duplicate);
    let copied_id = *doc.release_clip(&[90], 0).unwrap().last().unwrap();
    assert_ne!(copied_id, c);
    let restored = doc.objects.iter().find(|o| o.id == copied_id).unwrap();
    assert_eq!(restored.layer, 2);
    assert!((restored.position[0] - 60.).abs() < 0.001);
    assert_eq!(doc.release_clip(&[a], 0).unwrap(), [a, c]);
    doc.validate().unwrap();
}

#[test]
fn existing_stencils_release_into_editable_world_geometry_and_invalid_sets_are_atomic() {
    let (mut doc, [a, b, c]) = setup();
    doc.objects[0].stencil = Some(Stencil {
        shape: Shape::Ellipse,
        fill_rule: FillRule::NonZero,
        origin: [0.2, 0.3],
        size: [0.6, 0.5],
        alpha: Some([0, 255]),
    });
    let before = doc.clone();
    assert!(doc.set_clip(&[c, b], 1, None).is_err());
    assert_eq!(doc, before);
    let outline = doc.objects[0]
        .stencil
        .as_ref()
        .unwrap()
        .outline(doc.objects[0].paint_space(1).unwrap())
        .unwrap();
    let selected = doc.release_clip(&[a], 1).unwrap();
    let source = doc.objects.iter().find(|o| o.id == selected[1]).unwrap();
    let expected = outline.bounds();
    assert_eq!(source.fill, [0, 0, 0, 255]);
    assert!(matches!(&source.shape,Shape::Path{closed:true,nodes} if nodes.len()>=4));
    let got = source.outline(1).unwrap().bounds();
    assert!(
        (got.left() - expected.left()).abs() < 0.001
            && (got.bottom() - expected.bottom()).abs() < 0.001
    );
    doc.validate().unwrap();
}

#[test]
fn symbol_capture_preserves_the_group_mask_and_the_releasable_path_in_local_coordinates() {
    let (mut doc, [a, b, c]) = setup();
    let original = doc.objects[2].clone();
    doc.set_clip(&[a, c], 0, Some([65, 195])).unwrap();
    let (symbol, origin) =
        document_core::SymbolDefinition::capture("Cutout", &doc, 0, &[a, b]).unwrap();
    let mut art = (*symbol.artwork).clone();
    assert!(art.objects[0].shares_clip_mask(&art.objects[1], 0));
    let id = art.objects[0].id;
    let ids = art.release_clip(&[id], 0).unwrap();
    assert_eq!(ids.len(), 3);
    let restored = art.objects.iter().find(|o| o.id == ids[2]).unwrap();
    for p in [[0., 0.], [1., 0.], [1., 1.]] {
        let placed = restored.local_to_world(p, 0);
        let world = original.local_to_world(p, 0);
        assert!((placed[0] + origin[0] - world[0]).abs() < 0.002);
        assert!((placed[1] + origin[1] - world[1]).abs() < 0.002);
    }
}

#[test]
fn a_stroked_horizontal_open_path_can_be_clipped_and_released() {
    let mut doc = Document::default();
    let line = doc.add_shape(
        [5., 12.],
        [80., 1.],
        [0; 4],
        Shape::Path {
            closed: false,
            nodes: [[0., 0.], [1., 0.]].map(PathNode::corner).to_vec(),
        },
        1,
    );
    doc.objects[0].stroke_width = 4.;
    let upper = doc.add_rectangle([10., 5.], [20., 25.], [5; 4]);
    let initial = doc.clone();
    doc.set_clip(&[line, upper], 0, None).unwrap();
    assert_eq!(doc.hit_test([14., 12.], 0), Some(line));
    assert_eq!(doc.hit_test([58., 12.], 0), None);
    doc.release_clip(&[line], 0).unwrap();
    assert_eq!(doc, initial);
}
