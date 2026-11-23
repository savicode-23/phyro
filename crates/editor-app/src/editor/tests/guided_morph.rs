use super::*;
use document_core::{PathNode, Shape};

fn polygon(points: &[[f32; 2]]) -> Shape {
    Shape::Path {
        nodes: points.iter().copied().map(PathNode::corner).collect(),
        closed: true,
    }
}

#[test]
fn selecting_two_shapes_in_click_order_previews_then_commits_a_single_undoable_morph() {
    let mut doc = Document {
        frames: 1,
        fps: 12,
        ..Default::default()
    };
    let target = doc.add_shape(
        [76., 44.],
        [30., 35.],
        [92, 48, 117, 255],
        Shape::Ellipse,
        1,
    );
    let source = doc.add_rectangle([20., 30.], [38., 22.], [32, 143, 119, 255]);
    let original = AnimationProject::from_document(doc);
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor.replace_project(original.clone(), None);
    w.editor.set_tool(Tool::Select);
    let board = w.board();
    w.click_at(board.min + board.size() * (egui::vec2(39., 41.) / 128.));
    super::path_ops::shift_click(
        &mut w,
        board.min + board.size() * (egui::vec2(91., 61.) / 128.),
    );
    assert_eq!(w.editor.selection, [source, target]);
    w.click("Morph");
    w.control("Match points");
    w.control("Pause");
    let frame_number = |out: egui::FullOutput| {
        out.shapes
            .iter()
            .rev()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) => t.galley.text().strip_prefix("Frame ")?.parse::<u32>().ok(),
                _ => None,
            })
            .expect("morph playhead")
    };
    let first = frame_number(w.frame(vec![]));
    let time = w.ctx.input(|i| i.time);
    let later = frame_number(w.frame_with_input(egui::RawInput {
        time: Some(time + 0.26),
        ..Default::default()
    }));
    assert_ne!(first, later, "playback starts as soon as Morph opens");
    w.click("Pause");
    let paused = frame_number(w.frame(vec![]));
    assert_eq!(
        paused,
        frame_number(w.frame_with_input(egui::RawInput {
            time: Some(time + 0.85),
            ..Default::default()
        }))
    );
    w.click("Cancel");
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
    w.click("Morph");
    w.click("Done");

    let clip = &w.editor.project.clips[0];
    assert_eq!(clip.frames, 13);
    assert_eq!(clip.cels.len(), 1);
    let animated = &clip.drawing_at(0).objects[1];
    assert_eq!(animated.id, source);
    assert_eq!(document_core::Property::Geometry.frames(animated), [0, 12]);
    assert_eq!(animated.morph_guides(0, 12), Some([].as_slice()));
    assert_eq!(animated.fill, [32, 143, 119, 255]);
    let donor = &clip.drawing_at(12).objects[0];
    assert_eq!(donor.shape, Shape::Ellipse);
    assert!(!donor.visible_at(12));
    let reference = renderer::render(clip.drawing_at(0), 0).unwrap();
    let halfway = renderer::render(clip.drawing_at(6), 6).unwrap();
    let finish = renderer::render(clip.drawing_at(12), 12).unwrap();
    assert_ne!(reference.data(), halfway.data());
    assert_ne!(finish.data(), halfway.data());
    assert_eq!(finish.pixel(91, 61).unwrap().alpha(), 255);
    assert_eq!(reference.pixel(91, 61).unwrap().alpha(), 0);
    assert_eq!(w.editor.selection, [source]);

    let next = w.editor.project.clone();
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("shape-morph.vanim");
    project_io::save_animation(&native, &next, false).unwrap();
    assert_eq!(project_io::load_animation(&native).unwrap(), next);
    w.editor.restore(false);
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
    w.editor.restore(true);
    assert_eq!(w.editor.project, next);
}

fn morph_window(w: &EditorWindow) -> egui::Rect {
    w.ctx
        .memory(|m| m.area_rect(egui::Id::new("shape-morph-window")))
        .expect("floating Shape morph window")
}

#[test]
fn morph_floats_over_the_drawing_and_timeline_at_desktop_and_minimum_sizes() {
    for size in [egui::vec2(1100., 740.), egui::vec2(800., 500.)] {
        let mut w = EditorWindow::with_artboard(size, [160, 120]);
        w.ctx.style_mut(|s| s.animation_time = 0.);
        let doc = w.editor.project.clips[0].drawing_at_mut(0);
        let first = doc.add_rectangle([16., 12.], [35., 28.], [128; 4]);
        let last = doc.add_rectangle([80., 50.], [48., 40.], [128; 4]);
        w.editor.set_tool(Tool::Select);
        w.editor.selection = vec![first, last];
        w.editor.capture_baseline();
        let initial = w.editor.project.clone();
        let board = w.board();
        let next = w.timeline_icon("Next");
        w.click("Morph");
        w.control("Pause");
        let window = morph_window(&w);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        assert!(screen.contains_rect(window), "{window:?} at {size:?}");
        assert!(
            window.area() < screen.area() * 0.6,
            "{window:?} at {size:?}"
        );
        let output = w.frame(vec![]);
        assert_eq!(EditorWindow::board_rect(&output), board);
        assert!(
            board.intersect(window).area() < board.area(),
            "artwork is covered at {size:?}"
        );
        assert!(
            !window.contains(w.control("Blank frame")),
            "timeline is covered at {size:?}"
        );
        assert!(!window.contains(next));
        w.click_at(next);
        let canvas_point = board.left_center() + egui::vec2(3., 0.);
        assert!(!window.contains(canvas_point));
        w.frame_with_input(egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(canvas_point),
                egui::Event::Zoom(2.),
            ],
            dropped_files: vec![egui::DroppedFile {
                path: Some(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../examples/guided-flame.vanim"),
                ),
                ..Default::default()
            }],
            ..Default::default()
        });
        for pressed in [true, false] {
            w.frame(vec![egui::Event::Key {
                key: egui::Key::Period,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        assert_eq!(w.editor.timeline.frame, 0);
        assert_eq!(w.editor.selection, [first, last]);
        assert_eq!(w.editor.project, initial);
        w.click("Cancel");
        assert_eq!(w.board(), board);
        w.click("Morph");

        let title = w.control("Shape morph");
        let left = if size.x > 800. { -350. } else { -40. };
        w.drag(title, egui::vec2(left, 25.));
        let moved = morph_window(&w);
        assert!(
            moved.min.distance(window.min) > 12.,
            "title drag at {size:?}"
        );
        if size.x > 800. {
            w.drag(
                moved.right_bottom() - egui::vec2(3., 3.),
                egui::vec2(310., 120.),
            );
            let enlarged = morph_window(&w);
            assert!(
                enlarged.width() > 800.,
                "corner drag resized to {enlarged:?}"
            );
            w.click("Cancel");
            w.click("Morph");
            w.control("Pause");
            assert_eq!(morph_window(&w), enlarged);
            w.size = egui::vec2(800., 500.);
            w.frame(vec![]);
            w.frame(vec![]);
            let small = egui::Rect::from_min_size(egui::Pos2::ZERO, w.size);
            assert!(small.contains_rect(morph_window(&w)));
        }
        let title = w.control("Shape morph");
        let window = morph_window(&w);
        let close = egui::pos2(window.right() - (title.y - window.top()), title.y);
        w.click_at(close);
        assert!(
            w.editor.guided_morph.is_none(),
            "title-bar close at {:?}",
            w.size
        );
        assert_eq!(w.editor.project, initial);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn match_two_outline_points_apply_reopen_remove_and_undo() {
    for size in [egui::vec2(1100., 740.), egui::vec2(800., 500.)] {
        match_points_at_size(size);
    }
}

fn match_points_at_size(size: egui::Vec2) {
    let mut w = EditorWindow::with_artboard(size, [200, 150]);
    w.ctx.style_mut(|style| style.animation_time = 0.);
    let clip = &mut w.editor.project.clips[0];
    clip.set_length(12).unwrap();
    let doc = clip.drawing_at_mut(0);
    let source = doc.add_shape(
        [8., 5.],
        [60., 72.],
        [200; 4],
        polygon(&[[0., 0.], [1., 0.], [0.4, 1.]]),
        1,
    );
    doc.add_shape(
        [70., 40.],
        [64., 35.],
        [100; 4],
        polygon(&[[0., 0.], [1., 0.], [1., 1.], [0., 1.]]),
        1,
    );
    doc.objects[1].rotation = 24.;
    doc.objects[1].shape.contours_mut()[0][0].outgoing = [0.2, -0.3];
    doc.objects[1]
        .effects
        .push(document_core::PathEffect::Mirror { horizontal: true });
    doc.objects[1].name = "End drawing".into();
    let original = doc.clone();
    let end_id = original.objects[1].id;
    w.editor.timeline.frame = 0;
    w.editor.selection = vec![source, end_id];
    w.editor.set_tool(Tool::Select);
    w.editor.capture_baseline();
    w.frame(vec![]);
    w.click("Morph");
    w.frame(vec![]);
    w.click("Match points");
    // Pick the middle of a rendered cubic on each outline, away from its anchors.
    let heading = w.control("Start · Frame 1");
    let help = w.control("Click a point on the start pose to match it.");
    let window = morph_window(&w);
    let curves: Vec<_> = w
        .frame(vec![])
        .shapes
        .into_iter()
        .filter_map(|s| {
            let egui::Shape::CubicBezier(curve) = s.shape else {
                return None;
            };
            let [a, b, c, d] = curve.points;
            let p = a
                .lerp(b, 0.5)
                .lerp(b.lerp(c, 0.5), 0.5)
                .lerp(b.lerp(c, 0.5).lerp(c.lerp(d, 0.5), 0.5), 0.5);
            (window.contains_rect(s.clip_rect) && heading.y < p.y && p.y < help.y).then_some(p)
        })
        .collect();
    let middle = window.center().x;
    w.click_at(
        *curves
            .iter()
            .find(|p| p.x < middle)
            .expect("starting outline"),
    );
    w.control("Cancel point");
    w.click_at(
        *curves
            .iter()
            .find(|p| p.x > middle)
            .expect("ending outline"),
    );
    w.control("1 ×");
    w.click("Done");
    assert!(w.editor.guided_morph.is_none());
    let doc = w.editor.scene();
    let mut donor = doc.objects[1].clone();
    assert!(!donor.visible_at(0));
    donor.animated.visible = original.objects[1].animated.visible;
    assert_eq!(donor, original.objects[1]);
    let obj = &doc.objects[0];
    let end = 11;
    let guide = obj.morph_guides(0, end).unwrap()[0];
    assert!(guide.from.t > 0.05 && guide.from.t < 0.95);
    assert!(guide.to.t > 0.05 && guide.to.t < 0.95);
    let pose = obj.shape_at(end);
    for (posed, donor) in pose.contours()[0]
        .iter()
        .zip(original.objects[1].evaluated_shape_at(end).contours()[0].iter())
    {
        for (a, b) in [
            (posed.point, donor.point),
            (posed.incoming, donor.incoming),
            (posed.outgoing, donor.outgoing),
        ] {
            let expected = original.objects[1].local_to_world(b, end);
            let actual = obj.local_to_world(a, end);
            assert!(egui::Pos2::from(expected).distance(actual.into()) < 0.001);
        }
    }
    assert_ne!(obj.shape_at(4), obj.shape_at(0));
    w.editor.timeline.frame = 4;
    w.click("Edit morph…");
    w.click("Match points");
    w.click("1 ×");
    w.click("Cancel");
    assert_eq!(
        w.editor.scene().objects[0].morph_guides(0, end).unwrap(),
        &[guide]
    );
    w.click("Edit morph…");
    w.click("Match points");
    w.click("1 ×");
    w.click("Done");
    assert_eq!(
        w.editor.scene().objects[0].morph_guides(0, end).unwrap(),
        &[]
    );
    w.editor.restore(false);
    assert_eq!(
        w.editor.scene().objects[0].morph_guides(0, end).unwrap(),
        &[guide]
    );
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &original);
    w.editor.restore(true);
    assert_eq!(
        w.editor.scene().objects[0].morph_guides(0, end).unwrap(),
        &[guide]
    );
}
