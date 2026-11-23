use super::*;

fn board(session: &mut CanvasSession) -> Rect {
    let (_, frame) = session.paint(egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
        ..Default::default()
    });
    frame
        .shapes
        .iter()
        .find_map(|s| painted_artboard(&s.shape))
        .unwrap()
}

#[test]
fn shape_creation_honors_center_and_proportion_modifiers() {
    for tool in [Tool::Rectangle, Tool::Ellipse] {
        for (mods, position, size) in [
            (egui::Modifiers::CTRL, [44., 53.], [40., 22.]),
            (egui::Modifiers::SHIFT, [64., 64.], [15.5, 15.5]),
            (
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                [48.5, 48.5],
                [31., 31.],
            ),
        ] {
            let mut session = CanvasSession::new(tool);
            let board = board(&mut session);
            let scale = board.width() / 128.;
            session.modifiers = mods;
            assert!(session.drag(
                board.center(),
                board.center() + egui::vec2(20., 11.) * scale
            ));
            let object = &session.doc.objects[0];
            assert!(Pos2::from(object.position).distance(position.into()) < 0.001);
            assert!(Pos2::from(object.size).distance(size.into()) < 0.001);
        }
    }
}

#[test]
fn ctrl_pen_snaps_to_angled_and_cardinal_directions_even_with_grid_snap() {
    let mut session = CanvasSession::new(Tool::Pen);
    let board = board(&mut session);
    let scale = board.width() / 128.;
    let at = |point: Pos2| board.min + point.to_vec2() * scale;
    let mut anchor = egui::pos2(40., 22.);
    assert!(!session.click(at(anchor)));
    session.snap.enabled = true;
    session.modifiers = egui::Modifiers::CTRL;
    for (delta, degrees) in [
        (egui::vec2(32., 9.), 15_f32),
        (egui::vec2(-23., 37.), 120.),
        (egui::vec2(3., 32.), 90.),
    ] {
        assert!(!session.click(at(anchor + delta)));
        let node = *session.canvas.pen.last().unwrap();
        let next = Pos2::from(node.point);
        let actual = next - anchor;
        assert!((actual.angle().to_degrees() - degrees).abs() < 0.001);
        let direction = Vec2::angled(degrees.to_radians());
        assert!((actual.length() - delta.dot(direction)).abs() < 0.001);
        assert_eq!(node, PathNode::corner(node.point));
        anchor = next;
    }
    assert_eq!(session.canvas.pen.len(), 4);
}

#[test]
fn pen_click_jitter_stays_a_corner_but_a_started_drag_can_make_a_short_handle() {
    let mut session = CanvasSession::new(Tool::Pen);
    let board = board(&mut session);
    let first = board.center();
    let second = first + egui::vec2(40., 12.);
    let jitter = egui::vec2(4., 2.);
    for (at, drag) in [(first, false), (second, true)] {
        session.input(egui::Event::PointerMoved(at));
        assert!(!session.button(at, true));
        if drag {
            session.input(egui::Event::PointerMoved(at + egui::vec2(10., 0.)));
        }
        session.input(egui::Event::PointerMoved(at + jitter));
        assert!(!session.button(at + jitter, false));
    }
    let [corner, curve] = &session.canvas.pen[..] else {
        panic!("expected two nodes")
    };
    assert_eq!(*corner, PathNode::corner(corner.point));
    let handle = (Pos2::from(curve.outgoing) - Pos2::from(curve.point)) * (board.width() / 128.);
    assert!((handle - jitter).length() < 0.001);
}

#[test]
fn shift_click_adds_disjoint_nodes_and_off_center_grabs_keep_the_offset() {
    let mut session = CanvasSession::new(Tool::Node);
    let id = session.doc.add_shape(
        [20.; 2],
        [80.; 2],
        [255; 4],
        Shape::Path {
            nodes: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                .map(PathNode::corner)
                .to_vec(),
            closed: true,
        },
        1,
    );
    session.selected = vec![id];
    let board = board(&mut session);
    let scale = board.width() / 128.;
    let at = |x, y| board.min + egui::vec2(x, y) * scale;
    assert!(!session.click(at(20., 20.)));
    session.modifiers = egui::Modifiers::SHIFT;
    assert!(!session.click(at(100., 100.)));
    assert_eq!(
        crate::path_editor::selected_nodes(&session.ctx, id, 0).runs(),
        [(0, 0, 0), (0, 2, 2)]
    );
    session.modifiers = egui::Modifiers::NONE;
    let outside = at(20., 20.) - egui::vec2(8., 0.);
    assert!(session.drag(outside, outside + egui::vec2(12., 8.) * scale));
    let nodes = &session.doc.objects[0].shape.contours()[0];
    for (index, point) in [[0.15, 0.1], [1., 0.], [1.15, 1.1], [0., 1.]]
        .into_iter()
        .enumerate()
    {
        assert!(Pos2::from(nodes[index].point).distance(point.into()) < 0.001);
    }
}
