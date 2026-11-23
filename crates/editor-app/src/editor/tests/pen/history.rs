use super::*;
use egui::{vec2, Event, Modifiers};

fn undo(w: &mut EditorWindow) {
    chord(w, Key::Z, Modifiers::COMMAND);
}
fn redo(w: &mut EditorWindow) {
    chord(w, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
}

#[test]
fn pen_undo_and_redo_restore_one_node_and_its_handles_then_branch() {
    let mut w = EditorWindow::with_artboard(vec2(1100., 740.), [128; 2]);
    key(&mut w, Key::B);
    let board = w.board();
    let at = |x, y| on_board(board, x, y);
    let (a, b, c, d, replacement) = (
        at(10., 30.),
        at(40., 50.),
        at(67., 32.),
        at(91., 68.),
        at(79., 93.),
    );
    w.click_at(a);
    w.drag(b, vec2(11., -18.));
    w.drag(c, vec2(-9., 14.));
    w.click_at(d);
    let original = w.frame(vec![Event::PointerMoved(at(112., 65.))]);
    let ab = preview_segment(&original, a, b);
    let bc = preview_segment(&original, b, c);
    let cd = preview_segment(&original, c, d);
    assert!(bc[1].distance(b) > 5. && bc[2].distance(c) > 5.);

    w.click_edit("Undo");
    undo(&mut w);
    assert_eq!(preview_segment(&w.frame(vec![]), a, b), ab);
    assert!(w.editor.scene().objects.is_empty() && !w.editor.history.can_undo());
    redo(&mut w);
    assert_eq!(preview_segment(&w.frame(vec![]), b, c), bc);
    w.click_edit("Redo");
    assert_eq!(preview_segment(&w.frame(vec![]), c, d), cd);

    undo(&mut w);
    w.click_at(replacement);
    assert_eq!(w.editor.canvas.pen_history_available(true), Some(false));
    redo(&mut w);
    right_click(&mut w, replacement);
    let committed = w.editor.project.clone();
    let object = &w.editor.scene().objects[0];
    let nodes = object.shape.contours()[0].as_slice();
    assert_eq!(nodes.len(), 4);
    let last = object.local_to_world(nodes[3].point, 0);
    assert!(at(last[0], last[1]).distance(replacement) < 0.1);
    assert_eq!(w.editor.canvas.pen_history_available(false), None);
    undo(&mut w);
    assert!(w.editor.scene().objects.is_empty());
    redo(&mut w);
    assert_eq!(w.editor.project, committed);
}

#[test]
fn undoing_the_first_pen_node_ends_the_draft_before_touching_prior_history() {
    let mut w = EditorWindow::with_artboard(vec2(1100., 740.), [128; 2]);
    let board = w.board();
    let (a, b) = (on_board(board, 15., 17.), on_board(board, 84., 92.));
    key(&mut w, Key::E);
    w.drag(a, vec2(24., 31.));
    let previous = w.editor.project.clone();
    key(&mut w, Key::B);
    w.click_at(b);
    undo(&mut w);
    assert!(!w.editor.canvas.path_in_progress());
    assert_eq!(w.editor.project, previous);
    redo(&mut w);
    assert!(w.editor.canvas.pen_in_progress());
    undo(&mut w);
    undo(&mut w);
    assert!(w.editor.scene().objects.is_empty());
    // Once document Undo ran, Redo must restore the ellipse rather than a draft.
    redo(&mut w);
    assert!(!w.editor.canvas.pen_in_progress());
    assert_eq!(w.editor.project, previous);
    undo(&mut w);
    w.click_at(b);
    // A live outline must not expose the unrelated document redo.
    redo(&mut w);
    assert!(w.editor.canvas.pen_in_progress() && w.editor.scene().objects.is_empty());
    key(&mut w, Key::Escape);
    redo(&mut w);
    assert_eq!(w.editor.project, previous);
}

#[test]
fn undo_from_either_open_endpoint_keeps_the_source_and_can_reattach() {
    for prepend in [false, true] {
        let (mut w, original) = continuation::selected_open_path();
        let board = w.board();
        let source = &original.objects[0];
        let old = &source.shape.contours()[0];
        let attached = if prepend {
            old[0]
        } else {
            *old.last().unwrap()
        };
        let join = continuation::on_object(board, source, attached.point);
        let sign = if prepend { -1. } else { 1. };
        let a = join + vec2(sign * 39., 17.);
        let b = join + vec2(sign * 80., -26.);
        w.click_at(join);
        w.drag(a, vec2(7., 13.));
        w.click_at(b);
        undo(&mut w);
        w.click_edit("Undo");
        assert!(w.editor.canvas.pen_in_progress());
        assert_eq!(w.editor.scene(), &original);
        undo(&mut w);
        assert!(!w.editor.canvas.pen_in_progress());
        assert_eq!(w.editor.scene(), &original);
        redo(&mut w);
        assert!(w.editor.canvas.pen_in_progress());
        let replacement = join + vec2(sign * 43., -12.);
        w.click_at(replacement);
        key(&mut w, Key::Enter);
        let [updated] = w.editor.scene().objects.as_slice() else {
            panic!("expected one continued path")
        };
        assert_eq!(updated.id, source.id);
        let nodes = &updated.shape.contours()[0];
        assert_eq!(nodes.len(), old.len() + 1);
        let offset = usize::from(prepend);
        assert_eq!(&nodes[offset..offset + old.len()], old.as_slice());
        undo(&mut w);
        assert_eq!(w.editor.scene(), &original);
    }
}
