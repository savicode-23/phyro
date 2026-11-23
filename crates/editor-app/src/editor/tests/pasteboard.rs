use super::*;
use document_core::{PathNode, Shape};
use egui::{pos2, vec2, Pos2, Rect};

fn window(doc: &Document, tool: Tool) -> (EditorWindow, Rect) {
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.set_tool(tool);
    let board = w.board();
    (w, board)
}

fn screen(board: Rect, x: f32, y: f32) -> Pos2 {
    board.min + board.size() * vec2(x / 128., y / 128.)
}

#[test]
fn a_box_selection_moves_around_a_stationary_pasteboard_shape_in_paint_order() {
    let mut doc = Document::default();
    let back = doc.add_rectangle([-18., 25.], [13., 9.], [34, 180, 96, 255]);
    doc.add_rectangle([-14., 29.], [50., 9.], [170, 80, 20, 160]);
    let front = doc.add_rectangle([-4., 27.], [12., 12.], [30, 50, 200, 192]);
    let (mut w, board) = window(&doc, Tool::Select);
    let corner = screen(board, -22., 21.);
    w.drag(corner, screen(board, 13., 43.) - corner);
    assert_eq!(w.editor.selection, [back, front]);

    let start = screen(board, -10., 27.);
    let first = screen(board, 2., 42.);
    let second = screen(board, 9., 45.);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    w.frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
    let preview = |w: &mut EditorWindow, p| {
        let out = w.frame(vec![egui::Event::PointerMoved(p)]);
        let parts = w.ctx.tessellate(out.shapes, out.pixels_per_point);
        let manager = w.ctx.tex_manager();
        let textures = manager.read();
        parts
            .into_iter()
            .filter_map(|part| {
                let egui::epaint::Primitive::Mesh(mesh) = part.primitive else {
                    return None;
                };
                (textures.meta(mesh.texture_id)?.name == "selection transform")
                    .then(|| (mesh.texture_id, mesh.calc_bounds()))
            })
            .collect::<Vec<_>>()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let (initial, next) = loop {
        let a = preview(&mut w, first);
        let b = preview(&mut w, second);
        // Replacing an interim capture can change its sampling margin. Compare
        // geometry on the same textures across both pointer positions.
        if a.len() == 3 && a.iter().map(|(id, _)| id).eq(b.iter().map(|(id, _)| id)) {
            break (a, b);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "selection preview did not arrive"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    for (i, ((_, a), (_, b))) in initial.iter().zip(&next).enumerate() {
        let offset = if i == 1 {
            egui::Vec2::ZERO
        } else {
            second - first
        };
        assert!(b.min.distance(a.min + offset) < 0.1 && b.max.distance(a.max + offset) < 0.1);
    }
    assert_eq!(w.editor.scene(), &doc);
    w.frame(vec![button(second, false)]);
    let after = &w.editor.scene().objects;
    assert_eq!(after[1], doc.objects[1]);
    for slot in [0, 2] {
        let expected = Pos2::from(doc.objects[slot].position) + vec2(19., 18.);
        assert!(Pos2::from(after[slot].position).distance(expected) < 0.01);
    }
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn select_moves_parked_art_back_and_picks_a_crossing_shape_beyond_the_page() {
    let mut doc = Document::default();
    let parked = doc.add_rectangle([-34., 32.], [18., 16.], [200; 4]);
    let crossing = doc.add_rectangle([119., 60.], [27., 22.], [240; 4]);
    let locked = commands::add_layer(&mut doc);
    doc.add_shape([-39., 29.], [26., 24.], [100; 4], Shape::Rectangle, locked);
    doc.layers.last_mut().unwrap().locked = true;
    let (mut w, board) = window(&doc, Tool::Select);
    let start = screen(board, -29., 37.);
    let end = screen(board, 27., 41.);
    assert!(!board.contains(start));
    w.click_at(start);
    assert_eq!(w.editor.selection, [parked]);
    w.drag(start, end - start);
    let moved = w.editor.scene();
    assert!(Pos2::from(moved.objects[0].position).distance(pos2(22., 36.)) < 0.001);
    assert_eq!(moved.objects[2], doc.objects[2]);
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
    let crossing_point = screen(board, 138., 71.);
    assert!(!board.contains(crossing_point));
    w.click_at(crossing_point);
    assert_eq!(w.editor.selection, [crossing]);
    w.click_at(screen(board, -37., 35.)); // Only the locked rectangle covers the pointer.
    assert!(w.editor.selection.is_empty());
    assert!(!w.editor.can_undo());
}

#[test]
fn a_snapped_drag_tracks_the_selection_across_the_page_edge_and_back() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([40., 46.], [20., 16.], [190, 110, 62, 255]);
    let parked = doc.objects[0].clone();
    for i in 0..520 {
        let mut object = parked.clone();
        object.id = doc.next_id;
        doc.next_id += 1;
        object.position = [400. + i as f32, 300.];
        doc.objects.push(object);
    }
    let (mut w, board) = window(&doc, Tool::Select);
    w.editor.style.snap.enabled = true;
    let start = screen(board, 45., 50.);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    w.frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
    assert_eq!(w.editor.selection, [id]);

    // Begin entirely on the page, cross onto the pasteboard, then come back
    // without releasing. The visible selection must follow every transition.
    for x in [36., -22., 68.] {
        let pointer = screen(board, x + 5., 50.);
        let out = w.frame(vec![egui::Event::PointerMoved(pointer)]);
        let corner = screen(board, x, 46.);
        assert!(
            out.shapes.iter().any(|s| matches!(&s.shape,
                egui::Shape::Rect(r) if r.rect.size() == vec2(7., 7.)
                    && r.stroke.color == egui::Color32::from_rgb(38, 137, 236)
                    && r.rect.center().distance(corner) < 2.
            )),
            "missing selection corner at {corner:?}"
        );
        assert_eq!(w.editor.scene(), &doc);
    }
    w.frame(vec![button(screen(board, 73., 50.), false)]);
    assert_eq!(w.editor.scene().objects[0].position, [68., 46.]);
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn nodes_select_and_split_a_thin_edge_outside_the_page_and_undo() {
    let mut doc = Document::default();
    let id = doc.add_shape(
        [-32., 47.],
        [42., 6.],
        [0; 4],
        Shape::Path {
            nodes: vec![PathNode::corner([0., 0.5]), PathNode::corner([1., 0.5])],
            closed: false,
        },
        1,
    );
    doc.objects[0].stroke = [0, 30, 220, 255];
    doc.objects[0].stroke_width = 0.4;
    let (mut w, board) = window(&doc, Tool::Node);
    let exact = screen(board, -17., 50.);
    assert!(!board.contains(exact));
    w.click_at(exact);
    assert_eq!(w.editor.selection, [id]);
    w.click_at(screen(board, -35., 72.));
    assert!(w.editor.selection.is_empty());
    let near = exact + vec2(0., 3.);
    assert_eq!(
        doc.hit_test([-17., 50. + 3. * 128. / board.height()], 0),
        None
    );
    w.click_at(near);
    w.click_at(near);
    assert_eq!(w.editor.selection, [id]);
    let path = &w.editor.scene().objects[0];
    assert_eq!(path.shape.contours()[0].len(), 3);
    assert!(
        Pos2::from(path.local_to_world(path.shape.contours()[0][1].point, 0))
            .distance(pos2(-17., 50.))
            < 0.01
    );
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &doc);
    w.editor.restore(true);
    assert_eq!(w.editor.scene().objects[0].shape.contours()[0].len(), 3);
}

#[test]
fn draw_shapes_and_snap_open_and_closed_pen_paths_on_the_pasteboard() {
    for tool in [Tool::Rectangle, Tool::Ellipse] {
        let doc = Document::default();
        let (mut w, board) = window(&doc, tool);
        let start = screen(board, -36., 32.);
        let end = screen(board, -16., 52.);
        w.drag(start, end - start);
        let shape = &w.editor.scene().objects[0];
        assert!(Pos2::from(shape.position).distance(pos2(-36., 32.)) < 0.001);
        assert!(Pos2::from(shape.size).distance(pos2(20., 20.)) < 0.001);
        assert_eq!(
            shape.shape,
            if tool == Tool::Rectangle {
                Shape::Rectangle
            } else {
                Shape::Ellipse
            }
        );
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
    }
    for closed in [false, true] {
        let doc = Document::default();
        let (mut w, board) = window(&doc, Tool::Pen);
        w.editor.style.snap.enabled = true;
        w.editor.project.grid.spacing = 8.;
        for (x, y) in [(-32.2, 39.8), (-16.3, 55.8), (-24.2, 72.3)] {
            w.click_at(screen(board, x, y));
        }
        if closed {
            w.click_at(screen(board, -31.4, 39.7));
        } else {
            w.frame(vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        let path = &w.editor.scene().objects[0];
        let Shape::Path {
            nodes,
            closed: actual,
        } = &path.shape
        else {
            panic!("expected Pen artwork")
        };
        assert_eq!(*actual, closed);
        let world: Vec<_> = nodes
            .iter()
            .map(|n| path.local_to_world(n.point, 0))
            .collect();
        assert_eq!(world, [[-32., 40.], [-16., 56.], [-24., 72.]]);
        w.editor.restore(false);
        assert_eq!(w.editor.scene(), &doc);
    }
}

#[test]
fn menu_over_the_pasteboard_and_clicks_in_the_inspector_do_not_start_pen_anchors() {
    let doc = Document {
        width: 256,
        height: 64,
        ..Default::default()
    };
    let (mut w, board) = window(&doc, Tool::Pen);
    w.click("Path");
    let popup = w.control("Exclusion");
    assert!(
        popup.y > 55. && !board.contains(popup),
        "popup={popup:?}, artboard={board:?}"
    );
    w.click_at(popup);
    assert!(!w.editor.canvas.pen_in_progress());
    w.click_at(pos2(1040., 230.));
    assert!(!w.editor.canvas.pen_in_progress());
    assert!(w.editor.scene().objects.is_empty());
}
