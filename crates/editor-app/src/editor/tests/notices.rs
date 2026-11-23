use super::*;

#[test]
fn failed_save_keeps_the_edit_and_its_details_can_be_copied_after_dismissal() {
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Drawing.vanim");
    project_io::save_animation(&path, &w.editor.project, false).unwrap();
    w.editor
        .project
        .rename_clip(0, "In-progress drawing")
        .unwrap();
    w.editor.finish_edit();
    let board = w.board();
    assert!(!w.editor.save_to(path.clone(), false));
    w.frame(vec![]);
    assert!(w.control("Could not save project").y < 140.);
    assert_eq!(w.board(), board);
    assert_eq!(w.editor.project.clips[0].name, "In-progress drawing");
    assert!(w.editor.can_undo());
    assert_ne!(project_io::load_animation(&path).unwrap(), w.editor.project);

    w.click("Dismiss");
    let output = w.frame(vec![]);
    assert!(output.shapes.iter().all(|shape| {
        !matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Could not save project")
    }));
    w.click("Help");
    w.click("Recent issues");
    assert!(w.editor.notices.open);
    let at = w.control("Copy details");
    w.frame(vec![egui::Event::PointerMoved(at)]);
    for pressed in [true, false] {
        let output = w.frame(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
        if !pressed {
            assert!(output.platform_output.commands.iter().any(|command| {
                matches!(command, egui::OutputCommand::CopyText(text)
                    if text.contains("Drawing.vanim") && text.contains("Use File > Save as"))
            }));
        }
    }
    w.click("Close");
    assert!(!w.editor.notices.open);
}

#[test]
fn startup_open_failure_can_show_details_directly() {
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    let dir = tempfile::tempdir().unwrap();
    w.editor
        .open_startup_paths(&w.ctx, vec![dir.path().join("Missing.vanim")]);
    w.frame(vec![]);
    w.control("Could not open project");
    w.click("Details");
    assert!(w.editor.welcome_preview.is_some());
    assert!(w.editor.notices.open);
    w.frame(vec![]);
    w.control("Copy details");
    w.click("Close");
    w.click("Recent issues");
    assert!(w.editor.notices.open);
}
