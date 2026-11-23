use super::*;

fn press(w: &mut EditorWindow, key: egui::Key, mods: egui::Modifiers) {
    for pressed in [true, false] {
        w.frame_with_input(egui::RawInput {
            modifiers: mods,
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: mods,
            }],
            ..Default::default()
        });
    }
}

fn make_project(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    project_io::save_animation(path, &AnimationProject::default(), false).unwrap();
}

#[test]
fn switch_qualified_tabs_by_mouse_and_keyboard_then_close() {
    let directory = tempfile::tempdir().unwrap();
    let fox = directory.path().join("fox").join("run.vanim");
    let hare = directory.path().join("hare").join("run.vanim");
    make_project(&fox);
    make_project(&hare);
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.editor.open_project(fox.clone());
    w.editor.open_project(hare.clone());
    assert_eq!(w.editor.document_tab_count(), 2);
    assert_eq!(w.editor.path.as_ref(), Some(&hare));
    w.click("fox / run.vanim");
    assert_eq!(w.editor.path.as_ref(), Some(&fox));
    let control = if cfg!(target_os = "macos") {
        egui::Modifiers::CTRL
    } else {
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND
    };
    press(&mut w, egui::Key::Tab, control);
    assert_eq!(w.editor.path.as_ref(), Some(&hare));
    press(&mut w, egui::Key::Tab, control | egui::Modifiers::SHIFT);
    assert_eq!(w.editor.path.as_ref(), Some(&fox));
    press(&mut w, egui::Key::W, egui::Modifiers::COMMAND);
    assert_eq!(w.editor.document_tab_count(), 1);
    assert_eq!(w.editor.path.as_ref(), Some(&hare));
    w.control("run.vanim");
}

#[test]
fn overflowing_tabs_reveal_active_document_and_close_an_inactive_tab_directly() {
    let directory = tempfile::tempdir().unwrap();
    let mut w = EditorWindow::new(egui::vec2(1450., 850.));
    for i in 0..10 {
        let path = directory.path().join(format!("pose-{i}.vanim"));
        make_project(&path);
        w.editor.open_project(path);
    }
    assert_eq!(w.editor.document_tab_count(), 10);
    w.control("pose-9.vanim");
    w.size = egui::vec2(800., 500.);
    let selected = w.control("pose-9.vanim");
    assert!(selected.x < w.control("Tabs").x);
    let painted = w.frame(vec![]).shapes.into_iter().filter(|s| {
        matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().starts_with("pose-") && t.galley.text().ends_with(".vanim"))
    }).count();
    assert!(painted <= 7, "painted {painted} tabs for a 746-point strip");
    w.click("Tabs");
    w.click("pose-0.vanim");
    assert_eq!(w.editor.active_document_tab(), 0);
    w.frame(vec![]);
    let next_id = w.editor.document_tab_info(1).0;
    let close_button = egui::Id::new(("close-document", next_id));
    let click_at = w.ctx.read_response(close_button).unwrap().rect.center();
    w.click_at(click_at);
    assert_eq!(w.editor.document_tab_count(), 9);
    assert!(w.editor.path.as_ref().unwrap().ends_with("pose-0.vanim"));
}
