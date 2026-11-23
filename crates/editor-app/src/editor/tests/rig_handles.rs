use super::*;

#[test]
fn dragging_bone_tips_previews_artwork_and_planting_survives_timeline_edits() {
    let mut w = EditorWindow::with_artboard(egui::vec2(1100., 740.), [128; 2]);
    w.ctx.style_mut(|style| style.animation_time = 0.);
    let doc = w.editor.project.clips[0].drawing_at_mut(0);
    let hip = doc.add_bone("Body", None, [24., 56.], [40., 56.]).unwrap();
    let thigh = doc
        .add_bone("Thigh", Some(hip), [40., 56.], [65., 56.])
        .unwrap();
    let shin = doc
        .add_bone("Shin", Some(thigh), [65., 56.], [90., 56.])
        .unwrap();
    let art = doc.add_rectangle([79., 54.], [10., 4.], [255; 4]);
    doc.bind_rigid(art, Some(shin)).unwrap();
    doc.set_bone_rotation_key(hip, 0, 0.).unwrap();
    doc.set_bone_rotation_key(hip, 4, 10.).unwrap();
    let original = doc.clone();
    w.editor.project.clips[0]
        .cels
        .insert(2, std::sync::Arc::new(original.clone()));
    w.editor.capture_baseline();
    w.frame(vec![]);
    w.click("Rig");
    let board = w.board();
    let screen = |x: f32, y: f32| board.min + egui::vec2(x, y) * (board.width() / 128.);
    let (start, end) = (screen(90., 56.), screen(64., 74.));
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    w.frame(vec![egui::Event::PointerMoved(start)]);
    w.frame(vec![button(start, true)]);
    w.frame(vec![egui::Event::PointerMoved(end)]);
    let preview = w
        .editor
        .canvas
        .rig
        .handle_preview()
        .expect("displayed IK pose");
    assert_eq!(w.editor.scene(), &original);
    let pixels = renderer::render(&preview, 0).unwrap();
    assert_ne!(
        pixels.data(),
        renderer::render(&original, 0).unwrap().data()
    );
    pen::chord(&mut w, Key::Escape, egui::Modifiers::NONE);
    w.frame(vec![button(end, false)]);
    assert!(w.editor.canvas.rig.handle_preview().is_none());
    assert_eq!(w.editor.scene(), &original);

    w.drag(start, end - start);
    assert_eq!(
        renderer::render(w.editor.scene(), 0).unwrap().data(),
        pixels.data()
    );
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &original);
    w.click_edit("Redo");
    w.click("Pin foot");
    let doc = w.editor.scene();
    let target = doc.foot_pin_at(shin, 0).unwrap();
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.01;
    assert!(close(target, [64., 74.]));
    assert!(close(doc.bone_segment_at(shin, true, 4).unwrap().1, target));
    let pinned = w.editor.project.clone();
    assert!(close(
        pinned.clips[0]
            .drawing_at(4)
            .bone_segment_at(shin, true, 4)
            .unwrap()
            .1,
        target
    ));
    w.drag(end, egui::vec2(-12., 16.));
    assert_eq!(w.editor.project, pinned);
    w.click_edit("Undo");
    assert!(w.editor.project.clips[0]
        .drawing_at(0)
        .foot_pin_at(shin, 0)
        .is_none());
    assert!(w.editor.project.clips[0]
        .drawing_at(4)
        .foot_pin_at(shin, 4)
        .is_none());
    w.click_edit("Redo");
    assert_eq!(w.editor.project, pinned);
    w.click("3");
    w.click_at(end);
    w.click("Release foot");
    assert!(w.editor.project.clips[0]
        .drawing_at(1)
        .foot_pin_at(shin, 1)
        .is_some());
    assert!(w.editor.scene().foot_pin_at(shin, 2).is_none());
    w.click_edit("Undo");
    assert_eq!(w.editor.project, pinned);
    w.click_edit("Redo");
    assert!(w.editor.scene().foot_pin_at(shin, 4).is_none());
}
