use super::*;
use std::fs;

fn file(dir: &Path, filename: &str) -> PathBuf {
    let path = dir.join(filename);
    let mut project = AnimationProject::from_document(Document {
        width: 128,
        height: 96,
        ..Default::default()
    });
    let clip = &mut project.clips[0];
    clip.set_length(12).unwrap();
    clip.drawing_at_mut(0)
        .add_rectangle([20., 20.], [30., 30.], [100, 60, 30, 255]);
    project_io::save_animation(&path, &project, false).unwrap();
    path
}

fn rename(editor: &mut Editor, name: &str) {
    editor.project.rename_clip(0, name).unwrap();
    editor.finish_edit();
}

#[test]
fn tabs_keep_their_drawing_view_selection_playhead_and_undo_redo() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "A.vanim");
    let b = file(dir.path(), "B.vanim");
    let mut w = EditorWindow::new(egui::vec2(1000., 680.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor.open_project(a);
    assert_eq!(w.editor.document_tab_count(), 1);
    let tab_a = w.editor.active_document_tab();
    let clean_a = w.editor.project.clone();
    w.editor.timeline.frame = 4;
    let id = w.editor.scene().objects[0].id;
    w.editor.selection = vec![id];
    let fit = w.board();
    w.frame(vec![egui::Event::PointerMoved(fit.center())]);
    w.frame(vec![egui::Event::Zoom(1.5)]);
    let zoomed = w.board();
    assert!(zoomed.width() > fit.width() * 1.4);
    rename(&mut w.editor, "A in progress");
    let edited_a = w.editor.project.clone();
    w.editor.open_project(b);
    let tab_b = w.editor.active_document_tab();
    assert_ne!(tab_b, tab_a);
    assert_eq!(w.editor.timeline.frame, 0);
    assert!(w.editor.selection.is_empty());
    assert!(!w.editor.can_undo());
    let clean_b = w.editor.project.clone();
    assert!(w.board().width() < zoomed.width());
    w.editor.timeline.frame = 9;
    rename(&mut w.editor, "B in progress");
    let edited_b = w.editor.project.clone();
    w.editor.restore(false);
    assert_eq!(w.editor.project, clean_b);
    w.editor.select_document_tab(tab_a);
    assert_eq!(w.editor.project, edited_a);
    assert_eq!(w.editor.timeline.frame, 4);
    assert_eq!(w.editor.selection, [id]);
    assert_eq!(w.board(), zoomed);
    w.editor.restore(false);
    assert_eq!(w.editor.project, clean_a);
    w.editor.select_document_tab(tab_b);
    assert_eq!(w.editor.project, clean_b);
    assert_eq!(w.editor.timeline.frame, 9);
    w.editor.restore(true);
    assert_eq!(w.editor.project, edited_b);
    w.editor.select_document_tab(tab_a);
    w.editor.restore(true);
    assert_eq!(w.editor.project, edited_a);
}

#[test]
fn opening_aliases_focuses_live_edits_and_same_named_files_get_distinct_tabs() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    fs::create_dir(one.path().join("work")).unwrap();
    let a = file(one.path(), "hero.vanim");
    let b = file(two.path(), "hero.vanim");
    let alias = one.path().join("work").join("..").join("hero.vanim");
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    w.editor.open_project(a.clone());
    let tab_a = w.editor.active_document_tab();
    let tab_a_id = w.editor.document_tab_info(tab_a).0;
    rename(&mut w.editor, "Local hero");
    let mut external = project_io::load_animation(&a).unwrap();
    external.rename_clip(0, "External hero").unwrap();
    project_io::save_animation(&a, &external, true).unwrap();
    w.editor.open_project(b.clone());
    let tab_b = w.editor.active_document_tab();
    assert_ne!(tab_a_id, w.editor.document_tab_info(tab_b).0);
    assert_eq!(w.editor.document_tab_count(), 2);
    w.editor.open_project(alias.clone());
    assert_eq!(w.editor.active_document_tab(), tab_a);
    assert_eq!(w.editor.project.clips[0].name, "Local hero");
    assert!(w.editor.can_undo());
    fs::remove_file(&a).unwrap();
    w.editor.select_document_tab(tab_b);
    w.editor.open_project(alias);
    assert_eq!(w.editor.active_document_tab(), tab_a);
    assert_eq!(w.editor.document_tab_count(), 2);
    assert_eq!(w.editor.document_tab_info(tab_a).0, tab_a_id);
    assert_eq!(w.editor.project.clips[0].name, "Local hero");
    let other_on_disk = fs::read(&b).unwrap();
    assert!(!w.editor.save_to(b.clone(), true));
    assert_eq!(fs::read(&b).unwrap(), other_on_disk);
    assert_eq!(w.editor.path.as_ref(), Some(&a));
}

#[test]
fn closing_documents_honors_cancel_discard_and_save_and_leaves_an_editable_blank() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path(), "Fox.vanim");
    let mut w = EditorWindow::new(egui::vec2(900., 640.));
    rename(&mut w.editor, "Unfiled sketch");
    w.editor.open_project(path.clone());
    assert_eq!(w.editor.document_tab_count(), 2);
    let file_tab = w.editor.active_document_tab();
    w.editor.new_document_tab();
    let new_tab = w.editor.active_document_tab();
    assert_eq!(w.editor.document_tab_count(), 3);
    assert!(w.editor.path.is_none());
    assert!(!w.editor.can_undo());
    rename(&mut w.editor, "Discard me");
    assert!(!w
        .editor
        .close_document_tab_with(new_tab, |_| OpenDecision::Cancel));
    assert_eq!(w.editor.project.clips[0].name, "Discard me");
    assert_eq!(w.editor.document_tab_count(), 3);
    assert!(w
        .editor
        .close_document_tab_with(new_tab, |_| OpenDecision::Proceed));
    assert_eq!(w.editor.document_tab_count(), 2);
    w.editor.select_document_tab(file_tab);
    rename(&mut w.editor, "Saved on close");
    assert!(w.editor.close_document_tab_with(file_tab, |e| {
        assert!(e.save(false));
        OpenDecision::Saved
    }));
    assert_eq!(
        project_io::load_animation(&path).unwrap().clips[0].name,
        "Saved on close"
    );
    assert_eq!(w.editor.document_tab_count(), 1);
    assert_eq!(w.editor.project.clips[0].name, "Unfiled sketch");
    assert!(w
        .editor
        .close_document_tab_with(0, |_| OpenDecision::Proceed));
    assert_eq!(w.editor.document_tab_count(), 1);
    assert!(w.editor.path.is_none());
    assert!(!w.editor.can_undo());
    rename(&mut w.editor, "Next drawing");
    assert!(w.editor.can_undo());
}

#[test]
fn cancelling_quit_keeps_every_dirty_document_and_its_undo_history() {
    let drafts = tempfile::tempdir().unwrap();
    let mut w = EditorWindow::new(egui::vec2(900., 640.));
    w.editor.enable_draft_recovery(drafts.path().to_owned());
    rename(&mut w.editor, "First");
    // The first tab leaves the foreground before its draft reaches the idle deadline.
    w.editor.new_document_tab();
    rename(&mut w.editor, "Second");
    w.editor.poll_draft_recovery(
        &w.ctx,
        std::time::Instant::now() + std::time::Duration::from_secs(30),
    );
    for index in 0..2 {
        w.editor.select_document_tab(index);
        w.editor
            .drafts
            .as_ref()
            .unwrap()
            .writer
            .wait_written()
            .unwrap();
    }
    let checkpoint_names = || {
        let mut names: Vec<_> = fs::read_dir(drafts.path())
            .unwrap()
            .map(|e| e.unwrap().path().join("draft.vanim"))
            .filter(|p| p.is_file())
            .map(|p| {
                project_io::load_animation(&p).unwrap().clips[0]
                    .name
                    .clone()
            })
            .collect();
        names.sort();
        names
    };
    assert_eq!(checkpoint_names(), ["First", "Second"]);
    let mut asked = Vec::new();
    assert!(!w.editor.confirm_close_all_with(|editor| {
        asked.push(editor.project.clips[0].name.clone());
        if asked.len() == 1 {
            OpenDecision::Proceed
        } else {
            OpenDecision::Cancel
        }
    }));
    asked.sort();
    assert_eq!(asked, ["First", "Second"]);
    assert_eq!(w.editor.document_tab_count(), 2);
    for (index, name) in ["First", "Second"].into_iter().enumerate() {
        w.editor.select_document_tab(index);
        assert_eq!(w.editor.project.clips[0].name, name);
        assert!(w.editor.can_undo());
    }
    assert_eq!(checkpoint_names(), ["First", "Second"]);
    assert!(w
        .editor
        .close_document_tab_with(0, |_| OpenDecision::Proceed));
    assert_eq!(checkpoint_names(), ["Second"]);
}
