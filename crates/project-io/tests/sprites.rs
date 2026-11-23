use document_core::{AnimationProject, Document, ExportPreset};
use project_io::{export_sprite_bundle, export_sprite_output, sheet_layout, SpriteOutput};
use std::fs;
use tiny_skia::Pixmap;

fn artwork() -> AnimationProject {
    let mut doc = Document {
        width: 4,
        height: 3,
        frames: 3,
        fps: 10,
        ..Document::default()
    };
    doc.add_rectangle([1., 1.], [1., 1.], [255, 0, 0, 255]);
    let mut p = AnimationProject::from_document(doc);
    let c = &mut p.clips[0];
    let mut last = c.drawing_at(0).clone();
    last.objects[0].fill = [0, 200, 0, 255];
    last.objects[0].position = [2., 0.];
    c.cels.insert(2, std::sync::Arc::new(last));
    c.origin = [1.5, 2.5];
    c.name = "Walk".into();
    p.preset = ExportPreset {
        columns: 2,
        scale: 2,
        margin: 1,
        padding: 3,
        ..ExportPreset::default()
    };
    p
}

#[test]
#[ignore = "release timing probe; writes 1,100 individual PNGs and full grid and packed sheets"]
fn held_animation_export_timing() {
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/lantern-keeper.vanim");
    let project = project_io::load_animation(&example).unwrap();
    for ticks in [100, 1000] {
        let mut doc = project.clips[0].drawing_at(0).clone();
        doc.frames = ticks;
        doc.fps = 24;
        let mut project = AnimationProject::from_document(doc);
        let clip = &mut project.clips[0];
        clip.origin = [32.25, 106.5];
        clip.events
            .insert(ticks / 2, vec!["step".into(), "turn".into()]);
        let mut preset = ExportPreset {
            columns: if ticks == 100 { 10 } else { 32 },
            ..Default::default()
        };
        preset.packed.enabled = true;
        preset.packed.max_size = 1024;
        preset.packed.allow_rotation = true;
        for mode in [
            SpriteOutput::Frames,
            SpriteOutput::Sheet,
            SpriteOutput::Packed,
        ] {
            let root = tempfile::tempdir().unwrap();
            let start = std::time::Instant::now();
            export_sprite_output(
                &root.path().join("export"),
                clip,
                &preset,
                mode,
                None,
                |_, _| true,
            )
            .unwrap();
            eprintln!(
                "{ticks} held ticks, {mode:?}: {:.1} ms",
                start.elapsed().as_secs_f64() * 1000.
            );
        }
    }
}

#[test]
fn sheet_and_individual_ticks_share_geometry_pivots_and_holds() {
    let p = artwork();
    let dest = tempfile::tempdir().unwrap();
    let dir = dest.path().join("export");
    export_sprite_bundle(&dir, &p.clips[0], &p.preset, true).unwrap();
    let atlas = Pixmap::load_png(dir.join("sheet.png")).unwrap();
    assert_eq!((atlas.width(), atlas.height()), (21, 17));
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("sheet.json")).unwrap()).unwrap();
    assert_eq!(meta["fps"], 10);
    assert_eq!(meta["name"], "Walk");
    assert_eq!(meta["looping"], true);
    for (i, (x, y)) in [(1, 1), (12, 1), (1, 10)].into_iter().enumerate() {
        let frame = &meta["frames"][i];
        assert_eq!(frame["x"], x);
        assert_eq!(frame["y"], y);
        assert_eq!(frame["pivot"], serde_json::json!([3., 5.]));
        assert_eq!(frame["duration_ticks"], 1);
        let png = Pixmap::load_png(dir.join(format!("frames/frame_{i:04}.png"))).unwrap();
        for cy in 0..6 {
            for cx in 0..8 {
                assert_eq!(png.pixel(cx, cy), atlas.pixel(cx + x, cy + y));
            }
        }
    }
    assert_eq!(atlas.pixel(3, 3).unwrap().red(), 255);
    assert_eq!(atlas.pixel(14, 3).unwrap().red(), 255);
    assert_eq!(atlas.pixel(5, 10).unwrap().green(), 200);
    assert_eq!(atlas.pixel(14, 12).unwrap().alpha(), 0);
    assert_eq!(atlas.pixel(0, 0).unwrap().alpha(), 0);
}

#[test]
fn repeat_export_preserves_engine_files_rejects_edits_and_rolls_back_conflicts() {
    let mut p = artwork();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("exports");
    let receipt = export_sprite_output(
        &dir,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Sheet,
        None,
        |_, _| true,
    )
    .unwrap();
    fs::write(dir.join("sheet.png.import"), "godot import settings").unwrap();
    p.clips[0].origin = [0., 0.];
    let updated = export_sprite_output(
        &dir,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Sheet,
        Some(&receipt),
        |_, _| true,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("sheet.png.import")).unwrap(),
        "godot import settings"
    );
    let meta = fs::read(dir.join("sheet.json")).unwrap();
    // New files encounter a user-owned name after an old bundle was backed up.
    fs::create_dir(dir.join("frames")).unwrap();
    fs::write(dir.join("frames/frame_0001.png"), "mine").unwrap();
    assert!(export_sprite_output(
        &dir,
        &p.clips[0],
        &p.preset,
        SpriteOutput::SheetAndFrames,
        Some(&updated),
        |_, _| true
    )
    .is_err());
    assert_eq!(meta, fs::read(dir.join("sheet.json")).unwrap());
    assert_eq!(
        fs::read_to_string(dir.join("frames/frame_0001.png")).unwrap(),
        "mine"
    );
    fs::write(dir.join("sheet.png"), "edited").unwrap();
    assert!(export_sprite_output(
        &dir,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Sheet,
        Some(&updated),
        |_, _| true
    )
    .is_err());
    assert_eq!(fs::read_to_string(dir.join("sheet.png")).unwrap(), "edited");
}

#[test]
fn oversized_sheet_and_cancelled_job_create_no_destination() {
    let mut p = artwork();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("result");
    assert!(export_sprite_output(
        &out,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Frames,
        None,
        |n, _| n < 2
    )
    .is_err());
    assert!(!out.exists());
    p.preset.columns = 4096;
    p.preset.margin = 4096;
    assert!(sheet_layout(&p.clips[0], &p.preset).is_err());
    export_sprite_output(
        &out,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Frames,
        None,
        |_, _| true,
    )
    .unwrap();
    assert!(out.join("frame_0002.png").exists());
}
