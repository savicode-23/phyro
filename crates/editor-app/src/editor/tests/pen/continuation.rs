use super::*;
use document_core::{PathNode, VectorObject};
use egui::{vec2, Event, Pos2, Rect};

pub(super) fn selected_open_path() -> (EditorWindow, Document) {
    let mut nodes = [[0., 0.3], [0.45, 0.8], [1., 0.4]].map(PathNode::corner);
    nodes[0].outgoing = [0.15, -0.1];
    nodes[1].incoming = [0.25, 0.55];
    nodes[1].outgoing = [0.65, 1.05];
    nodes[2].incoming = [0.8, 0.2];
    let mut doc = Document::default();
    let id = doc.add_shape(
        [30., 33.],
        [58., 53.],
        [81, 62, 139, 255],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        1,
    );
    let obj = &mut doc.objects[0];
    obj.name = "Ribbon".into();
    obj.no_fill = true;
    obj.stroke = [201, 70, 41, 255];
    obj.stroke_width = 3.;
    obj.rotation = 14.;
    obj.scale = [0.85, 1.15];
    obj.animated.skew = 6.;
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    key(&mut w, Key::S);
    let middle = on_object(w.board(), &doc.objects[0], nodes[1].point);
    w.click_at(middle);
    assert_eq!(w.editor.selection, [id]);
    key(&mut w, Key::B);
    (w, doc)
}

pub(super) fn on_object(board: Rect, obj: &VectorObject, local: [f32; 2]) -> Pos2 {
    let [x, y] = obj.local_to_world(local, 0);
    on_board(board, x, y)
}

#[test]
fn pen_extends_either_endpoint_in_place_and_undo_restores_the_exact_object() {
    for prepend in [false, true] {
        let (mut w, original) = selected_open_path();
        let old = &original.objects[0];
        let old_nodes = &old.shape.contours()[0];
        let board = w.board();
        let endpoint = on_object(board, old, old_nodes[if prepend { 0 } else { 2 }].point);
        let x = if prepend { 18. } else { 111. };
        let curved = on_board(board, x, 80.);
        let end = on_board(board, x + 1., 104.);
        w.click_at(endpoint + vec2(2., -1.));
        let preview = w.frame(vec![Event::PointerMoved(curved)]);
        preview_segment(&preview, endpoint, curved);
        let tangent = vec2(13., -17.);
        w.drag(curved, tangent);
        w.click_at(end);
        assert_eq!(w.editor.scene(), &original);
        assert!(!w.editor.can_undo());
        if prepend {
            right_click(&mut w, end);
        } else {
            key(&mut w, Key::Enter);
        }

        let edited = w.editor.scene().clone();
        let [object] = edited.objects.as_slice() else {
            panic!("expected one extended object")
        };
        assert_eq!(w.editor.selection, [old.id]);
        assert!(w.editor.tool == Tool::Pen);
        let Shape::Path {
            nodes,
            closed: false,
        } = &object.shape
        else {
            panic!("expected open path")
        };
        assert_eq!(nodes.len(), 5);
        let offset = if prepend { 2 } else { 0 };
        assert_eq!(
            &nodes[offset..offset + old_nodes.len()],
            old_nodes.as_slice()
        );
        let curved_node = nodes[if prepend { 1 } else { 3 }];
        let tip = if prepend {
            curved_node.incoming
        } else {
            curved_node.outgoing
        };
        let last = nodes[if prepend { 0 } else { 4 }].point;
        for (p, screen) in [
            (curved_node.point, curved),
            (tip, curved + tangent),
            (last, end),
        ] {
            assert!(on_object(board, object, p).distance(screen) < 0.15);
        }
        let mut expected = original.clone();
        expected.objects[0].shape = object.shape.clone();
        assert_eq!(edited, expected);
        w.click_edit("Undo");
        assert_eq!(w.editor.scene(), &original);
        assert!(!w.editor.can_undo());
        w.click_edit("Redo");
        assert_eq!(w.editor.scene(), &edited);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Ribbon.vanim");
        w.editor.path = Some(path.clone());
        chord(
            &mut w,
            Key::S,
            egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        );
        let mut reopened = EditorWindow::new(w.size);
        reopened.editor.open_project(path);
        assert_eq!(reopened.editor.scene(), &edited);
    }
}

#[test]
fn pen_can_drag_a_tangent_at_either_end_and_close_at_the_opposite_endpoint() {
    for prepend in [false, true] {
        let (mut w, original) = selected_open_path();
        let old = &original.objects[0];
        let mut expected = old.shape.contours()[0].clone();
        let join = if prepend { 0 } else { 2 };
        let board = w.board();
        let start = on_object(board, old, expected[join].point);
        let close = on_object(board, old, expected[2 - join].point);
        let tangent = vec2(-21., 17.);
        w.drag(start + vec2(2., -1.), tangent);
        let added = on_board(board, 99., 102.);
        let preview = w.frame(vec![Event::PointerMoved(added)]);
        assert!(preview_segment(&preview, start, added)[1].distance(start + tangent) < 0.15);
        w.click_at(added);
        w.click_at(close + vec2(2., 1.));
        let [object] = w.editor.scene().objects.as_slice() else {
            panic!("expected one closed object")
        };
        let Shape::Path {
            nodes,
            closed: true,
        } = &object.shape
        else {
            panic!("expected closed path")
        };
        assert_eq!(object.id, old.id);
        assert_eq!(nodes.len(), 4);
        let offset = usize::from(prepend);
        let attached = nodes[offset + join];
        let tip = if prepend {
            expected[join].incoming = attached.incoming;
            attached.incoming
        } else {
            expected[join].outgoing = attached.outgoing;
            attached.outgoing
        };
        assert!(on_object(board, object, tip).distance(start + tangent) < 0.15);
        assert_eq!(&nodes[offset..offset + 3], expected.as_slice());
        let new_node = &nodes[if prepend { 0 } else { 3 }];
        assert!(on_object(board, object, new_node.point).distance(added) < 0.15);
        w.click_edit("Undo");
        assert_eq!(w.editor.scene(), &original);
    }
}

#[test]
fn cancelling_a_pen_extension_and_starting_beside_the_endpoint_preserves_the_source() {
    let (mut w, original) = selected_open_path();
    let old = &original.objects[0];
    let board = w.board();
    let endpoint = on_object(board, old, old.shape.contours()[0][2].point);
    w.click_at(endpoint);
    assert_eq!(w.editor.selection, [old.id]);
    w.click_at(on_board(board, 104., 96.));
    key(&mut w, Key::Escape);
    assert_eq!(w.editor.scene(), &original);
    assert!(!w.editor.can_undo());

    w.editor.selection = vec![old.id];
    w.click_at(endpoint);
    key(&mut w, Key::Enter);
    assert_eq!(w.editor.scene(), &original);
    assert_eq!(w.editor.selection, [old.id]);
    assert!(!w.editor.can_undo());

    let beside = endpoint + vec2(19., -1.);
    w.click_at(beside);
    w.click_at(on_board(board, 107., 36.));
    key(&mut w, Key::Enter);
    let [existing, added] = w.editor.scene().objects.as_slice() else {
        panic!("expected a separate path")
    };
    assert_eq!(existing, old);
    assert_ne!(added.id, old.id);
    let Shape::Path {
        nodes,
        closed: false,
    } = &added.shape
    else {
        panic!("expected open path")
    };
    assert_eq!(nodes.len(), 2);
    assert!(on_object(board, added, nodes[0].point).distance(beside) < 0.15);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &original);
    assert!(!w.editor.can_undo());
}
