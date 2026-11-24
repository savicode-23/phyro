use super::*;

fn view() -> Rect {
    Rect::from_min_max(egui::pos2(-40., -40.), egui::pos2(120., 120.))
}

#[test]
fn held_selection_upgrades_its_coarse_layers_once_after_first_movement() {
    let mut doc = Document::default();
    let first = doc.add_rectangle([0.; 2], [20.; 2], [255; 4]);
    let second = doc.add_rectangle([20.; 2], [20.; 2], [255; 4]);
    doc.add_rectangle([40.; 2], [20.; 2], [255; 4]);
    let source = Arc::new(doc);
    let ids = [first, second];
    let mut posed = source.as_ref().clone();
    for object in &mut posed.objects[..2] {
        object.position[0] += 5.;
    }
    let ctx = egui::Context::default();
    let mut cache = SelectionMove::default();
    let area = View {
        board: view(),
        scale: 6.,
    };
    cache.prepare(&ctx, &source, &ids, 0, (6., view()), false);
    let request = cache.request.as_ref().unwrap();
    let exact = PreparedLayers::new(
        capture(
            &source,
            &ids,
            0,
            6.,
            request.visible,
            renderer::MAX_SPRITE_EDGE,
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
    // Make the worker result available before first movement to cover the
    // repaint where an affordable upgrade was already waiting in the queue.
    let (send, receiver) = mpsc::channel();
    send.send(Some(exact)).unwrap();
    cache.job = Some(Job {
        receiver,
        ready: None,
        cancel: Default::default(),
    });
    cache.prepare(&ctx, &source, &ids, 0, (6., view()), false);
    assert!(cache.layers.is_none());
    assert!(cache.drag_images(&ctx, &posed, area).is_some());
    let coarse = cache.layers.as_ref().unwrap()[0].texture.size();
    assert!(cache.request.as_ref().unwrap().tried_coarse);

    cache.prepare(&ctx, &source, &ids, 0, (6., view()), false);
    let exact = &cache.layers.as_ref().unwrap()[0].texture;
    assert!(exact.size()[0] > coarse[0]);
    let texture = exact.id();
    assert!(cache.images(&posed, area).is_some());
    assert!(!cache.request.as_ref().unwrap().tried_coarse);
    assert!(cache.job.is_none());
    cache.prepare(&ctx, &source, &ids, 0, (6., view()), false);
    assert_eq!(cache.layers.as_ref().unwrap()[0].texture.id(), texture);
}

#[test]
fn a_large_ready_selection_waits_to_upload_until_idle() {
    let mut doc = Document::default();
    let first = doc.add_rectangle([0.; 2], [1600.; 2], [255; 4]);
    let second = doc.add_rectangle([20.; 2], [10.; 2], [255; 4]);
    let source = Arc::new(doc);
    let ids = [first, second];
    let ctx = egui::Context::default();
    let viewport = Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(1700.));
    let mut cache = SelectionMove::default();
    cache.prepare(&ctx, &source, &ids, 0, (1., viewport), false);
    assert!(cache
        .drag_images(
            &ctx,
            &source,
            View {
                board: viewport,
                scale: 1.
            }
        )
        .is_some());
    let coarse = cache.layers.as_ref().unwrap()[0].texture.id();
    let exact = PreparedLayers::new(
        capture(
            &source,
            &ids,
            0,
            1.,
            viewport,
            renderer::MAX_SPRITE_EDGE,
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
    assert!(exact.bytes > HELD_UPLOAD_BYTES);
    let (send, receiver) = mpsc::channel();
    send.send(Some(exact)).unwrap();
    cache.job = Some(Job {
        receiver,
        ready: None,
        cancel: Default::default(),
    });

    cache.prepare(&ctx, &source, &ids, 0, (1., viewport), false);
    assert_eq!(cache.layers.as_ref().unwrap()[0].texture.id(), coarse);
    assert!(cache.job.as_ref().unwrap().ready.is_some());
    cache.prepare(&ctx, &source, &ids, 0, (1., viewport), true);
    assert_ne!(cache.layers.as_ref().unwrap()[0].texture.id(), coarse);
    assert!(cache.job.is_none());
}

#[test]
fn sparse_selection_in_a_dense_drawing_waits_for_a_transform() {
    let mut doc = Document::default();
    let first = doc.add_rectangle([0.; 2], [8.; 2], [255; 4]);
    let second = doc.add_rectangle([20.; 2], [8.; 2], [255; 4]);
    let distant = doc.objects[0].clone();
    for i in 0..1024 {
        let mut object = distant.clone();
        object.id = second + i + 1;
        object.position = [800. + i as f32 * 16., 600.];
        doc.objects.push(object);
    }
    doc.next_id = second + 1025;
    let source = Arc::new(doc);
    let ctx = egui::Context::default();
    let mut cache = SelectionMove::default();
    cache.prepare(&ctx, &source, &[first, second], 0, (1., view()), true);
    assert!(cache.request.is_none() && cache.job.is_none());
    cache.prepare(&ctx, &source, &[first, second], 0, (1., view()), false);
    assert_eq!(cache.request.as_ref().unwrap().members, [0, 1]);
}

#[test]
fn nonadjacent_members_preserve_paint_order_and_shared_masks_require_one_run() {
    let mut doc = Document::default();
    let a = doc.add_rectangle([-10., 10.], [58., 35.], [180, 20, 100, 160]);
    let b = doc.add_rectangle([10., 5.], [32., 50.], [50, 190, 30, 200]);
    let c = doc.add_rectangle([20., 20.], [35., 40.], [40, 85, 220, 160]);
    let stop = AtomicBool::new(false);
    let runs = capture(
        &doc,
        &[a, c],
        0,
        1.,
        view(),
        renderer::MAX_SPRITE_EDGE,
        &stop,
    )
    .unwrap();
    assert_eq!(
        runs.iter().map(|r| r.0).collect::<Vec<_>>(),
        [true, false, true]
    );
    for o in &mut doc.objects {
        if o.id == a || o.id == c {
            o.position[0] += 4.;
        }
    }
    let expected = renderer::render(&doc, 0).unwrap();
    // Composite the two selected runs at their translated pose through the
    // stationary middle run, including antialiased translucent overlap.
    let mut combined = renderer::Pixmap::new(expected.width(), expected.height()).unwrap();
    for (moving, sprite) in &runs {
        let x = sprite.origin[0] as i32 + if *moving { 4 } else { 0 };
        combined.draw_pixmap(
            x,
            sprite.origin[1] as i32,
            sprite.pixels.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
    }
    assert!(combined
        .data()
        .iter()
        .zip(expected.data())
        .all(|(x, y)| x.abs_diff(*y) <= 1));

    doc.objects[0].group = Some(10);
    doc.objects[1].group = Some(10);
    let mask = doc.add_rectangle([-10., 5.], [42., 44.], [255; 4]);
    doc.set_clip(&[a, b, mask], 0, Some([64, 160])).unwrap();
    assert!(capture(
        &doc,
        &[a, c],
        0,
        1.,
        view(),
        renderer::MAX_SPRITE_EDGE,
        &stop
    )
    .is_none());
    assert!(capture(
        &doc,
        &[a, b],
        0,
        1.,
        view(),
        renderer::MAX_SPRITE_EDGE,
        &stop
    )
    .is_some());
}

#[test]
fn only_a_shared_evaluated_pose_can_transform_all_selected_artwork() {
    let mut src = Document::default();
    let a = src.add_rectangle([-20., 3.], [12., 7.], [255; 4]);
    let b = src.add_rectangle([20., 13.], [32., 28.], [255; 4]);
    let mut dest = src.clone();
    let view = View {
        board: view(),
        scale: 1.,
    };
    let source = Arc::new(src);
    let r = Request {
        source,
        ids: vec![a, b],
        frame: 0,
        density: 1.,
        visible: view.board,
        representative: 1,
        members: vec![0, 1],
        sorted_ids: vec![a, b],
        scheduled: true,
        tried_coarse: false,
    };
    for o in &mut dest.objects {
        o.position[0] += 7.;
        o.position[1] -= 3.;
    }
    let affine = r.transform(&dest, view).unwrap();
    assert!(affine.map(Pos2::ZERO).distance(egui::pos2(7., -3.)) < 0.001);
    dest.objects[0].position[0] += 3.;
    assert!(r.transform(&dest, view).is_none());
}
