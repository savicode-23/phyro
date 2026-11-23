use super::*;
use document_core::{AnimationProject, Layer, LineCap, PathNode, Shape, Stencil, SymbolInstance};

fn compare(gesture: &mut DragPreview, before: &Document, after: &Document, frame: u32, scale: f32) {
    let preview = gesture.render(before, after, frame, scale).unwrap();
    let reference = crate::render_scaled(after, frame, scale).unwrap();
    let mismatch = preview
        .pixels()
        .iter()
        .zip(reference.pixels())
        .position(|(a, b)| a != b);
    assert_eq!(mismatch, None, "frame {frame} at scale {scale}");
}

#[test]
fn live_edits_match_full_render_through_overlaps_layer_changes_and_zoom() {
    let mut before = Document::default();
    before.add_rectangle([1.5, 3.], [48., 50.], [75, 194, 85, 171]);
    before.add_rectangle([4., 6.2], [25., 34.], [235, 170, 95, 223]);
    before.layers.push(Layer {
        id: 2,
        name: "Overlay".into(),
        visible: true,
        locked: true,
    });
    before.add_shape([8., 9.], [40., 35.], [60, 87, 242, 202], Shape::Ellipse, 2);
    let front = before.objects.last_mut().unwrap();
    front.stroke_width = 2.;
    front.opacity = 0.75;
    front.stencil = Some(Stencil {
        shape: Shape::Ellipse,
        origin: [0., 0.],
        size: [1., 1.],
        fill_rule: Default::default(),
        alpha: Some([72, 218]),
    });
    let mut gesture = DragPreview::default();
    let mut preview = before.clone();
    let compare = |gesture: &mut DragPreview, doc: &Document, frame, scale| {
        let actual = gesture.render(&before, doc, frame, scale).unwrap();
        let expected = crate::render_scaled(doc, frame, scale).unwrap();
        assert!(actual.data() == expected.data());
    };
    for x in [18.2, 28.1, 6.] {
        preview.objects[1].position[0] = x;
        compare(&mut gesture, &preview, 0, 1.5);
    }
    assert_eq!(gesture.background.as_ref().unwrap().prefix, 1);
    // An earlier paint changed while the gesture was active.
    preview.objects[0].fill[1] = 30;
    compare(&mut gesture, &preview, 0, 1.5);
    assert_eq!(gesture.background.as_ref().unwrap().prefix, 0);
    preview.layers.swap(0, 1);
    compare(&mut gesture, &preview, 1, 1.25);
    preview.objects.remove(1);
    preview.layers[0].visible = false;
    compare(&mut gesture, &preview, 1, 1.25);
    preview.width = 83;
    compare(&mut gesture, &preview, 1, 1.25);
}

#[test]
fn shared_preview_reuses_static_art_and_rechecks_foreground_and_layer_order() {
    let mut before = Document::default();
    before.add_rectangle([0., 0.], [80., 80.], [128, 40, 80, 255]);
    before.add_rectangle([5., 5.], [14., 18.], [32, 180, 240, 216]);
    before.layers.push(Layer {
        id: 2,
        name: "Glass".into(),
        visible: true,
        locked: false,
    });
    before.add_shape(
        [10., 12.],
        [21., 20.],
        [170, 90, 72, 135],
        Shape::Ellipse,
        2,
    );
    let mut preview = Arc::new(before.clone());
    let before = Arc::new(before);
    let mut gesture = DragPreview::default();
    let check = |gesture: &mut DragPreview, doc: &Document| {
        let actual = gesture.render_from_snapshot(&before, doc, 0, 1.25).unwrap();
        assert!(actual.data() == crate::render_scaled(doc, 0, 1.25).unwrap().data());
    };
    Arc::get_mut(&mut preview).unwrap().objects[1].position = [15., 12.];
    check(&mut gesture, &preview);
    assert_eq!(Arc::strong_count(&before), 2);
    // The canvas can recycle the same preview allocation throughout the drag.
    Arc::get_mut(&mut preview).unwrap().objects[1].position = [23., 14.];
    check(&mut gesture, &preview);
    Arc::get_mut(&mut preview).unwrap().objects[2].opacity = 0.2;
    check(&mut gesture, &preview);
    Arc::get_mut(&mut preview).unwrap().layers.swap(0, 1);
    check(&mut gesture, &preview);
    // Releasing or cancelling can finish on a borrowed scene.
    compare(&mut gesture, &before, &before, 0, 1.25);
    drop(gesture);
    assert_eq!(Arc::strong_count(&before), 1);
}

#[test]
fn parked_foreground_is_culled_until_it_moves_over_the_edit() {
    let mut before = Document::default();
    before.add_rectangle([4., 4.], [20.; 2], [180, 20, 90, 218]);
    for i in 0..128 {
        before.add_rectangle([320. + i as f32, 5.], [15.; 2], [12, 143, 190, 192]);
    }
    let mut after = before.clone();
    let mut gesture = DragPreview::default();
    after.objects[0].position = [9., 7.];
    compare(&mut gesture, &before, &after, 0, 1.5);
    let foreground = gesture
        .background
        .as_ref()
        .unwrap()
        .foreground
        .as_ref()
        .unwrap();
    assert_eq!(foreground.len, 128);
    assert!(foreground.visible.is_empty());
    // An independent inspector edit invalidates the cached foreground.
    after.objects[80].position = [15., 10.];
    compare(&mut gesture, &before, &after, 0, 1.5);
    after.objects[0].position = [13., 8.];
    compare(&mut gesture, &before, &after, 0, 1.5);
}

#[test]
fn single_replacement_checks_masks_new_sources_and_target_changes() {
    let mut doc = Document::default();
    let first = doc.add_rectangle([10., 10.], [30., 25.], [34, 194, 120, 195]);
    let second = doc.add_rectangle([16., 12.], [25., 25.], [78, 100, 210, 201]);
    for o in &mut doc.objects {
        o.group = Some(1);
    }
    let mask = doc.add_rectangle([12., 8.], [36., 38.], [255; 4]);
    doc.set_clip(&[first, second, mask], 0, Some([70, 195]))
        .unwrap();
    doc.add_rectangle([90., 46.], [8.; 2], [255; 4]);
    let mut source = Arc::new(doc);
    let mut gesture = DragPreview::default();
    for (slot, x) in [(0, 4.), (0, 17.), (2, 70.), (1, 31.), (0, 12.)] {
        let mut preview = source.as_ref().clone();
        preview.objects[slot].position[0] = x;
        let actual = gesture
            .render_object_edit(&source, &preview.objects[slot], 0, 1.5)
            .unwrap();
        assert!(actual.data() == crate::render_scaled(&preview, 0, 1.5).unwrap().data());
        if slot == 2 {
            Arc::make_mut(&mut source).objects[0].opacity = 0.35;
        }
    }
}

#[test]
fn cached_foreground_matches_hairlines_joins_reorders_and_multi_object_edits() {
    let mut before = Document::default();
    before.add_rectangle([0.; 2], [128.; 2], [35, 57, 52, 211]);
    before.add_rectangle([45., 20.], [30., 43.], [110, 30, 153, 92]);
    let shape = Shape::Path {
        nodes: [[0., 0.96], [0.5, -0.3], [0.62, 1.2]]
            .map(PathNode::corner)
            .to_vec(),
        closed: false,
    };
    before.add_shape([6.2, 19.3], [33., 31.], [0; 4], shape, 1);
    let line = &mut before.objects[2];
    line.stroke_width = 0.24;
    line.stroke = [201, 167, 163, 223];
    line.cap = LineCap::Square;
    before.add_shape(
        [58., 17.5],
        [26., 53.],
        [70, 188, 234, 145],
        Shape::Ellipse,
        1,
    );
    let mut preview = before.clone();
    let mut gesture = DragPreview::default();
    for x in [-50., 11., 49., 70., 143.] {
        preview.objects[2].position[0] = x;
        preview.objects[2].rotation += 21.3;
        preview.objects[2].scale = [2.3, 0.7];
        compare(&mut gesture, &before, &preview, 0, 1.37);
    }
    preview.objects[2].stroke_width = 7.81;
    preview.objects[1].rotation = -23.;
    compare(&mut gesture, &before, &preview, 0, 1.37);
    preview.objects[2].position[0] = 51.;
    compare(&mut gesture, &before, &preview, 0, 1.37);
    preview.objects.swap(1, 2);
    compare(&mut gesture, &before, &preview, 0, 1.37);
    preview.objects[3].opacity = 0.3;
    compare(&mut gesture, &before, &preview, 0, 1.37);
}

#[test]
fn skinned_cloth_crosses_overflowing_symbols_and_nested_clip_viewports() {
    let mut doc = Document::default();
    let upper = doc.add_bone("Upper", None, [20., 40.], [35., 43.]).unwrap();
    let lower = doc
        .add_bone("Lower", Some(upper), [35., 43.], [47., 50.])
        .unwrap();
    let cloth = Shape::Path {
        nodes: [[-0.12, 0.1], [1.1, 0.2], [0.85, 0.95], [0.1, 1.]]
            .map(PathNode::corner)
            .to_vec(),
        closed: true,
    };
    let sleeve = doc.add_shape([20., 39.], [31., 13.], [167, 56, 92, 226], cloth, 1);
    doc.bind_skin_at(sleeve, lower, 0).unwrap();
    doc.set_bone_rotation_key(lower, 0, 0.).unwrap();
    doc.set_bone_rotation_key(lower, 4, 74.).unwrap();

    // Static symbols can paint past all four sides of their nominal viewport.
    let mut symbol = Document {
        width: 10,
        height: 10,
        ..Document::default()
    };
    symbol.add_shape(
        [-8., -7.],
        [27., 19.],
        [60, 150, 103, 157],
        Shape::Ellipse,
        1,
    );
    doc.add_rectangle([28., 47.], [22., 17.], [255; 4]);

    let mut project = AnimationProject::from_document(doc);
    let middle = project.add_clip("Boot placement").unwrap();
    let child = project.add_clip("Boot art").unwrap();
    let art = project.clips[child].drawing_at_mut(0);
    art.width = 32;
    art.height = 32;
    art.add_rectangle([-12., -15.], [68., 73.], [195, 91, 28, 187]);
    let art = project.clips[middle].drawing_at_mut(0);
    art.width = 32;
    art.height = 32;
    project
        .place_clip(project.clips[child].id, middle, 0, 1, [-4., 2.])
        .unwrap();
    project
        .place_clip(project.clips[middle].id, 0, 0, 1, [42., 37.])
        .unwrap();
    let root = project.clips[0].drawing_at_mut(0);
    root.objects[1].symbol = Some(SymbolInstance::new(1, std::sync::Arc::new(symbol)));
    let boot = root.objects.last_mut().unwrap();
    boot.rotation = 27.3;
    boot.animated.skew = -15.;
    boot.opacity = 0.76;
    let before = root.clone();
    let mut preview = before.clone();
    let mut gesture = DragPreview::default();
    for x in [-70., 30., 45.2, 58.7, 145.] {
        preview.objects[0].position[0] = x;
        compare(&mut gesture, &before, &preview, 3, 1.61);
    }
    // Now the clipped placement itself is the changed object below the static symbol.
    preview.objects.swap(1, 2);
    let before = preview.clone();
    let mut gesture = DragPreview::default();
    for x in [-60., -1., 27.9, 154.] {
        preview.objects[1].position[0] = x;
        preview.objects[1].scale = [1.8, 0.61];
        compare(&mut gesture, &before, &preview, 3, 1.61);
    }
}
