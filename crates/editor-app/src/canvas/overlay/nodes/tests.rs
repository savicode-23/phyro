use super::*;

// Compare the emitted triangle buffers against the same controls painted directly.
fn triangles(
    ctx: &egui::Context,
    overlay: &mut NodeOverlay,
    document: &Arc<Document>,
    cache: bool,
    offset: f32,
) -> Vec<(Rect, egui::Mesh)> {
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(300.))),
            ..Default::default()
        },
        |ctx| {
            let view = View {
                board: Rect::from_min_size(egui::pos2(offset, 0.), Vec2::splat(300.)),
                scale: 2.,
            };
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(Rect::from_min_max(
                    egui::pos2(12., 18.),
                    egui::pos2(255., 260.),
                ));
            overlay.paint(
                &painter,
                view,
                document,
                0,
                &[document.objects[0].id],
                cache.then_some(document),
                None,
            );
        },
    );
    ctx.tessellate(output.shapes, output.pixels_per_point)
        .into_iter()
        .map(|primitive| {
            let Primitive::Mesh(mesh) = primitive.primitive else {
                panic!("expected node mesh")
            };
            (primitive.clip_rect, mesh)
        })
        .collect()
}

#[test]
fn marquee_selected_anchor_is_visible_over_a_later_handles_circle() {
    let mut doc = Document::default();
    let mut nodes = [[0.1, 0.1], [0.3, 0.5], [0.6, 0.5], [0.8, 0.8]].map(PathNode::corner);
    nodes[3].incoming = nodes[1].point;
    doc.add_shape(
        [0.; 2],
        [100.; 2],
        [255; 4],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        1,
    );
    let rect = Rect::from_min_max(egui::pos2(24., 44.), egui::pos2(64., 56.));
    let selection = NodeSet::in_box(&doc.objects[0], 0, rect, None);
    assert_eq!(selection.runs(), [(0, 1, 2)]);
    let view = View {
        board: Rect::from_min_size(Pos2::ZERO, Vec2::splat(128.)),
        scale: 1.,
    };
    let marks = marks(&doc.objects[0], 0, view, &selection, Rect::EVERYTHING);
    for node in &nodes[1..3] {
        let at = Pos2::from(doc.objects[0].local_to_world(node.point, 0));
        let last = marks.iter().rfind(|mark| match mark {
            egui::Shape::Circle(circle) => circle.center == at,
            egui::Shape::Rect(rect) => rect.rect.center() == at,
            _ => false,
        });
        assert!(matches!(last, Some(egui::Shape::Circle(ring)) if ring.radius == 5.));
    }
}

#[test]
fn cached_nodes_match_direct_triangles_after_geometry_range_and_view_changes() {
    let mut doc = Document::default();
    let nodes = (0..512)
        .map(|i| PathNode::corner([(i % 32) as f32 / 31., (i / 32) as f32 / 15.]))
        .collect();
    let id = doc.add_shape(
        [4., 8.],
        [116., 112.],
        [255; 4],
        Shape::Path {
            nodes,
            closed: false,
        },
        1,
    );
    doc.objects[0].rotation = 13.;
    let mut doc = Arc::new(doc);
    let ctx = egui::Context::default();
    let mut cache = NodeOverlay::default();
    let mut last = Vec::new();
    for step in 0..4 {
        if step == 1 {
            Arc::make_mut(&mut doc).objects[0].shape.contours_mut()[0][256].incoming = [1.4, -0.4];
        }
        if step == 2 {
            crate::path_editor::select(&ctx, id, 0, 0, 300);
        }
        let offset = if step == 3 { 15.25 } else { 0. };
        let expected = triangles(&ctx, &mut NodeOverlay::default(), &doc, false, offset);
        assert_ne!(last, expected, "fixture must visibly change");
        for _ in 0..3 {
            assert_eq!(expected, triangles(&ctx, &mut cache, &doc, true, offset));
        }
        assert!(matches!(
            &cache.cached.as_ref().unwrap().state,
            MeshState::Ready(_)
        ));
        last = expected;
    }
}
