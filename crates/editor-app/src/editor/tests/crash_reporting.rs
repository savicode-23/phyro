use super::*;

#[test]
fn help_and_recovery_use_the_report_from_this_editing_session() {
    let temp = tempfile::tempdir().unwrap();
    let store = Diagnostics::in_dir(temp.path().join("reports"), "abc1234").unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.attach_diagnostics(Some(store.clone()));
    let mut doc = Document::default();
    let id = doc.add_rectangle([24., 24.], [32., 32.], [64, 140, 185, 255]);
    doc.objects[0].name = "Private customer artwork".into();
    let private_file = temp.path().join("Private project.vanim");
    project_io::save_animation(&private_file, &AnimationProject::from_document(doc), false)
        .unwrap();
    w.editor.open_project(private_file);
    assert!(!w.editor.crash_reports.is_open());
    let board = w.board();
    let center = board.min + board.size() * egui::vec2(40. / 128., 40. / 128.);
    w.click("Select");
    w.drag(center, egui::vec2(22., 8.));
    w.frame(vec![]);
    assert_eq!(w.editor.selection, [id]);
    let report = store.record_startup_failure("graphics", Some(93)).unwrap();
    let body = store.read(&report).unwrap();
    for expected in ["Objects: 1", "Tool: Select", "Open", "Move", "abc1234"] {
        assert!(body.contains(expected), "Missing {expected} in {body}");
    }
    assert!(!body.contains("Private") && !body.contains(temp.path().to_str().unwrap()));
    w.click("Help");
    w.click("Crash reports…");
    assert!(w.editor.crash_reports.is_open());
    w.click("Close");
    let mut restart = EditorWindow::new(egui::vec2(800., 500.));
    restart.editor.attach_diagnostics(Some(store.clone()));
    // Help may already count this report as reviewed. A newly saved one always appears.
    store.record_startup_failure("window", None).unwrap();
    restart.editor.attach_diagnostics(Some(store.clone()));
    restart.frame(vec![]);
    assert!(restart.editor.crash_reports.is_open());
    restart.click("Close");
    assert!(!restart.editor.crash_reports.is_open());
    let mut next = EditorWindow::new(egui::vec2(800., 500.));
    next.editor.attach_diagnostics(Some(store.clone()));
    assert!(!next.editor.crash_reports.is_open());
    assert_eq!(store.reports().unwrap().len(), 2);
}
