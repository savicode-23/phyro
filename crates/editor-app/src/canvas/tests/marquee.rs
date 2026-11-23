use super::*;

fn painted_board(s: &mut CanvasSession) -> Rect {
    let (_, out) = s.paint(egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
        ..Default::default()
    });
    out.shapes
        .iter()
        .find_map(|s| painted_artboard(&s.shape))
        .expect("visible artboard")
}

#[test]
fn node_marquee_selects_disjoint_anchors_across_contours_and_moves_their_handles() {
    use crate::path_editor::{selected_nodes, styles};
    use document_core::path_edit::HandleStyle;
    let mut s = CanvasSession::new(Tool::Node);
    s.doc.width = 128;
    s.doc.height = 128;
    let curve = |points: [[f32; 2]; 4]| {
        points
            .map(|point| PathNode {
                point,
                incoming: [point[0] + 0.03, point[1]],
                outgoing: [point[0] - 0.03, point[1]],
                handle_style: HandleStyle::Symmetric,
            })
            .to_vec()
    };
    let id = s.doc.add_shape(
        [12., 16.],
        [100., 92.],
        [255; 4],
        Shape::Compound {
            contours: vec![
                curve([[0.1, 0.1], [0.9, 0.2], [0.1, 0.6], [0.9, 0.9]]),
                curve([[0.32, 0.36], [0.64, 0.36], [0.64, 0.54], [0.32, 0.54]]),
            ],
        },
        1,
    );
    s.selected.push(id);
    let board = painted_board(&mut s);
    let screen = |x, y| board.min + egui::vec2(x, y) * (board.width() / 128.);
    assert!(!s.drag(screen(54., 92.), screen(8., 12.)));
    assert_eq!(s.selected, [id]);
    assert_eq!(
        selected_nodes(&s.ctx, id, 0).runs(),
        [(0, 0, 0), (0, 2, 2), (1, 0, 0), (1, 3, 3)]
    );
    s.modifiers = egui::Modifiers::SHIFT;
    assert!(!s.drag(screen(92., 26.), screen(110., 44.)));
    s.modifiers = egui::Modifiers::NONE;
    let selected = selected_nodes(&s.ctx, id, 0);
    assert_eq!(selected.runs(), [(0, 0, 2), (1, 0, 0), (1, 3, 3)]);
    let before = s.doc.objects[0].shape.clone();
    let anchor = screen(22., 25.2);
    assert!(s.drag(anchor, anchor + egui::vec2(9., 12.)));
    let after = s.doc.objects[0].shape.clone();
    for (c, contour) in before.contours().iter().enumerate() {
        for (i, old) in contour.iter().enumerate() {
            let new = after.contours()[c][i];
            if selected.contains(c, i) {
                for (a, b) in [
                    (old.point, new.point),
                    (old.incoming, new.incoming),
                    (old.outgoing, new.outgoing),
                ] {
                    let delta = (Pos2::from(b) - Pos2::from(a))
                        * Vec2::new(100., 92.)
                        * (board.width() / 128.);
                    assert!((delta - egui::vec2(9., 12.)).length() < 0.001);
                }
            } else {
                assert_eq!(*old, new);
            }
        }
    }
    assert!(styles::apply(
        &s.ctx,
        &mut s.doc,
        &s.selected,
        0,
        s.timeline.key_mode,
        HandleStyle::Corner
    ));
    for (c, contour) in after.contours().iter().enumerate() {
        for (i, old) in contour.iter().enumerate() {
            let expected = if selected.contains(c, i) {
                PathNode::corner(old.point)
            } else {
                *old
            };
            assert_eq!(s.doc.objects[0].shape.contours()[c][i], expected);
        }
    }
    assert!(!s.drag(screen(4., 4.), screen(10., 10.)));
    assert_eq!(s.selected, [id]);
    assert!(selected_nodes(&s.ctx, id, 0).runs().is_empty());
    assert!(!styles::apply(
        &s.ctx,
        &mut s.doc,
        &s.selected,
        0,
        s.timeline.key_mode,
        HandleStyle::Smooth
    ));
}

#[test]
fn shift_marquee_extends_in_document_order_with_exact_strokes_and_layer_local_groups() {
    let mut s = CanvasSession::new(Tool::Select);
    s.doc.width = 128;
    s.doc.height = 128;
    let peer_layer = crate::commands::add_layer(&mut s.doc);
    let hidden = crate::commands::add_layer(&mut s.doc);
    s.doc.layers.last_mut().unwrap().visible = false;
    let locked = crate::commands::add_layer(&mut s.doc);
    s.doc.layers.last_mut().unwrap().locked = true;
    let mut grouped = |position, layer| {
        let id = s
            .doc
            .add_shape(position, [4.; 2], [255; 4], Shape::Rectangle, layer);
        s.doc.objects.last_mut().unwrap().group = Some(91);
        id
    };
    let near = grouped([18., 18.], 1);
    let sibling = grouped([96., 90.], 1);
    grouped([96., 90.], peer_layer); // A group ID only groups members within its layer.
    grouped([22., 22.], hidden);
    grouped([26., 24.], locked);
    let stroke = s
        .doc
        .add_shape([42., 34.], [22., 16.], [255; 4], Shape::Ellipse, 1);
    let o = s.doc.objects.last_mut().unwrap();
    o.stroke_width = 9.;
    o.rotation = 19.;
    o.animated.skew = 12.;
    let [_, _, r, b] = o.bounds(0);
    let prior = s.doc.add_rectangle([105., 104.], [4.; 2], [255; 4]);
    s.selected = vec![prior];
    s.modifiers = egui::Modifiers::SHIFT;
    let board = painted_board(&mut s);
    let screen = |x, y| board.min + egui::vec2(x, y) * (board.width() / 128.);
    let start = screen(5., 5.);
    s.input(egui::Event::PointerMoved(start));
    s.button(start, true);
    s.input(egui::Event::PointerMoved(screen(r - 1., b + 2.)));
    assert_eq!(
        s.selected,
        [near, sibling, prior],
        "the marquee clips the right edge of the stroke"
    );
    s.input(egui::Event::PointerMoved(screen(r + 2., b + 2.)));
    assert_eq!(s.selected, [near, sibling, stroke, prior]);
    s.button(screen(r - 1., b + 2.), false);
    assert_eq!(
        s.selected,
        [near, sibling, prior],
        "use the release position"
    );
}

#[test]
fn reverse_marquee_selects_pasteboard_geometry_and_rechecks_changed_layers() {
    let mut s = CanvasSession::new(Tool::Select);
    s.doc.width = 128;
    s.doc.height = 128;
    let x = 15_000.;
    let y = -8_000.;
    let id = s
        .doc
        .add_shape([x, y], [21., 13.], [255; 4], Shape::Ellipse, 1);
    let o = s.doc.objects.last_mut().unwrap();
    o.rotation = 31.;
    o.animated.skew = -13.;
    o.stroke_width = 5.;
    let [l, t, r, b] = o.bounds(0);
    s.doc.add_rectangle([x + 50., y + 40.], [3.; 2], [255; 4]);
    let board = painted_board(&mut s);
    let scale = board.width() / 128.;
    let center = board.min + egui::vec2((l + r) * 0.5, (t + b) * 0.5) * scale;
    s.canvas.pan = Pos2::new(200., 200.) - center;
    let origin = board.min + s.canvas.pan;
    let screen = |px, py| origin + egui::vec2(px, py) * scale;
    let start = screen(r + 2., b + 2.);
    let end = screen(l - 2., t - 2.);
    s.input(egui::Event::PointerMoved(start));
    s.button(start, true);
    s.input(egui::Event::PointerMoved(end));
    assert_eq!(s.selected, [id]);
    s.doc.layers[0].locked = true;
    s.input(egui::Event::PointerMoved(end + egui::vec2(-1., 0.)));
    assert!(s.selected.is_empty());
    s.doc.layers[0].locked = false;
    s.button(end, false);
    assert_eq!(s.selected, [id]);
}
