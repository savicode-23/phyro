use super::*;
use std::time::{Duration, Instant};

fn window(size: egui::Vec2) -> EditorWindow {
    let w = EditorWindow::new(size);
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w
}

fn labels(frame: &egui::FullOutput) -> Vec<&str> {
    frame
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if s.clip_rect
                    .intersects(t.galley.rect.translate(t.pos.to_vec2())) =>
            {
                Some(t.galley.text())
            }
            _ => None,
        })
        .collect()
}

fn wait_for(w: &mut EditorWindow, ready: impl Fn(&EditorWindow, &egui::FullOutput) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let frame = w.frame(vec![]);
        if ready(w, &frame) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out: {} – {:?}",
            w.editor.status,
            labels(&frame)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn project_at(path: &Path, clip_name: &str, color: [u8; 4]) -> AnimationProject {
    let mut drawing = Document {
        width: 128,
        height: 96,
        ..Default::default()
    };
    drawing.add_rectangle([18., 17.], [30., 31.], color);
    let mut project = AnimationProject::from_document(drawing);
    project.rename_clip(0, clip_name).unwrap();
    project_io::save_animation(path, &project, false).unwrap();
    project
}

#[test]
fn both_welcome_sizes_offer_new_open_and_recents_and_new_starts_a_real_drawing() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = window(size);
        w.editor.open_startup_paths(&w.ctx, Vec::new());
        assert!(w.editor.welcome_preview.is_some());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        for label in [
            "Phyro",
            "New project",
            "Open project…",
            "Recent projects",
            "No recent projects yet",
        ] {
            assert!(screen.contains(w.control(label)), "{label} on {size:?}");
        }
        w.click("New project");
        assert!(w.editor.welcome_preview.is_none());
        assert_eq!(w.editor.document_tab_count(), 1);
        assert!(w.editor.path.is_none());
        let board = w.board();
        w.click("Rectangle");
        w.drag(board.center(), egui::vec2(35., 28.));
        assert_eq!(w.editor.scene().objects.len(), 1);
        assert!(w.editor.can_undo());
    }
}

#[test]
fn clicking_a_recent_previews_its_artwork_and_open_uses_the_selected_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("Hero.vanim");
    let b = dir.path().join("Forest.vanim");
    project_at(&a, "Run", [145, 80, 24, 255]);
    let forest = project_at(&b, "Forest light", [9, 92, 58, 255]);
    let mut w = window(egui::vec2(1100., 740.));
    w.editor.recent_projects = vec![a.clone(), b.clone()];
    w.editor.open_startup_paths(&w.ctx, Vec::new());
    wait_for(&mut w, |_, f| {
        labels(f).iter().any(|t| t.contains("Run · Frame "))
    });
    w.click("Forest.vanim");
    wait_for(&mut w, |_, f| {
        labels(f)
            .iter()
            .any(|t| t.contains("Forest light · Frame "))
    });
    assert!(w.editor.path.is_none());
    assert!(w.editor.scene().objects.is_empty());
    assert_eq!(w.editor.recent_projects, [a.clone(), b.clone()]);
    w.click("Open selected project");
    wait_for(&mut w, |w, _| w.editor.file_batch.is_none());
    assert!(w.editor.welcome_preview.is_none());
    assert_eq!(w.editor.project, forest);
    assert_eq!(w.editor.path.as_ref(), Some(&b));
    assert_eq!(w.editor.recent_projects, [b, a]);
    assert_eq!(w.editor.document_tab_count(), 1);
}

#[test]
fn a_missing_recent_keeps_the_welcome_usable_and_another_recent_opens_on_double_click() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("Missing.vanim");
    let good = dir.path().join("Lantern.vanim");
    let project = project_at(&good, "Swing", [230, 168, 30, 255]);
    let mut w = window(egui::vec2(800., 500.));
    w.editor.recent_projects = vec![missing, good.clone()];
    w.editor.open_startup_paths(&w.ctx, Vec::new());
    w.click("Open selected project");
    wait_for(&mut w, |w, _| {
        w.editor.file_batch.is_none() && w.editor.status.contains("Missing.vanim")
    });
    assert!(w.editor.welcome_preview.is_some());
    assert_eq!(w.editor.recent_projects, std::slice::from_ref(&good));
    w.control("Open project…");
    let file = w.control("Lantern.vanim");
    w.click_at(file);
    w.click_at(file);
    wait_for(&mut w, |w, _| w.editor.path.as_ref() == Some(&good));
    assert_eq!(w.editor.project, project);
    assert!(w.editor.welcome_preview.is_none());
}

#[test]
fn direct_file_launch_bypasses_welcome_and_automatic_recovery_takes_priority() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hero.vanim");
    let original = project_at(&file, "Original", [0, 78, 50, 255]);
    let mut launched = window(egui::vec2(1100., 740.));
    launched
        .editor
        .open_startup_paths(&launched.ctx, vec![file.clone()]);
    let frame = launched.frame(vec![]);
    assert!(!labels(&frame).contains(&"Recent projects"));
    wait_for(&mut launched, |w, _| w.editor.path.as_ref() == Some(&file));
    assert_eq!(launched.editor.project, original);
    assert!(launched.editor.welcome_preview.is_none());

    let drafts = dir.path().join("drafts");
    let mut recovered = original;
    recovered.rename_clip(0, "Recovered hero").unwrap();
    let mut writer = crate::recovery::Writer::new(drafts.clone());
    writer
        .checkpoint(Some(file.clone()), recovered.clone())
        .unwrap();
    writer.wait_written().unwrap();
    drop(writer);
    let mut deferred = window(egui::vec2(1100., 740.));
    deferred.editor.enable_draft_recovery(drafts.clone());
    deferred
        .editor
        .open_startup_paths(&deferred.ctx, vec![file.clone()]);
    assert!(deferred.editor.recovery_dialog.is_open());
    wait_for(&mut deferred, |_, f| labels(f).contains(&"Later"));
    deferred.click("Later");
    wait_for(&mut deferred, |w, _| w.editor.path.as_ref() == Some(&file));
    drop(deferred);
    let mut w = window(egui::vec2(1100., 740.));
    w.editor.enable_draft_recovery(drafts);
    w.editor.open_startup_paths(&w.ctx, Vec::new());
    assert!(w.editor.recovery_dialog.is_open());
    wait_for(&mut w, |_, f| labels(f).contains(&"Recover a copy"));
    w.click("Recover a copy");
    wait_for(&mut w, |w, _| w.editor.project == recovered);
    assert!(w.editor.welcome_preview.is_none());
    assert!(w.editor.path.is_none());
    w.board();
}

#[test]
fn dragging_a_project_onto_welcome_opens_it_in_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Walk.vanim");
    let project = project_at(&path, "Walk cycle", [122, 106, 59, 255]);
    let mut w = window(egui::vec2(800., 500.));
    w.editor.open_startup_paths(&w.ctx, Vec::new());
    w.control("New project");
    w.frame_with_input(egui::RawInput {
        dropped_files: vec![egui::DroppedFile {
            path: Some(path.clone()),
            ..Default::default()
        }],
        ..Default::default()
    });
    wait_for(&mut w, |w, _| w.editor.path.as_ref() == Some(&path));
    assert!(w.editor.welcome_preview.is_none());
    assert_eq!(w.editor.project, project);
    assert_eq!(w.editor.document_tab_count(), 1);
}
