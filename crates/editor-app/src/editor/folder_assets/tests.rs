use super::*;
use document_core::{Document, RasterImage};
use std::time::{Duration, Instant};

impl Editor {
    pub(in crate::editor) fn place_folder_asset(
        &mut self,
        ctx: &egui::Context,
        path: std::path::PathBuf,
        clip_id: Option<u64>,
        point: [f32; 2],
    ) {
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.place_asset(ctx, AssetSource::File { path, clip_id }, label, point);
    }
}

fn complete(editor: &mut Editor, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while editor.folder_assets.is_pending() || editor.file_batch.is_some() {
        assert!(Instant::now() < deadline, "folder placement timed out");
        editor.poll_folder_placement(ctx);
        editor.poll_file_batch(ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn project_clip_drop_rejects_a_cycle_or_a_clip_from_another_document() {
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    let parent = editor.project.add_clip("Scene").unwrap();
    let active = editor.project.clips[0].id;
    editor
        .project
        .place_clip(active, parent, 0, 1, [0.; 2])
        .unwrap();
    editor.capture_baseline();
    let before = editor.project.clone();
    let id = before.clips[parent].id;
    let session = before.clips[parent].session_id();
    let other_session = AnimationProject::default().clips[0].session_id();
    for (session, reason) in [
        (session, "cycle"),
        (other_session, "no longer in this project"),
    ] {
        editor.place_asset(
            &ctx,
            AssetSource::ProjectClip { id, session },
            "Scene".into(),
            [10., 10.],
        );
        complete(&mut editor, &ctx);
        assert_eq!(editor.project, before);
        assert!(editor.status.contains(reason), "{}", editor.status);
        assert!(!editor.history.can_undo());
    }
}

#[test]
fn placing_a_clip_transfers_only_its_dependencies_and_preserves_its_viewport() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("Fox.vanim");
    let mut src = AnimationProject::from_document(Document::default());
    src.rename_clip(0, "Run").unwrap();
    let leg = src.add_clip("Leg").unwrap();
    src.add_clip("Unused").unwrap();
    let boot = src.clips[leg]
        .drawing_at_mut(0)
        .add_rectangle([1., 1.], [4., 4.], [100; 4]);
    src.convert_to_symbol(leg, 0, &[boot], "Boot").unwrap();
    src.place_clip(src.clips[leg].id, 0, 0, 1, [2., 3.])
        .unwrap();
    project_io::save_animation(&path, &src, false).unwrap();
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    let collision =
        editor.project.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([0., 0.], [3., 3.], [7; 4]);
    editor
        .project
        .convert_to_symbol(0, 0, &[collision], "Boot")
        .unwrap();
    editor.capture_baseline();
    let original = editor.project.clone();
    editor.place_folder_asset(&ctx, path.clone(), Some(src.clips[0].id), [300., 200.]);
    complete(&mut editor, &ctx);
    assert_eq!(editor.project.clips.len(), 3, "{}", editor.status);
    assert_eq!(editor.project.symbols.len(), 2);
    let placed = editor.scene().objects.last().unwrap();
    assert_eq!(editor.selection, vec![placed.id]);
    assert_eq!(placed.position, [236., 136.]);
    assert_eq!(placed.size, [128., 128.]);
    let instance = placed.clip_instance.as_ref().unwrap();
    assert_eq!(instance.source_size, Some([128, 128]));
    let leg = &instance.shared_clip().unwrap().drawing_at(0).objects[0];
    assert_eq!(
        leg.clip_instance.as_ref().unwrap().source_size,
        Some([128, 128])
    );
    assert_eq!(
        leg.instance_at(0).unwrap().0.objects[0]
            .symbol
            .as_ref()
            .unwrap()
            .artwork()
            .unwrap()
            .objects[0]
            .fill,
        [100; 4]
    );
    let first = editor.project.clone();
    assert!(editor.history.undo(&mut editor.project));
    assert_eq!(original, editor.project);
    assert!(!editor.history.can_undo());
    assert!(editor.history.redo(&mut editor.project));
    assert_eq!(first, editor.project);
    let host = tmp.path().join("Forest.vanim");
    project_io::save_animation(&host, &first, false).unwrap();
    editor.open_project(host);
    // File-local numeric IDs have already collided. A repeated import reuses
    // the same dependencies even after saving, closing and reopening the scene.
    editor.place_folder_asset(&ctx, path, Some(src.clips[0].id), [70., 80.]);
    complete(&mut editor, &ctx);
    assert_eq!(editor.project.clips.len(), 3, "{}", editor.status);
    assert_eq!(editor.project.symbols.len(), 2);
    assert_eq!(editor.scene().objects.len(), 3);
    assert_eq!(
        editor.scene().objects[1]
            .clip_instance
            .as_ref()
            .unwrap()
            .clip_id,
        editor.scene().objects[2]
            .clip_instance
            .as_ref()
            .unwrap()
            .clip_id
    );
}

#[test]
fn folder_svg_and_png_are_editable_and_embedded_at_the_drop_point() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    let host = editor.project.add_clip("World").unwrap();
    editor
        .project
        .place_clip(editor.project.clips[0].id, host, 0, 1, [0., 0.])
        .unwrap();
    editor.capture_baseline();
    let svg = dir.path().join("Moss.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="8" height="7" fill="#135933"/></svg>"##).unwrap();
    editor.place_folder_asset(&ctx, svg, None, [60., 50.]);
    complete(&mut editor, &ctx);
    let placed = &editor.scene().objects[0];
    assert_eq!(placed.position, [48., 44.]);
    assert_eq!(placed.embedded_artwork.as_ref().unwrap().objects.len(), 1);
    let png = dir.path().join("Leaf.png");
    let bytes = renderer::Pixmap::new(2, 2).unwrap().encode_png().unwrap();
    let image = RasterImage::from_bytes(&bytes).unwrap();
    std::fs::write(&png, bytes).unwrap();
    editor.place_folder_asset(&ctx, png, None, [60., 50.]);
    complete(&mut editor, &ctx);
    let placed = &editor.scene().objects[1];
    assert_eq!(placed.position, [59., 49.]);
    assert_eq!(placed.image.as_deref(), Some(&image));
    assert!(!placed.reference_only);
    let placed_in_world = &editor.project.clips[host].drawing_at(0).objects[0];
    assert_eq!(placed_in_world.instance_at(0).unwrap().0.objects.len(), 2);
    let saved = dir.path().join("art.vanim");
    project_io::save_animation(&saved, &editor.project, false).unwrap();
    assert_eq!(project_io::load_animation(&saved).unwrap(), editor.project);
}

#[test]
fn folder_placement_follows_the_target_layers_hold_through_other_drawings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Leaf.png");
    std::fs::write(
        &path,
        renderer::Pixmap::new(2, 2).unwrap().encode_png().unwrap(),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    let clip = &mut editor.project.clips[0];
    clip.set_length(12).unwrap();
    let effects = commands::add_layer(clip.drawing_at_mut(0));
    clip.duplicate_layer_frame(4, effects).unwrap();
    clip.duplicate_layer_frame(9, 1).unwrap();
    editor.capture_baseline();
    editor.timeline.frame = 6;
    editor.place_folder_asset(&ctx, path, None, [60., 50.]);
    complete(&mut editor, &ctx);
    let id = editor.selection[0];
    let has_image = |p: &AnimationProject, tick| {
        p.clips[0]
            .drawing_at(tick)
            .objects
            .iter()
            .any(|o| o.id == id && o.image.is_some())
    };
    assert!(has_image(&editor.project, 0));
    assert!(has_image(&editor.project, 8));
    assert!(!has_image(&editor.project, 9));
    assert!(editor.history.undo(&mut editor.project));
    assert!(!has_image(&editor.project, 0));
    assert!(!has_image(&editor.project, 8));
}

#[test]
fn slow_import_marker_stays_at_its_destination_until_the_result_is_applied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Tile.png");
    std::fs::write(
        &path,
        renderer::Pixmap::new(2, 2).unwrap().encode_png().unwrap(),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    let point = [-18., 41.];
    let input = |time| egui::RawInput {
        time: Some(time),
        ..Default::default()
    };
    let _ = ctx.run(input(1.), |ctx| {
        editor.place_folder_asset(ctx, path.clone(), None, point);
        assert_eq!(editor.folder_assets.loading_point(ctx, 0, 0, 1), None);
    });
    let _ = ctx.run(input(1.2), |ctx| {
        assert_eq!(
            editor.folder_assets.loading_point(ctx, 0, 0, 1),
            Some(point)
        );
        assert_eq!(editor.folder_assets.loading_point(ctx, 0, 1, 1), None);
        assert_eq!(editor.folder_assets.loading_point(ctx, 0, 0, 2), None);
    });
    complete(&mut editor, &ctx);
    assert_eq!(editor.folder_assets.loading_point(&ctx, 0, 0, 1), None);
    assert_eq!(editor.scene().objects.len(), 1);
}

#[test]
fn interrupted_or_invalid_placement_does_not_change_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Fox.vanim");
    let initial = AnimationProject::default();
    project_io::save_animation(&path, &initial, false).unwrap();
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    editor.place_folder_asset(&ctx, path.clone(), None, [4., 5.]);
    editor.timeline.frame = 1;
    complete(&mut editor, &ctx);
    assert_eq!(editor.project, initial);
    assert!(!editor.history.can_undo());
    editor.place_folder_asset(&ctx, path.clone(), None, [4., 5.]);
    editor.project.clips[0]
        .drawing_at_mut(0)
        .add_rectangle([1., 1.], [3., 3.], [5; 4]);
    let edited = editor.project.clone();
    complete(&mut editor, &ctx);
    assert_eq!(editor.project, edited);
    assert!(editor.status.contains("drawing changed"));
    editor.place_folder_asset(&ctx, path.clone(), Some(999), [4., 5.]);
    complete(&mut editor, &ctx);
    assert_eq!(editor.project, edited);
    assert!(editor.status.contains("no longer exists"));
    editor.place_folder_asset(&ctx, path.clone(), None, [4., 5.]);
    editor.replace_project(initial.clone(), None);
    assert!(!editor.folder_assets.is_pending());
    editor.open_paths(&ctx, vec![path.clone()]);
    complete(&mut editor, &ctx);
    assert_eq!(editor.path.as_deref(), Some(path.as_path()));
    assert_eq!(editor.project, initial);
}
