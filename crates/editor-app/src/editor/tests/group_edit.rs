use super::{pen, *};
use document_core::arrange::{Group, SelectionScope};
use egui::{Event, Modifiers, OutputCommand, PointerButton, Pos2};

fn scene() -> (EditorWindow, Group, [u64; 3]) {
    let mut doc = Document::default();
    let left = doc.add_rectangle([12., 23.], [20., 20.], [220, 80, 60, 255]);
    let right = doc.add_rectangle([63., 36.], [20., 20.], [60, 220, 100, 255]);
    let outside = doc.add_rectangle([95., 68.], [20., 20.], [60, 100, 220, 255]);
    commands::group(&mut doc, &[left, right]);
    let group = Group::of(&doc.objects[0]).unwrap();
    doc.rename_group(group.layer, group.id, "Lantern parts")
        .unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    (w, group, [left, right, outside])
}

fn point(w: &mut EditorWindow, [x, y]: [f32; 2]) -> Pos2 {
    let board = w.board();
    board.min + board.size() * egui::vec2(x / 128., y / 128.)
}

fn enter(w: &mut EditorWindow) {
    let left = point(w, [17., 29.]);
    pen::right_click(w, left);
    w.click("Enter group");
}

#[test]
fn group_edit_has_a_visible_scope_and_limits_canvas_edits() {
    let (mut w, group, [left, right, outside]) = scene();
    let before = w.editor.project.clone();
    enter(&mut w);
    assert_eq!(w.editor.selection_scope(), SelectionScope::InGroup(group));
    assert!(w.editor.selection.is_empty());
    assert!(!w.editor.can_undo());
    assert_eq!(w.editor.project, before);
    w.control("Editing group");
    w.control("Lantern parts");
    let output = w.frame(vec![]);
    assert!(output.shapes.iter().any(|s| matches!(
        &s.shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == crate::theme::MINT
    )));

    let outside_at = point(&mut w, [105., 77.]);
    w.click_at(outside_at);
    assert!(w.editor.selection.is_empty());
    let left_at = point(&mut w, [17., 29.]);
    assert_eq!(w.editor.selection_scope(), SelectionScope::InGroup(group));
    assert_eq!(
        w.editor
            .scene()
            .hit_test_in([17., 29.], 0, SelectionScope::InGroup(group)),
        Some(left)
    );
    w.click_at(left_at);
    assert_eq!(w.editor.selection, [left], "canvas hit at {left_at:?}");
    w.drag(left_at, egui::vec2(28., 0.));
    assert_eq!(w.editor.selection, [left]);
    assert_ne!(
        w.editor.scene().objects[0].position,
        before.clips[0].drawing_at(0).objects[0].position
    );
    assert_eq!(
        w.editor.scene().objects[1..],
        before.clips[0].drawing_at(0).objects[1..]
    );
    w.click_edit("Undo");
    assert_eq!(w.editor.project, before);

    let start = point(&mut w, [4., 16.]);
    let end = point(&mut w, [120., 99.]);
    w.drag(start, end - start);
    assert_eq!(w.editor.selection, [left, right]);
    pen::chord(&mut w, Key::A, Modifiers::COMMAND);
    assert_eq!(w.editor.selection, [left, right]);
    pen::chord(&mut w, Key::Escape, Modifiers::NONE);
    assert_eq!(w.editor.selection_scope(), SelectionScope::Groups);
    let outside_at = point(&mut w, [105., 77.]);
    w.click_at(outside_at);
    assert_eq!(w.editor.selection, [outside]);
}

#[test]
fn ctrl_click_copies_one_member_and_pasting_inside_joins_the_group() {
    let (mut w, group, [left, right, _]) = scene();
    let target = point(&mut w, [68., 42.]);
    w.click_at(target);
    assert_eq!(w.editor.selection, [left, right]);
    let modifiers = Modifiers::CTRL | Modifiers::COMMAND;
    for pressed in [true, false] {
        w.frame_with_input(egui::RawInput {
            modifiers,
            events: vec![Event::PointerButton {
                pos: target,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            }],
            ..Default::default()
        });
    }
    assert_eq!(w.editor.selection, [right]);
    assert_eq!(w.editor.selection_scope(), SelectionScope::Objects);
    assert!(!w.editor.can_undo());

    let copy = w
        .frame(vec![Event::Copy])
        .platform_output
        .commands
        .into_iter()
        .find_map(|command| match command {
            OutputCommand::CopyText(copy) => Some(copy),
            _ => None,
        })
        .expect("copied vector artwork");
    w.frame(vec![Event::Paste(copy.clone())]);
    assert_eq!(w.editor.scene().objects.len(), 4);
    let standalone = w.editor.scene().objects.last().unwrap();
    assert!(standalone.group.is_none());
    assert_eq!(standalone.position, [63., 36.]);
    w.click_edit("Undo");

    enter(&mut w);
    w.frame(vec![Event::Paste(copy)]);
    let added = w.editor.scene().objects.last().unwrap();
    assert_eq!(Group::of(added), Some(group));
    assert_eq!(added.group_name.as_deref(), Some("Lantern parts"));
    assert_eq!(w.editor.selection, [added.id]);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene().objects.len(), 3);
    assert_eq!(w.editor.selection_scope(), SelectionScope::InGroup(group));
    w.click("Exit group");
    assert_eq!(w.editor.selection_scope(), SelectionScope::Groups);
}

#[test]
fn group_edit_follows_the_layers_held_drawing() {
    let (mut w, group, _) = scene();
    let mut project = w.editor.project.clone();
    let clip = &mut project.clips[0];
    let other = clip.next_layer_id();
    let before = clip.clone();
    commands::add_layer_with_id(clip.drawing_at_mut(0), other);
    clip.sync_layer_edits(&before, 0).unwrap();
    clip.blank_layer_frame(2, other).unwrap();
    clip.blank_layer_frame(4, group.layer).unwrap();
    w.editor.replace_project(project, None);
    enter(&mut w);
    for frame in [1, 2, 3] {
        w.editor.timeline.frame = frame;
        w.frame(vec![]);
        assert_eq!(w.editor.selection_scope(), SelectionScope::InGroup(group));
    }
    w.editor.timeline.frame = 4;
    w.frame(vec![]);
    assert_eq!(w.editor.selection_scope(), SelectionScope::Groups);
}
