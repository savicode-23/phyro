use super::*;
use document_core::{PathNode, Shape};

fn menu(window: &mut EditorWindow, label: &str) {
    window.click("Object");
    window.click("Clip and mask");
    window.click(label);
}

#[test]
fn object_menu_clips_and_releases_grouped_artwork_with_the_original_path_and_undo() {
    let mut doc = Document::default();
    let left = doc.add_rectangle([10., 24.], [32., 50.], [50, 130, 200, 255]);
    let right = doc.add_rectangle([42., 24.], [36., 50.], [160, 90, 50, 255]);
    commands::group(&mut doc, &[left, right]);
    let source = doc.add_shape(
        [20., 34.],
        [45., 50.],
        [255, 40, 40, 255],
        Shape::Path {
            closed: true,
            nodes: [[0., 0.], [1., 0.], [0.5, 1.]].map(PathNode::corner).into(),
        },
        1,
    );
    let cutter = doc.objects.last_mut().unwrap();
    cutter.name = "Triangular window".into();
    cutter.stroke = [20, 60, 80, 255];
    cutter.stroke_width = 2.;
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.selection = vec![source, left, right];
    w.board();
    menu(&mut w, "Set Clip");
    let clipped = w.editor.scene().clone();
    assert_eq!(w.editor.selection, [left, right]);
    assert_eq!(clipped.objects.len(), 2);
    let pixels = renderer::render(&clipped, 0).unwrap();
    assert_eq!(pixels.pixel(12, 26).unwrap().alpha(), 0);
    assert_eq!(pixels.pixel(32, 40).unwrap().blue(), 200);
    assert_eq!(pixels.pixel(50, 40).unwrap().red(), 160);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    w.click_edit("Redo");
    assert_eq!(w.editor.scene(), &clipped);
    menu(&mut w, "Release Clip");
    assert_eq!(w.editor.scene().objects, doc.objects);
    assert!(w.editor.selection.contains(&source));
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &clipped);
    w.click_edit("Redo");
    assert_eq!(w.editor.scene().objects, doc.objects);
}

#[test]
fn disabled_clip_commands_leave_single_unclipped_artwork_alone() {
    let mut doc = Document::default();
    let selected = doc.add_rectangle([5.; 2], [30.; 2], [210; 4]);
    for action in ["Set Clip", "Set Mask", "Release Clip"] {
        let mut w = EditorWindow::new(egui::vec2(800., 500.));
        w.ctx.style_mut(|s| s.animation_time = 0.);
        w.editor
            .replace_project(AnimationProject::from_document(doc.clone()), None);
        w.editor.selection = vec![selected];
        w.board();
        menu(&mut w, action);
        assert_eq!(w.editor.scene(), &doc);
        assert!(!w.editor.can_undo());
    }
}
