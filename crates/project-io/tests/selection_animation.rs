use document_core::{AnimationProject, Document, ExportPreset};
use project_io::{
    export_selection_animation, selection_animation_dimensions, SelectionRange, SpriteOutput,
};
use serde_json::{json, Value};
use std::{fs, io::ErrorKind, path::Path, sync::Arc};
use tiny_skia::{IntRect, Pixmap};

fn artwork() -> AnimationProject {
    let mut first = Document {
        frames: 6,
        fps: 18,
        ..Default::default()
    };
    first.layers[0].name = "Rig".into();
    let blank = first.clone();
    first.add_rectangle([-5., 0.], [2., 2.], [255, 0, 0, 255]);
    first.objects[0].name = "Limb".into();
    first.objects[0].group = Some(6);
    first.objects[0]
        .keys
        .extend([(0, [-5., 0.]), (1, [-3., 0.])]);
    first.add_rectangle([-5., 1.], [1., 1.], [0, 0, 255, 255]);
    first.objects[1].group = Some(6);
    let mut project = AnimationProject::from_document(first);
    let clip = &mut project.clips[0];
    clip.origin = [2.5, 3.25];
    clip.events.insert(0, vec!["Excluded".into()]);
    clip.events
        .insert(2, vec!["Foot left".into(), "Dust".into()]);
    clip.cels.insert(2, Arc::new(blank.clone()));
    let mut next = blank;
    next.add_rectangle([-100., -100.], [10., 10.], [255; 4]);
    next.objects[0].name = "Other".into(); // Reuses the reference limb's ID.
    next.add_rectangle([7., 2.], [2., 2.], [0, 255, 0, 255]);
    next.objects[1].name = "Limb".into();
    next.objects[1].group = Some(27);
    next.add_rectangle([9., 2.], [1., 1.], [0, 0, 255, 255]);
    next.objects[2].group = Some(27);
    clip.cels.insert(3, Arc::new(next));
    project
}

fn range(reference: u32, first: u32, last: u32) -> SelectionRange {
    SelectionRange {
        reference,
        first,
        last,
    }
}

fn metadata(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join("sheet.json")).unwrap()).unwrap()
}

#[test]
fn measuring_an_animated_selection_requires_a_first_drawing() {
    let mut project = artwork();
    let clip = &mut project.clips[0];
    clip.cels.remove(&0);
    let error = selection_animation_dimensions(
        clip,
        &[1],
        range(0, 0, 4),
        &project.preset,
        SpriteOutput::Frames,
        |_, _| true,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.to_string().contains("first frame"));
}

#[test]
fn sheets_and_frames_keep_groups_empty_exposures_and_trimmed_game_positions() {
    let dir = tempfile::tempdir().unwrap();
    let project = artwork();
    let clip = &project.clips[0];
    let preset = ExportPreset {
        scale: 2,
        columns: 3,
        margin: 1,
        padding: 2,
        ..Default::default()
    };
    let out = dir.path().join("assets");
    let written = export_selection_animation(
        &out,
        clip,
        &[1],
        range(0, 1, 4),
        &preset,
        SpriteOutput::SheetAndFrames,
        |_, _| true,
    )
    .unwrap();
    assert_eq!(written.origin, [-10, 0]);
    assert_eq!(written.size, [30, 8]);
    assert_eq!(written.pivot, [15., 6.5]);
    let data = metadata(&out);
    assert_eq!(data["source_start"], 1);
    assert_eq!(data["source_end"], 4);
    assert_eq!(data["fps"], 18);
    assert_eq!(data["looping"], clip.looping);
    assert_eq!(
        data["events"],
        json!([{"frame":1,"source_frame":2,"name":"Foot left"},{"frame":1,"source_frame":2,"name":"Dust"}])
    );
    let atlas = Pixmap::load_png(out.join("sheet.png")).unwrap();
    for (index, f) in data["frames"].as_array().unwrap().iter().enumerate() {
        let png = Pixmap::load_png(out.join(f["file"].as_str().unwrap())).unwrap();
        assert_eq!([png.width(), png.height()], written.size);
        assert_eq!(f["source_frame"], index + 1);
        assert_eq!(f["duration_ticks"], 1);
        assert_eq!(f["pivot"], json!(written.pivot));
        let pos = IntRect::from_xywh(
            f["x"].as_i64().unwrap() as i32,
            f["y"].as_i64().unwrap() as i32,
            30,
            8,
        )
        .unwrap();
        assert!(atlas.clone_rect(pos).unwrap().data() == png.data());
        let (offset, size) = match index {
            0 => {
                assert_eq!(png.pixel(4, 0).unwrap().red(), 255);
                assert_eq!(png.pixel(0, 2).unwrap().blue(), 255);
                ([0, 0], [8, 4])
            }
            1 => {
                assert!(png.pixels().iter().all(|pixel| pixel.alpha() == 0));
                ([0, 0], [0, 0])
            }
            _ => {
                assert_eq!(png.pixel(24, 4).unwrap().green(), 255);
                assert_eq!(png.pixel(29, 4).unwrap().blue(), 255);
                assert_eq!(png.pixel(0, 2).unwrap().alpha(), 0);
                ([24, 4], [6, 4])
            }
        };
        assert_eq!(f["trim_offset"], json!(offset));
        assert_eq!(f["trim_size"], json!(size));
    }
    assert_eq!(written.receipt.files().count(), 6);
}

#[test]
fn unnamed_held_body_and_named_redrawn_effects_match_independently() {
    let mut doc = Document::default();
    doc.layers.push(document_core::Layer {
        id: 2,
        name: "Effects".into(),
        ..Default::default()
    });
    let body = doc.add_rectangle([0., 0.], [2., 2.], [255, 0, 0, 255]);
    doc.objects[0].name.clear();
    let fx = doc.add_shape(
        [3., 0.],
        [2., 2.],
        [0, 255, 0, 255],
        document_core::Shape::Rectangle,
        2,
    );
    doc.objects[1].name = "Spark".into();
    let mut p = AnimationProject::from_document(doc);
    let clip = &mut p.clips[0];
    clip.blank_layer_frame(2, 2).unwrap();
    let before = clip.clone();
    let new = clip.drawing_at_mut(2);
    new.add_shape(
        [3., 0.],
        [2., 2.],
        [0, 0, 255, 255],
        document_core::Shape::Rectangle,
        2,
    );
    new.objects.last_mut().unwrap().name = "Spark".into();
    clip.sync_layer_edits(&before, 2).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("selected");
    export_selection_animation(
        &out,
        clip,
        &[body, fx],
        range(0, 0, 4),
        &p.preset,
        SpriteOutput::Frames,
        |_, _| true,
    )
    .unwrap();
    for i in 0..5 {
        let image = Pixmap::load_png(out.join(format!("frames/frame_{i:04}.png"))).unwrap();
        assert_eq!(image.pixel(0, 0).unwrap().red(), 255);
        let effect = image.pixel(3, 0).unwrap();
        assert_eq!(
            (effect.green(), effect.blue()),
            if i < 2 { (255, 0) } else { (0, 255) }
        );
    }
}

#[test]
fn name_collisions_and_invalid_ranges_never_publish_partial_results() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("export");
    for source in [false, true] {
        let mut p = artwork();
        let clip = &mut p.clips[0];
        let target_tick = if source { 0 } else { 3 };
        let doc = clip.drawing_at_mut(target_tick);
        let duplicate = doc.add_rectangle([0., 0.], [1., 1.], [255; 4]);
        doc.objects.last_mut().unwrap().name = "Limb".into();
        let err = export_selection_animation(
            &out,
            clip,
            &[1],
            range(0, 0, 4),
            &p.preset,
            SpriteOutput::Sheet,
            |_, _| true,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("Ambiguous")
                && err.contains("Limb")
                && err.contains("Rig")
                && err.contains(&format!("drawing {target_tick}")),
            "{err}"
        );
        assert!(!out.exists());
        // An explicit ID within a single exposure is unambiguous, even with duplicate names.
        export_selection_animation(
            &dir.path().join(format!("one-{target_tick}")),
            clip,
            &[duplicate],
            range(target_tick, target_tick, target_tick),
            &p.preset,
            SpriteOutput::Sheet,
            |_, _| true,
        )
        .unwrap();
    }
    let p = artwork();
    for r in [range(6, 0, 1), range(0, 4, 3), range(1, 0, 6)] {
        assert!(export_selection_animation(
            &out,
            &p.clips[0],
            &[1],
            r,
            &p.preset,
            SpriteOutput::Frames,
            |_, _| true
        )
        .is_err());
    }
    let err = export_selection_animation(
        &out,
        &p.clips[0],
        &[1],
        range(0, 0, 4),
        &p.preset,
        SpriteOutput::Frames,
        |done, _| done < 7,
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Interrupted);
    assert!(!out.exists());
    assert!(fs::read_dir(dir.path()).unwrap().all(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("one-")));
    fs::create_dir(&out).unwrap();
    fs::write(out.join("mine.png"), b"mine").unwrap();
    assert_eq!(
        export_selection_animation(
            &out,
            &p.clips[0],
            &[1],
            range(0, 0, 4),
            &p.preset,
            SpriteOutput::Frames,
            |_, _| true
        )
        .unwrap_err()
        .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(out.join("mine.png")).unwrap(), b"mine");
}

#[test]
fn all_unmatched_hidden_and_unnamed_frames_remain_transparent() {
    let dir = tempfile::tempdir().unwrap();
    for scenario in ["renamed_layer", "unnamed", "hidden", "empty"] {
        let mut p = artwork();
        let clip = &mut p.clips[0];
        match scenario {
            "renamed_layer" => clip.drawing_at_mut(3).layers[0].name = "Elsewhere".into(),
            "unnamed" => {
                clip.drawing_at_mut(0).objects[0].name.clear();
                clip.drawing_at_mut(3).objects[1].name.clear();
            }
            "hidden" => clip.drawing_at_mut(3).layers[0].visible = false,
            _ => {}
        }
        let r = if scenario == "empty" {
            range(0, 2, 2)
        } else {
            range(0, 3, 5)
        };
        let out = dir.path().join(scenario);
        let res = export_selection_animation(
            &out,
            clip,
            &[1],
            r,
            &p.preset,
            SpriteOutput::Frames,
            |_, _| true,
        )
        .unwrap();
        assert_eq!(res.size, [1, 1]);
        assert_eq!(res.origin, [0, 0]);
        assert_eq!(res.pivot, clip.origin);
        let data = metadata(&out);
        assert!(data["image"].is_null());
        for frame in data["frames"].as_array().unwrap() {
            assert_eq!(frame["trim_size"], json!([0, 0]));
            let image = Pixmap::load_png(out.join(frame["file"].as_str().unwrap())).unwrap();
            assert_eq!(image.pixel(0, 0).unwrap().alpha(), 0);
        }
    }
}

#[test]
fn thousand_tick_held_selection_reuses_geometry_while_unselected_artwork_moves() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document {
        frames: 1000,
        ..Default::default()
    };
    let id = doc.add_rectangle([-300., -300.], [2., 2.], [255; 4]);
    doc.add_rectangle([1., 1.], [6., 6.], [24; 4]);
    doc.objects[1]
        .keys
        .extend([(0, [1., 1.]), (999, [100., 100.])]);
    let p = AnimationProject::from_document(doc);
    let mut calls = Vec::new();
    let out = dir.path().join("sprites");
    let res = export_selection_animation(
        &out,
        &p.clips[0],
        &[id],
        range(500, 0, 999),
        &ExportPreset {
            columns: 32,
            ..Default::default()
        },
        SpriteOutput::Sheet,
        |done, total| {
            assert_eq!(total, 2000);
            calls.push(done);
            true
        },
    )
    .unwrap();
    // The measuring pass visits the held selection once for the entire clip.
    assert_eq!(&calls[..2], &[0, 1000]);
    let data = metadata(&out);
    assert_eq!(data["frames"].as_array().unwrap().len(), 1000);
    assert_eq!(res.origin, [-300, -300]);
    assert_eq!(res.size, [2, 2]);
    assert_eq!(res.receipt.files().count(), 2);
    assert_eq!(data["frames"][999]["trim_size"], json!([2, 2]));
}
