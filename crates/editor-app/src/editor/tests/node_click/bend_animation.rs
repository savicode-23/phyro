use super::*;
use crate::timeline::KeyMode;
use document_core::Property;

fn ctrl_gesture(w: &mut EditorWindow, start: Pos2, end: Pos2) {
    let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    let button = |pos, pressed| Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers,
    };
    for event in [
        Event::PointerMoved(start),
        button(start, true),
        Event::PointerMoved(start.lerp(end, 0.5)),
        Event::PointerMoved(end),
        button(end, false),
    ] {
        w.frame_with_input(egui::RawInput {
            events: vec![event],
            modifiers,
            ..Default::default()
        });
    }
}

#[test]
fn pulling_a_handle_and_bending_a_segment_respect_geometry_keys() {
    for pull_handle in [false, true] {
        let nodes = [[0., 0.5], [0.5, 0.5], [1., 0.5]].map(PathNode::corner);
        let doc = drawing(
            Shape::Path {
                nodes: nodes.to_vec(),
                closed: false,
            },
            true,
        );
        let mut w = window(&doc);
        w.editor.selection = vec![doc.objects[0].id];
        w.editor.timeline.frame = 3;
        w.editor.timeline.key_mode = KeyMode::Auto;
        let gesture = |w: &mut EditorWindow, frame| {
            let board = w.board();
            let obj = &w.editor.scene().objects[0];
            let posed = obj.shape_at(frame);
            let contour = &posed.contours()[0];
            let local = if pull_handle {
                contour[1].point
            } else {
                point_on_segment(&contour[1], &contour[2], 0.5)
            };
            let start = screen(board, obj.local_to_world(local, frame));
            let delta = vec2(-14., -21.);
            if pull_handle {
                ctrl_gesture(w, start, start + delta);
            } else {
                w.drag(start, delta);
            }
        };
        gesture(&mut w, 3);
        let keyed = w.editor.scene().clone();
        let obj = &keyed.objects[0];
        assert_eq!(Property::Geometry.frames(obj), [0, 3]);
        assert_eq!(obj.shape, doc.objects[0].shape);
        assert_eq!(obj.shape_at(0).as_ref(), &doc.objects[0].shape);
        let posed = obj.shape_at(3);
        let changed = posed.contours()[0].as_slice();
        let mut expected = nodes;
        if pull_handle {
            assert_ne!(changed[1].incoming, nodes[1].incoming);
            expected[1].incoming = changed[1].incoming;
        } else {
            assert_ne!(changed[1].outgoing, nodes[1].outgoing);
            assert_ne!(changed[2].incoming, nodes[2].incoming);
            expected[1].outgoing = changed[1].outgoing;
            expected[2].incoming = changed[2].incoming;
        }
        assert_eq!(changed, expected);

        w.editor.timeline.frame = 1;
        w.editor.timeline.key_mode = KeyMode::Manual;
        gesture(&mut w, 1);
        assert_eq!(w.editor.scene(), &keyed);
        w.click_edit("Undo");
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }
}

#[test]
fn ctrl_click_with_pointer_jitter_toggles_the_clicked_nodes_style_with_undo() {
    let nodes = [[0., 0.2], [0.3, 0.8], [0.7, 0.3], [1., 0.5]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        true,
    );
    let obj = &doc.objects[0];
    let mut w = window(&doc);
    w.editor.selection = vec![obj.id];
    let board = w.board();
    let at = |i: usize| screen(board, obj.local_to_world(nodes[i].point, 0));
    w.click_at(at(1));
    use document_core::path_edit::HandleStyle::{Smooth, Symmetric};
    for style in [Symmetric, Smooth, Symmetric] {
        ctrl_gesture(&mut w, at(2), at(2) + vec2(4., 2.));
        let changed = &w.editor.scene().objects[0].shape.contours()[0];
        assert_eq!(changed[2].handle_style, style);
        assert_eq!(changed[2].point, nodes[2].point);
        assert_eq!(changed[..2], nodes[..2]);
        assert_eq!(changed[3], nodes[3]);
        assert_eq!(
            crate::path_editor::selected_range(&w.ctx, obj.id, 0),
            (0, 2, 2)
        );
    }
    for _ in 0..3 {
        w.editor.restore(false);
    }
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
}

#[test]
fn shift_click_jitter_does_not_create_symmetric_handles() {
    let nodes = [[0., 0.2], [0.5, 0.8], [1., 0.3]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        true,
    );
    let object = &doc.objects[0];
    let mut w = window(&doc);
    w.editor.selection = vec![object.id];
    let start = screen(w.board(), object.local_to_world(nodes[1].point, 0));
    let end = start + vec2(4., 2.);
    let modifiers = egui::Modifiers::SHIFT;
    let button = |pos, pressed| Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers,
    };
    for event in [
        Event::PointerMoved(start),
        button(start, true),
        Event::PointerMoved(end),
        button(end, false),
    ] {
        w.frame_with_input(egui::RawInput {
            modifiers,
            events: vec![event],
            ..Default::default()
        });
    }
    assert_eq!(
        crate::path_editor::selected_range(&w.ctx, object.id, 0),
        (0, 0, 1)
    );
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
}

#[test]
fn ctrl_drag_preserves_the_extended_handle_and_uses_only_real_endpoint_handles() {
    let mut nodes = [[0., 0.5], [0.5, 0.5], [1., 0.5]].map(PathNode::corner);
    nodes[1].incoming = [0.37, 0.29];
    nodes[1].handle_style = document_core::path_edit::HandleStyle::Smooth;
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        true,
    );
    for (i, incoming, delta) in [
        (1, false, vec2(-20., -13.)), // Drag toward an existing incoming handle.
        (0, false, vec2(-20., -13.)), // Outside the start of the open contour.
        (2, true, vec2(20., -13.)),   // Beyond the open contour's end.
    ] {
        let obj = &doc.objects[0];
        let mut w = window(&doc);
        w.editor.selection = vec![obj.id];
        let board = w.board();
        let at = |local| screen(board, obj.local_to_world(local, 0));
        let anchor = at(nodes[i].point);
        let tip = anchor + delta;
        ctrl_gesture(&mut w, anchor, tip);
        let changed = w.editor.scene().objects[0].shape.contours()[0].as_slice();
        let mut expected = nodes;
        expected[i].handle_style = document_core::path_edit::HandleStyle::Corner;
        let moved = if incoming {
            expected[i].incoming = changed[i].incoming;
            changed[i].incoming
        } else {
            expected[i].outgoing = changed[i].outgoing;
            changed[i].outgoing
        };
        assert!(
            at(moved).distance(tip) < 0.75,
            "node {i}, moved handle {moved:?}"
        );
        assert_eq!(changed, expected);
    }
}

#[test]
fn bending_the_closing_segment_only_curves_its_two_controls() {
    let nodes = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(PathNode::corner);
    let doc = drawing(
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: true,
        },
        true,
    );
    let obj = &doc.objects[0];
    let mut w = window(&doc);
    w.editor.selection = vec![obj.id];
    let board = w.board();
    let at = |local| screen(board, obj.local_to_world(local, 0));
    let start = at([0., 0.5]);
    let end = start + vec2(15., 9.);
    w.drag(start, end - start);
    let changed = w.editor.scene().objects[0].shape.contours()[0].as_slice();
    assert!(at(point_on_segment(&changed[3], &changed[0], 0.5)).distance(end) < 0.75);
    let mut expected = nodes;
    expected[3].outgoing = changed[3].outgoing;
    expected[0].incoming = changed[0].incoming;
    assert_eq!(changed, expected);
}
