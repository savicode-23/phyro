use super::*;

fn finish_opening_files(w: &mut EditorWindow) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while w.editor.file_batch.is_some() {
        w.frame(Vec::new());
        assert!(std::time::Instant::now() < deadline, "file open timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn startup_files_wait_for_recovery_and_open_beside_the_recovered_tab() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("drafts");
    let mut project = AnimationProject::default();
    project.rename_clip(0, "Recovered artwork").unwrap();
    {
        let mut writer = crate::recovery::Writer::new(root.clone());
        writer.checkpoint(None, project.clone()).unwrap();
        writer.wait_written().unwrap();
    }
    let paths = ["Character.vanim", "Scene.vanim"].map(|p| dir.path().join(p));
    for path in &paths {
        project_io::save_animation(path, &AnimationProject::default(), false).unwrap();
    }
    let mut w = EditorWindow::new(egui::vec2(900., 640.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor.enable_draft_recovery(root);
    w.editor.open_startup_paths(&w.ctx, paths.to_vec());
    w.frame(vec![]);
    assert!(w.editor.recovery_dialog.is_open());
    assert_eq!(w.editor.document_tab_count(), 1);
    assert!(w.editor.path.is_none());
    w.click("Recover a copy");
    finish_opening_files(&mut w);
    assert_eq!(w.editor.document_tab_count(), 3);
    assert_eq!(w.editor.path.as_ref(), Some(&paths[1]));
    w.editor.select_document_tab(0);
    assert_eq!(w.editor.project, project);
    assert!(!w.editor.baseline_is_saved);
}

#[test]
fn pasted_image_is_selected_on_the_active_layer_and_included_in_exports() {
    let mut w = EditorWindow::with_artboard(egui::vec2(850., 670.), [64, 64]);
    let mut original = w.editor.project.clone();
    let layer = commands::add_layer(original.clips[0].drawing_at_mut(0));
    let mut pixels = renderer::Pixmap::new(4, 2).unwrap();
    pixels.data_mut().fill(255);
    let image = std::sync::Arc::new(
        document_core::RasterImage::from_bytes(&pixels.encode_png().unwrap()).unwrap(),
    );
    for (visible, locked) in [(false, false), (true, true)] {
        let mut blocked = original.clone();
        let active = &mut blocked.clips[0].drawing_at_mut(0).layers[1];
        active.visible = visible;
        active.locked = locked;
        w.editor.replace_project(blocked.clone(), None);
        w.editor.style.layer = layer;
        assert!(w.editor.paste_raster_image(image.clone()).is_err());
        assert_eq!(w.editor.project, blocked);
        assert!(!w.editor.can_undo());
    }
    w.editor.replace_project(original.clone(), None);
    w.editor.style.layer = layer;
    w.editor.set_tool(Tool::Pencil);
    w.editor.paste_raster_image(image).unwrap();
    let doc = w.editor.scene();
    let pasted = &doc.objects[0];
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(pasted.layer, layer);
    assert_eq!(pasted.name, "Pasted image");
    assert!(!pasted.reference_only);
    assert_eq!((pasted.position, pasted.size), ([30., 31.], [4., 2.]));
    assert_eq!(w.editor.style.layer, layer);
    assert_eq!(w.editor.selection, [pasted.id]);
    assert!(w.editor.tool == Tool::Select);
    assert_eq!(doc.hit_test([31., 31.5], 0), Some(pasted.id));
    let exported = renderer::render_for_export(doc, 0, 1.).unwrap();
    assert_eq!(exported.pixel(31, 31).unwrap().alpha(), 255);
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("Pasted.svg");
    project_io::export_svg(&svg, doc, 0, false).unwrap();
    assert!(project_io::import_svg(&svg).unwrap().objects[0]
        .image
        .is_some());
    let after = w.editor.project.clone();
    w.editor.restore(false);
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
    w.editor.restore(true);
    assert_eq!(w.editor.project, after);
}

#[test]
fn reference_import_keeps_the_active_layer_and_held_drawing_and_undoes_as_one_edit() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Pose board.png");
    let mut pixels = renderer::Pixmap::new(128, 32).unwrap();
    pixels
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .fill([30, 20, 10, 255]);
    pixels.save_png(&path).unwrap();

    let mut w = EditorWindow::with_artboard(egui::vec2(850., 670.), [64, 64]);
    let mut original = w.editor.project.clone();
    let clip = &mut original.clips[0];
    clip.set_length(20).unwrap();
    let doc = clip.drawing_at_mut(0);
    doc.layers[0].name = "Reference".into(); // Existing artwork can use this name.
    let art = doc.add_rectangle([10.; 2], [5.; 2], [255; 4]);
    clip.cels.insert(8, Arc::clone(&clip.cels[&0]));
    original.add_clip("Other").unwrap();
    w.editor.replace_project(original.clone(), None);
    w.editor.timeline.frame = 5;
    w.editor.selection = vec![art];
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.frame(vec![]);
    w.click("File");
    w.control("Import Reference Image…");

    w.editor
        .import_reference_from(&dir.path().join("missing.png"));
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
    w.editor.import_reference_from(&path);
    assert!(w.editor.can_undo(), "{}", w.editor.status);
    assert_eq!(w.editor.selection, [art]);
    assert_eq!(w.editor.style.layer, 1);
    let doc = w.editor.scene();
    let guide = &doc.layers[0];
    assert_eq!(guide.name, "Reference 2");
    assert!(guide.visible && guide.locked);
    let guide_id = guide.id;
    let image = doc.paint_order()[0];
    assert_eq!((image.layer, image.name.as_str()), (guide_id, "Pose board"));
    assert_eq!((image.position, image.size), ([0., 24.], [64., 16.]));
    assert!(image.reference_only);
    assert_eq!(doc.hit_test([1., 25.], 5), None);
    let clip = &w.editor.project.clips[0];
    assert_eq!(clip.cels.len(), 2);
    assert_eq!(clip.drawing_at(0), doc);
    let later = clip.drawing_at(8);
    assert_eq!(later.layers, doc.layers);
    assert_eq!(
        later.objects.iter().find(|o| o.id == art),
        original.clips[0].drawing_at(8).objects.first()
    );
    assert!(later
        .objects
        .iter()
        .any(|o| o.layer == guide_id && o.reference_only));
    assert_eq!(clip.layer_exposure(8, 1).0, 8);
    assert_eq!(clip.layer_exposure(8, guide_id), (0, 20));
    assert_eq!(w.editor.project.clips[1], original.clips[1]);
    let mut blank = clip.clone();
    blank.blank_layer_frame(5, 1).unwrap();
    assert_eq!(blank.drawing_at(5).objects.len(), 1);
    assert!(blank.drawing_at(5).objects[0].reference_only);

    let imported = w.editor.project.clone();
    w.editor.restore(false);
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
    w.editor.restore(true);
    assert_eq!(w.editor.project, imported);
    w.editor.import_reference_from(&path);
    assert_eq!(w.editor.scene().layers.len(), 2);
    let imported_images: Vec<_> = w
        .editor
        .scene()
        .objects
        .iter()
        .filter(|o| o.layer == guide_id)
        .filter_map(|o| o.image.as_ref())
        .collect();
    assert_eq!(imported_images.len(), 2);
    assert!(Arc::ptr_eq(imported_images[0], imported_images[1]));
    std::fs::remove_file(&path).unwrap();
    let saved = dir.path().join("Scene.vanim");
    assert!(w.editor.save_to(saved.clone(), false));
    assert_eq!(
        project_io::load_animation(&saved).unwrap(),
        w.editor.project
    );
}

#[test]
fn native_file_drop_validates_before_switching_and_updates_recent_only_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("Dropped.VANIM");
    let other = dir.path().join("Other.vanim");
    let damaged = dir.path().join("Damaged.vanim");
    let svg = dir.path().join("drawing.svg");
    project_io::save_animation(&good, &AnimationProject::default(), false).unwrap();
    std::fs::write(&damaged, "invalid project").unwrap();
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    let drop_at_window = |w: &mut EditorWindow, paths: &[&Path]| {
        w.frame_with_input(egui::RawInput {
            dropped_files: paths
                .iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path.to_path_buf()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        finish_opening_files(w);
    };
    drop_at_window(&mut w, &[&svg, &good]);
    assert_eq!(w.editor.path.as_ref(), Some(&good));
    w.editor.project.rename_clip(0, "Unsaved edits").unwrap();
    w.editor.finish_edit();
    for (paths, message) in [
        (vec![svg.as_path()], ".vanim"),
        (vec![damaged.as_path()], "Could not open"),
    ] {
        drop_at_window(&mut w, &paths);
        assert!(w.editor.status.contains(message), "{}", w.editor.status);
        assert_eq!(w.editor.project.clips[0].name, "Unsaved edits");
        assert!(w.editor.can_undo());
        assert_eq!(
            w.editor.recent_projects.as_slice(),
            std::slice::from_ref(&good)
        );
    }
    let previous = w.editor.active_document_tab();
    let count = w.editor.document_tab_count();
    project_io::save_animation(&other, &AnimationProject::default(), false).unwrap();
    drop_at_window(&mut w, &[&good, &svg, &other, &damaged]);
    assert_eq!(w.editor.path.as_ref(), Some(&other));
    assert_eq!(w.editor.document_tab_count(), count + 1);
    assert_eq!(w.editor.recent_projects, [other.clone(), good.clone()]);
    assert!(
        w.editor.status.contains("Damaged.vanim"),
        "{}",
        w.editor.status
    );
    assert!(!w.editor.can_undo());
    assert_eq!(
        project_io::load_animation(&good).unwrap(),
        AnimationProject::default()
    );
    std::fs::remove_file(&good).unwrap();
    drop_at_window(&mut w, &[&good]);
    assert_eq!(w.editor.active_document_tab(), previous);
    assert_eq!(w.editor.project.clips[0].name, "Unsaved edits");
    assert!(w.editor.can_undo());
    assert!(!good.exists());
}

#[test]
fn startup_and_file_new_use_the_default_artboard_and_open_keeps_the_saved_size() {
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    let dimensions = |editor: &Editor| [editor.scene().width, editor.scene().height];
    assert_eq!(dimensions(&w.editor), [1000; 2]);

    let lantern_walk =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    w.editor.open_project(lantern_walk);
    assert_eq!(dimensions(&w.editor), [128; 2]);
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.frame(vec![]);
    w.click("File");
    w.click("New");
    assert_eq!(dimensions(&w.editor), [1000; 2]);
    assert!(w.editor.scene().objects.is_empty());
    assert!(!w.editor.can_undo());
}

#[test]
fn recent_menu_reopens_projects_and_removes_missing_entries_without_discarding_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    let a = dir.path().join("Walk.vanim");
    let b = dir.path().join("Turn.vanim");
    let mut p = AnimationProject::default();
    project_io::save_animation(&a, &p, false).unwrap();
    p.rename_clip(0, "Turn").unwrap();
    project_io::save_animation(&b, &p, false).unwrap();
    w.editor.open_project(a.clone());
    w.editor.open_project(b.clone());
    let turn = w.editor.active_document_tab();
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.frame(vec![]);
    w.click("File");
    w.click("Open Recent");
    w.click("Walk.vanim");
    finish_opening_files(&mut w);
    assert_eq!(w.editor.path.as_ref(), Some(&a));
    assert_eq!(w.editor.recent_projects, [a.clone(), b.clone()]);
    w.editor.close_document_tab(turn);
    std::fs::remove_file(&b).unwrap();
    w.editor.project.rename_clip(0, "Unsaved").unwrap();
    w.editor.finish_edit();
    w.click("File");
    w.click("Open Recent");
    w.click("Turn.vanim");
    finish_opening_files(&mut w);
    assert_eq!(w.editor.project.clips[0].name, "Unsaved");
    assert_eq!(w.editor.recent_projects, [a]);
    assert!(w.editor.status.contains("Could not open"));
}

#[test]
fn artboard_dialog_updates_every_clip_and_undoes_as_one_edit() {
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    let mut p = AnimationProject::default();
    p.clips[0]
        .drawing_at_mut(0)
        .add_rectangle([27., 4.], [8.; 2], [255; 4]);
    p.clips[0].duplicate_frame(0).unwrap();
    p.duplicate_clip(0, "Walk").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Resized.vanim");
    w.editor.replace_project(p.clone(), Some(path.clone()));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.frame(vec![]);
    w.click("File");
    w.click("Artboard size…");
    let input = w.field_on_row("Width", " px");
    w.drag(input, egui::vec2(32., 0.));
    let width = w.editor.artboard_size.unwrap()[0];
    let old = p.clips[0].drawing_at(0);
    assert!(width > old.width);
    w.click("Apply");
    let resized = w.editor.project.clone();
    for (before, after) in p.clips.iter().zip(&resized.clips) {
        for (frame, cel) in &after.cels {
            assert_eq!([cel.width, cel.height], [width, old.height]);
            assert_eq!(cel.objects, before.cels[frame].objects);
        }
    }
    w.editor.restore(false);
    assert_eq!(w.editor.project, p);
    assert!(!w.editor.can_undo());
    w.editor.restore(true);
    assert_eq!(w.editor.project, resized);
    assert!(w.editor.save(false));
    assert_eq!(project_io::load_animation(&path).unwrap(), resized);
    w.click("File");
    w.click("Artboard size…");
    w.click("Cancel");
    assert_eq!(w.editor.project, resized);
}
