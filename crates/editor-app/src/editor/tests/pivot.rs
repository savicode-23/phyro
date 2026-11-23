use super::*;

#[test]
fn select_shows_a_pivot_that_can_be_dragged_off_the_artwork_and_grabbed_again() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([24., 32.], [48., 36.], [155, 84, 30, 255]);
    let mut w = EditorWindow::new(egui::vec2(1050., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.style.snap.enabled = false;
    let board = w.board();
    let screen = |x, y| board.min + board.size() * (egui::vec2(x, y) / 128.);
    let center = screen(48., 50.);
    let outside = screen(87., 72.);
    let body = screen(34., 42.);
    let rim = egui::vec2(4., 0.);
    w.click_at(body);
    assert_eq!(w.editor.selection, [id]);
    let paint = w.frame(vec![]);
    assert!(paint
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Circle(c)
        if c.center.distance(center) < 0.01 && (c.radius - 4.).abs() < 0.01)));
    w.click_at(center + rim);
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());

    w.drag(center + rim, outside - center);
    let placed = w.editor.scene().clone();
    let placed_point = egui::Pos2::from(placed.objects[0].pivot_at(0));
    assert!(placed_point.distance(egui::pos2(87., 72.)) < 0.001);
    assert_eq!(placed.objects[0].position, doc.objects[0].position);
    w.drag(outside, screen(80., 14.) - outside);
    assert_eq!(w.editor.selection, [id]);
    let new_point = egui::Pos2::from(w.editor.scene().objects[0].pivot_at(0));
    assert!(new_point.distance(egui::pos2(80., 14.)) < 0.001);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &placed);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());
    w.drag(body, screen(39., 48.) - body);
    assert_ne!(
        w.editor.scene().objects[0].position,
        doc.objects[0].position
    );
    assert_eq!(w.editor.scene().objects[0].pivot, None);
}

#[test]
fn placing_and_resetting_a_pivot_are_independent_edits_and_escape_restores_normal_dragging() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([24., 32.], [48., 36.], [155, 84, 30, 255]);
    let mut w = EditorWindow::new(egui::vec2(1050., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(doc.clone()), None);
    w.editor.style.snap.enabled = false;
    let board = w.board();
    let screen = |x, y| board.min + board.size() * (egui::vec2(x, y) / 128.);
    let center = screen(48., 50.);
    let endpoint = screen(85., 61.);
    w.click_at(center);
    w.click("Edit pivot");
    let primary = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    w.frame(vec![
        egui::Event::PointerMoved(center),
        primary(center, true),
    ]);
    let preview = w.frame(vec![egui::Event::PointerMoved(endpoint)]);
    assert_eq!(w.editor.scene(), &doc);
    assert!(preview
        .shapes
        .iter()
        .any(|shape| matches!(&shape.shape, egui::Shape::Circle(c)
        if c.center.distance(endpoint) < 0.01 && (c.radius - 4.).abs() < 0.01)));
    w.frame(vec![primary(endpoint, false)]);
    let placed = w.editor.scene().clone();
    let object = &placed.objects[0];
    assert!(egui::Pos2::from(object.pivot_at(0)).distance(egui::pos2(85., 61.)) < 0.001);
    assert_eq!(
        renderer::render(&placed, 0).unwrap().data(),
        renderer::render(&doc, 0).unwrap().data()
    );

    pen::chord(&mut w, Key::Escape, egui::Modifiers::NONE);
    assert_eq!(w.editor.selection, [id]);
    w.drag(center, egui::vec2(23., 7.));
    assert_ne!(w.editor.scene().objects[0].position, object.position);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &placed);
    w.click("Reset pivot");
    assert_eq!(w.editor.scene(), &doc);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &placed);
    w.click_edit("Undo");
    assert_eq!(w.editor.scene(), &doc);
}
