use super::*;

fn panel(window: &EditorWindow, id: &str) -> egui::Rect {
    egui::containers::panel::PanelState::load(&window.ctx, egui::Id::new(id))
        .unwrap()
        .rect
}

fn window_with_tracks(size: egui::Vec2) -> (EditorWindow, u64) {
    let mut doc = Document::default();
    let id = doc.add_rectangle([20.; 2], [35.; 2], [135, 180, 235, 255]);
    for property in document_core::Property::ALL {
        property.add(&mut doc.objects[0], 2);
    }
    let mut window = EditorWindow::new(size);
    window.ctx.style_mut(|s| s.animation_time = 0.);
    window
        .editor
        .replace_project(AnimationProject::from_document(doc), None);
    window.editor.set_tool(Tool::Select);
    (window, id)
}

fn select_and_clear(window: &mut EditorWindow, id: u64) {
    let board = window.board();
    let timeline = panel(window, "timeline");
    let inspector = panel(window, "inspector");
    let center = board.min + board.size() * 0.30;
    let blank = board.min + board.size() * 0.72;
    for (point, expected) in [(center, Some(id)), (blank, None)] {
        window.click_at(point);
        assert_eq!(window.editor.selection.as_slice(), expected.as_slice());
        if expected.is_some() {
            assert!(timeline.contains(window.control("Position")));
        }
        assert_eq!(window.board(), board);
        assert_eq!(panel(window, "timeline"), timeline);
        assert_eq!(panel(window, "inspector"), inspector);
    }
}

#[test]
fn selection_keeps_the_canvas_and_both_user_resized_docks_in_place() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let (mut w, id) = window_with_tracks(size);
        select_and_clear(&mut w, id);
        let initial = panel(&w, "timeline");
        w.drag(initial.center_top(), egui::vec2(0., -48.));
        let taller = panel(&w, "timeline");
        assert!((taller.height() - initial.height() - 48.).abs() < 1.);
        let side = panel(&w, "inspector");
        w.drag(side.left_center(), egui::vec2(44., 0.));
        let narrower = panel(&w, "inspector");
        assert!((side.width() - narrower.width() - 44.).abs() < 1.);
        select_and_clear(&mut w, id);
    }
}

#[test]
fn view_menu_reclaims_canvas_space_and_remembers_the_chosen_layout() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let (mut w, id) = window_with_tracks(size);
        let initial = w.board();
        for label in ["Inspector", "Timeline"] {
            w.click("View");
            w.click(label);
        }
        let expanded = w.board();
        assert!(expanded.height() > initial.height());
        w.click("Pen");
        assert!(w.editor.tool == Tool::Pen);
        w.click("Select");
        w.click_at(expanded.min + expanded.size() * 0.30);
        assert_eq!(w.editor.selection, [id]);
        assert_eq!(w.board(), expanded);
        assert!(!w.editor.show_inspector && !w.editor.show_timeline);
        for label in ["Inspector", "Timeline"] {
            w.click("View");
            w.click(label);
        }
        assert_eq!(w.board(), initial);
        assert_eq!(w.editor.selection, [id]);
    }
}

#[test]
fn document_properties_configures_the_axonometric_grid_and_pen_snapping() {
    use document_core::{GridKind, GridSettings, GridVisibility, Shape};

    let mut w = EditorWindow::with_artboard(egui::vec2(1200., 960.), [128; 2]);
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.frame(vec![]);
    assert!(!w.editor.visible_panels[Tab::Document as usize]);
    w.click("View");
    w.click("Panels");
    w.click("Document Properties");
    assert_eq!(w.editor.inspector_tab, Tab::Document);
    w.click("Rectangular");
    w.click("Axonometric");
    let grid = w.editor.project.grid;
    assert_eq!(grid.kind, GridKind::Axonometric);
    assert_eq!(grid.visibility, GridVisibility::Visible);
    assert_eq!((grid.spacing, grid.angle_x, grid.angle_z), (32., 30., 30.));
    w.field_on_row("Y spacing", "px");
    w.field_on_row("X angle", "°");
    w.field_on_row("Z angle", "°");
    let origin = w.field_on_row("Origin Y", "px");
    w.drag(origin, egui::vec2(14., 0.));
    assert!(w.editor.project.grid.origin[1] > 0.);
    w.editor.restore(false);
    assert_eq!(w.editor.project.grid, grid);
    w.editor.restore(false);
    assert_eq!(w.editor.project.grid, GridSettings::default());
    w.editor.restore(true);
    assert_eq!(w.editor.project.grid, grid);

    assert!(!w.editor.style.snap.enabled);
    let output = w.frame(vec![]);
    let board = EditorWindow::board_rect(&output);
    let slopes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == egui::Color32::from_black_alpha(22) =>
            {
                assert!(points.iter().all(|&p| board.expand(0.01).contains(p)));
                let delta = points[1] - points[0];
                Some(delta.y.atan2(delta.x).to_degrees())
            }
            _ => None,
        })
        .collect();
    for angle in [-30., 30., 90.] {
        assert!(
            slopes.iter().any(|value| (value - angle).abs() < 0.01),
            "{slopes:?}"
        );
    }

    w.click("View");
    w.click("Panels");
    w.click("Document Properties");
    assert!(!w.editor.visible_panels[Tab::Document as usize]);
    assert_eq!(w.editor.project.grid, grid);
    super::pen::chord(&mut w, Key::Escape, egui::Modifiers::NONE);
    w.click("Snap");
    w.click("Pen");
    let board = w.board();
    let scale = board.width() / 128.;
    let width = 16. * 3_f32.sqrt();
    let vertices = [[width, 48.], [2. * width, 64.]];
    for [x, y] in vertices {
        w.click_at(board.min + egui::vec2(x + 1.5, y - 2.) * scale);
    }
    super::pen::chord(&mut w, Key::Enter, egui::Modifiers::NONE);
    let object = &w.editor.scene().objects[0];
    let Shape::Path { nodes, .. } = &object.shape else {
        panic!("expected a Pen path")
    };
    assert_eq!(nodes.len(), vertices.len());
    for (node, vertex) in nodes.iter().zip(vertices) {
        let actual = egui::Pos2::from(object.local_to_world(node.point, 0));
        assert!(
            actual.distance(vertex.into()) < 0.001,
            "{actual:?} vs {vertex:?}"
        );
    }
}

#[test]
fn stacked_layers_and_properties_resize_move_and_restore_from_the_panel_menu() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let (mut w, id) = window_with_tracks(size);
        w.editor.selection = vec![id];
        w.frame(vec![]);
        let initial = w.board();
        w.click("View");
        w.click("Panels");
        w.click("Layers and Properties together");
        w.frame(vec![]);
        let properties = w.control("Properties");
        let layers = w.control("Layers");
        assert!(layers.y > properties.y + 35.);
        assert!(w.control("Layers and objects").y > layers.y);
        assert_eq!(w.board(), initial);
        let dock = panel(&w, "inspector");
        let split = egui::pos2(dock.center().x, dock.center().y);
        w.drag(split, egui::vec2(0., -26.));
        assert!(w.control("Layers").y < layers.y - 15.);
        let layers = w.control("Layers");
        // Drag to the upper-left edge of the canvas, including an initially empty dock.
        let left_target = egui::pos2(panel(&w, "drawing-tools").right() + 50., dock.top() + 20.);
        w.drag(layers, left_target - layers);
        w.frame(vec![]);
        let left = panel(&w, "inspector-left");
        assert!(left.contains(w.control("Layers and objects")));
        let layers = w.control("Layers");
        super::pen::right_click(&mut w, layers);
        w.click("Lower right");
        w.frame(vec![]);
        assert!(panel(&w, "inspector").contains(w.control("Layers and objects")));
        assert!(panel(&w, "inspector").contains(w.control("Properties")));
        w.click("View");
        w.click("Panels");
        w.click("Reset panel layout");
        w.frame(vec![]);
        assert_eq!(w.board(), initial);
        assert_eq!(w.editor.selection, [id]);
        let properties = w.control("Properties");
        let layers = w.control("Layers");
        assert!(layers.y > properties.y + 35.);
        if size.x > 1000. {
            let beside_properties = properties - egui::vec2(3., 0.);
            w.drag(layers, beside_properties - layers);
            assert!((w.control("Layers").y - w.control("Properties").y).abs() < 1.);
            w.control("Layers and objects");
        }
        w.click("View");
        w.click("Panels");
        w.click("Group panels into tabs");
        w.frame(vec![]);
        assert!((w.control("Layers").y - w.control("Properties").y).abs() < 1.);
    }
}

fn wheel(window: &mut EditorWindow, pos: egui::Pos2, points: f32) {
    window.frame(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., points),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    for _ in 0..18 {
        window.frame(Vec::new());
    }
}

#[test]
fn scrolled_tracks_remain_clickable_and_timeline_controls_are_reachable() {
    let (mut w, id) = window_with_tracks(egui::vec2(800., 500.));
    let board = w.board();
    w.click_at(board.min + board.size() * 0.30);
    assert_eq!(w.editor.selection, [id]);
    let fourth_frame_x = w.control("4").x;
    let timeline = panel(&w, "timeline");
    let inside = timeline.left_center() + egui::vec2(32., 0.);
    wheel(&mut w, inside, -600.);
    let last = w.control("Path progress");
    assert!(timeline.contains(last));
    w.click_at(last);
    w.click_at(egui::pos2(fourth_frame_x, last.y));
    assert_eq!(w.editor.timeline.frame, 3);
    assert_eq!(w.board(), board);
    wheel(&mut w, inside, 600.);
    w.control("Track: Path progress");
    let next = w.timeline_icon("Next");
    w.click_at(next);
    assert_eq!(w.editor.timeline.frame, 4);
    assert_eq!(panel(&w, "timeline"), timeline);
}

#[test]
fn resizing_reuses_the_artwork_then_sharpens_to_the_final_canvas_size() {
    let (mut w, _) = window_with_tracks(egui::vec2(1100., 740.));
    let mut time = 0.;
    let mut next = |window: &mut EditorWindow| {
        time += 1. / 60.;
        window.frame_with_input(egui::RawInput {
            time: Some(time),
            ..Default::default()
        })
    };
    let first = next(&mut w);
    let id = *w
        .ctx
        .tex_manager()
        .read()
        .allocated()
        .find(|(_, t)| t.name == "artboard artwork")
        .unwrap()
        .0;
    let initial = EditorWindow::board_rect(&first);
    let mut board = initial;
    for _ in 0..12 {
        w.size.y -= 4.;
        let frame = next(&mut w);
        board = EditorWindow::board_rect(&frame);
        assert!(frame.textures_delta.set.iter().all(|(k, _)| *k != id));
    }
    assert!((initial.height() - board.height() - 48.).abs() < 1.);
    let doc = w.editor.scene();
    let scale = board.width() / doc.width as f32;
    let expected = renderer::render_scaled(doc, 0, scale).unwrap();
    let start = std::time::Instant::now();
    loop {
        let frame = next(&mut w);
        assert_eq!(EditorWindow::board_rect(&frame), board);
        if let Some((_, delta)) = frame.textures_delta.set.into_iter().find(|(k, _)| *k == id) {
            let egui::ImageData::Color(image) = delta.image else {
                panic!("expected artboard pixels")
            };
            assert_eq!(delta.pos, None);
            assert!(
                *image
                    == egui::ColorImage::from_rgba_premultiplied(
                        [expected.width() as usize, expected.height() as usize],
                        expected.data()
                    )
            );
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn floating_bounds(w: &mut EditorWindow, name: &str) -> egui::Rect {
    w.frame(Vec::new());
    w.ctx.memory(|m| m.area_rect(egui::Id::new(name))).unwrap()
}

fn reselect_around_window(w: &mut EditorWindow, id: u64, name: &str, expected: egui::Rect) {
    let board = w.board();
    let selected = board.min + board.size() * 0.30;
    let blank = board.min + board.size() * egui::vec2(0.35, 0.68);
    for (point, selection) in [(blank, None), (selected, Some(id))] {
        assert!(!expected.contains(point));
        w.click_at(point);
        assert_eq!(w.editor.selection.as_slice(), selection.as_slice());
        assert_eq!(floating_bounds(w, name), expected);
        assert_eq!(w.board(), board);
    }
}

#[test]
fn curve_keeps_its_floating_bounds_and_controls_after_reselection() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let name = "Animation curve";
        let (mut w, id) = window_with_tracks(size);
        let doc = w.editor.project.clips[0].drawing_at_mut(0);
        doc.objects[0].keys.insert(0, [20., 20.]);
        doc.objects[0].keys.insert(2, [55., 20.]);
        doc.add_rectangle([83., 15.], [12.; 2], [137; 4]);
        w.editor.selection = vec![id];
        w.frame(vec![]);
        w.click("Curve");
        let opened = floating_bounds(&mut w, name);
        w.drag(
            opened.center_top() + egui::vec2(0., 14.),
            egui::vec2(size.x - opened.right() - 6., 0.),
        );
        let full = floating_bounds(&mut w, name);
        assert_eq!(full.size(), opened.size());
        reselect_around_window(&mut w, id, name, full);
        w.drag(
            full.right_bottom() - egui::vec2(1., 1.),
            egui::vec2(-300., -160.),
        );
        let small = floating_bounds(&mut w, name);
        assert!(small.width() < full.width() - 60. && small.height() < full.height() - 55.);
        reselect_around_window(&mut w, id, name, small);
        wheel(&mut w, small.right_bottom() - egui::vec2(23., 16.), -600.);
        w.control("Copy easing");
        let output = w.frame(vec![]);
        let key = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(dot)
                    if dot.fill == egui::Color32::GOLD
                        && dot.radius == 4.5
                        && shape.clip_rect.contains(dot.center) =>
                {
                    Some(dot.center)
                }
                _ => None,
            })
            .expect("first curve key is reachable in the scrolled graph");
        w.drag(key, egui::vec2(51., 0.));
        let keys = &w.editor.scene().objects[0].keys;
        assert!(!keys.contains_key(&0) && keys.contains_key(&1));
        assert_eq!(floating_bounds(&mut w, name), small);
    }
}
