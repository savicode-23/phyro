use document_core::{AnimationProject, Document, ExportPreset};
use project_io::{export_packed_atlas, export_sprite_output, render_packed_preview, SpriteOutput};
use renderer::Pixmap;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

fn scene() -> AnimationProject {
    let mut doc = Document {
        width: 18,
        height: 14,
        frames: 6,
        ..Default::default()
    };
    let a = doc.add_rectangle([3., 2.], [5., 3.], [240, 20, 80, 255]);
    let mut p = AnimationProject::from_document(doc);
    let clip = &mut p.clips[0];
    clip.origin = [4.25, 9.5];
    clip.events.insert(4, vec!["step".into(), "dust".into()]);
    for (start, x, size, color) in [
        (2, 7., [5., 3.], [240, 20, 80, 255]),
        (3, 4., [2., 7.], [5, 220, 10, 255]),
        (4, 2., [8., 8.], [5, 7, 230, 255]),
        (5, 0., [1., 1.], [0; 4]),
    ] {
        let mut doc = clip.drawing_at(0).clone();
        let o = doc.objects.iter_mut().find(|o| o.id == a).unwrap();
        o.position = [x, 2.];
        o.size = size;
        o.fill = color;
        clip.cels.insert(start, Arc::new(doc));
    }
    p.preset.packed.enabled = true;
    p
}
fn metadata(path: &Path) -> Value {
    let bytes = std::fs::read(path.join("sheet.json")).unwrap();
    assert_eq!(std::fs::read(path.join("animation.phyro")).unwrap(), bytes);
    serde_json::from_slice(&bytes).unwrap()
}
fn n(frame: &Value, key: &str) -> u32 {
    frame[key].as_u64().unwrap() as u32
}

fn assert_frames(p: &AnimationProject, dest: &Path) -> Value {
    let meta = metadata(dest);
    let images: Vec<_> = meta["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Pixmap::load_png(dest.join(v.as_str().unwrap())).unwrap())
        .collect();
    assert!(!images.is_empty());
    assert_eq!(meta["image"], "sheet.png");
    assert_eq!(meta["format_version"], 2);
    assert_eq!(
        meta["events"],
        serde_json::json!([{"frame":4,"name":"step"},{"frame":4,"name":"dust"}])
    );
    for (i, record) in meta["frames"].as_array().unwrap().iter().enumerate() {
        let full = renderer::render_scaled(
            p.clips[0].drawing_at(i as u32),
            i as u32,
            p.preset.scale as f32,
        )
        .unwrap();
        let atlas = &images[n(record, "page") as usize];
        let (x, y, w, h) = (
            n(record, "x"),
            n(record, "y"),
            n(record, "w"),
            n(record, "h"),
        );
        let (left, top) = (n(record, "trim_x"), n(record, "trim_y"));
        assert_eq!(n(record, "source_w"), full.width());
        assert_eq!(n(record, "source_h"), full.height());
        assert_eq!(
            record["pivot"],
            serde_json::json!(p.clips[0].origin.map(|x| x * p.preset.scale as f32))
        );
        assert_eq!(n(record, "duration_ticks"), 1);
        assert!(x + w <= atlas.width() && y + h <= atlas.height());
        let rotated = record["rotated"].as_bool().unwrap();
        let (crop_w, crop_h) = if rotated { (h, w) } else { (w, h) };
        let bleed = p.preset.packed.extrude;
        for dy in 0..h + 2 * bleed {
            for dx in 0..w + 2 * bleed {
                let px = dx.saturating_sub(bleed).min(w - 1);
                let py = dy.saturating_sub(bleed).min(h - 1);
                let (sx, sy) = if rotated {
                    (left + py, top + crop_h - 1 - px)
                } else {
                    (left + px, top + py)
                };
                assert_eq!(
                    atlas.pixel(x - bleed + dx, y - bleed + dy),
                    full.pixel(sx, sy),
                    "extrusion for frame {i}"
                );
            }
        }
        for sy in 0..full.height() {
            for sx in 0..full.width() {
                if !(left..left + crop_w).contains(&sx) || !(top..top + crop_h).contains(&sy) {
                    assert_eq!(full.pixel(sx, sy).unwrap().alpha(), 0);
                }
            }
        }
    }
    meta
}

#[test]
fn trim_pack_extrude_and_deduplicate_preserve_all_source_pixels_and_timing() {
    let mut p = scene();
    p.preset.margin = 1;
    p.preset.padding = 2;
    p.preset.scale = 2;
    p.preset.packed.extrude = 2;
    p.preset.packed.max_size = 24;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("packed");
    export_packed_atlas(&dest, &p.clips[0], &p.preset).unwrap();
    let meta = assert_frames(&p, &dest);
    let frames = meta["frames"].as_array().unwrap();
    for i in [1, 2] {
        for key in ["page", "x", "y", "w", "h"] {
            assert_eq!(frames[0][key], frames[i][key]);
        }
    }
    assert_ne!(frames[0]["trim_x"], frames[2]["trim_x"]);
    assert_eq!(frames[5]["w"], 1);
    assert_eq!(frames[5]["h"], 1);
    assert!(meta["images"].as_array().unwrap().len() > 1);
    let (preview, count) =
        render_packed_preview(&p.clips[0], &p.preset, usize::MAX, |_, _| true).unwrap();
    assert_eq!(count, meta["images"].as_array().unwrap().len());
    let last = Pixmap::load_png(dest.join(meta["images"][count - 1].as_str().unwrap())).unwrap();
    assert_eq!(preview.data(), last.data());
}

#[test]
fn retain_full_frames_and_distinct_cells_or_deduplicate_at_author_option() {
    let mut p = scene();
    p.preset.packed.trim = false;
    p.preset.packed.deduplicate = false;
    p.preset.packed.extrude = 0;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("all");
    export_packed_atlas(&dest, &p.clips[0], &p.preset).unwrap();
    let meta = assert_frames(&p, &dest);
    let mut slots = std::collections::BTreeSet::new();
    for r in meta["frames"].as_array().unwrap() {
        assert_eq!(
            (n(r, "trim_x"), n(r, "trim_y"), n(r, "w"), n(r, "h")),
            (0, 0, 18, 14)
        );
        assert!(slots.insert((n(r, "page"), n(r, "x"), n(r, "y"))));
    }
}

#[test]
fn keyed_nested_drawings_advance_through_a_held_parent_in_every_output() {
    let mut p = scene();
    p.clips[0].cels.retain(|&frame, _| frame == 0);
    p.clips[0].drawing_at_mut(0).objects.clear();
    let child = p.add_clip("Nested").unwrap();
    p.clips[child].set_length(3).unwrap();
    let doc = p.clips[child].drawing_at_mut(0);
    doc.add_rectangle([1., 1.], [3., 2.], [200, 0, 0, 255]);
    doc.objects[0].keys.extend([(0, [1., 1.]), (1, [6., 1.])]);
    let mut blue = doc.clone();
    blue.objects[0].keys.clear();
    blue.objects[0].fill = [0, 0, 255, 255];
    p.clips[child].cels.insert(2, Arc::new(blue));
    p.place_clip(p.clips[child].id, 0, 0, 1, [1., 2.]).unwrap();

    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("packed");
    export_packed_atlas(&dest, &p.clips[0], &p.preset).unwrap();
    assert_frames(&p, &dest);

    let clip = &p.clips[0];
    let dest = temp.path().join("grid");
    export_sprite_output(
        &dest,
        clip,
        &p.preset,
        SpriteOutput::SheetAndFrames,
        None,
        |_, _| true,
    )
    .unwrap();
    let preview = project_io::render_sheet_preview(clip, &p.preset, |_, _| true).unwrap();
    let sheet = Pixmap::load_png(dest.join("sheet.png")).unwrap();
    assert_eq!(sheet.data(), preview.data());
    let grid: Value =
        serde_json::from_slice(&std::fs::read(dest.join("sheet.json")).unwrap()).unwrap();
    let mut distinct = std::collections::BTreeSet::new();
    for (i, row) in grid["frames"].as_array().unwrap().iter().enumerate() {
        let expected = renderer::render(clip.drawing_at(i as u32), i as u32).unwrap();
        let file = Pixmap::load_png(dest.join(format!("frames/frame_{i:04}.png"))).unwrap();
        assert_eq!(file.data(), expected.data());
        for y in 0..expected.height() {
            for x in 0..expected.width() {
                assert_eq!(
                    sheet.pixel(n(row, "x") + x, n(row, "y") + y),
                    expected.pixel(x, y)
                );
            }
        }
        distinct.insert(expected.data().to_vec());
    }
    assert_eq!(distinct.len(), 3);
}

#[test]
fn oversized_frame_cancellation_and_repeat_publish_leave_outputs_consistent() {
    let mut p = scene();
    p.preset.packed.max_size = 2;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("atlas");
    assert!(export_packed_atlas(&dest, &p.clips[0], &p.preset).is_err());
    assert!(!dest.exists());
    p.preset.packed.max_size = 12;
    let old = export_sprite_output(
        &dest,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Packed,
        None,
        |_, _| true,
    )
    .unwrap();
    let original = metadata(&dest);
    std::fs::write(dest.join("keep.import"), b"local").unwrap();
    let error = export_sprite_output(
        &dest,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Packed,
        Some(&old),
        |done, total| done < total / 2,
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(metadata(&dest), original);
    p.preset.packed.max_size = 2048;
    export_sprite_output(
        &dest,
        &p.clips[0],
        &p.preset,
        SpriteOutput::Packed,
        Some(&old),
        |_, _| true,
    )
    .unwrap();
    assert_eq!(std::fs::read(dest.join("keep.import")).unwrap(), b"local");
    for path in original["images"].as_array().unwrap().iter().skip(1) {
        assert!(!dest.join(path.as_str().unwrap()).exists());
    }
    assert_frames(&p, &dest);
}

#[test]
fn older_presets_default_to_grid_and_bounded_packed_options() {
    let preset: ExportPreset =
        serde_json::from_str(r#"{"columns":2,"scale":1,"padding":0,"margin":0}"#).unwrap();
    assert!(!preset.packed.enabled);
    assert_eq!(preset.packed.max_size, 2048);
    assert!(preset.packed.trim);
    let mut bad = preset;
    bad.packed.extrude = 33;
    assert!(bad.validate().is_err());
}

#[test]
fn clockwise_rotation_is_opt_in_and_can_reduce_page_area() {
    let mut p = scene();
    let c = &mut p.clips[0];
    c.cels.clear();
    for (tick, size) in [(0, [5., 9.]), (2, [9., 5.])] {
        let mut doc = Document {
            width: 18,
            height: 14,
            frames: c.frames,
            fps: c.fps,
            ..Document::default()
        };
        doc.add_rectangle([1., 2.], size, [220, 80, 30, 255]);
        doc.add_rectangle([1., 2.], [2., 3.], [17, 190, 25, 255]);
        c.cels.insert(tick, Arc::new(doc));
    }
    let dir = tempfile::tempdir().unwrap();
    let cost = |meta: &Value, dest: &Path| {
        meta["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| {
                let image = Pixmap::load_png(dest.join(name.as_str().unwrap())).unwrap();
                image.width() * image.height()
            })
            .sum::<u32>()
    };
    let a = dir.path().join("upright");
    export_packed_atlas(&a, &p.clips[0], &p.preset).unwrap();
    let plain = assert_frames(&p, &a);
    assert!(plain["frames"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["rotated"] == false));
    let b = dir.path().join("rotated");
    p.preset.packed.allow_rotation = true;
    export_packed_atlas(&b, &p.clips[0], &p.preset).unwrap();
    let rotated = assert_frames(&p, &b);
    assert!(rotated["frames"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["rotated"] == true));
    assert!(cost(&rotated, &b) < cost(&plain, &a));
}

#[test]
fn animation_set_shares_pages_and_keeps_each_clips_pixels_clock_and_events() {
    let mut project = scene();
    let hero = project.clips[0].set_id;
    project.rename_animation_set(hero, "Hero").unwrap();
    project.rename_clip(0, "Run").unwrap();
    let idle = project.duplicate_clip(0, "Idle").unwrap();
    project.clips[idle].set_fps(8).unwrap();
    project.clips[idle].looping = false;
    project.clips[idle].events.clear();
    project.clips[idle].drawing_at_mut(0).objects[0].fill = [0, 161, 217, 255];
    project.clips[idle].origin = [1., 2.];
    let enemy = project.add_animation_set("Enemy").unwrap();
    project.add_clip_in_set(enemy, "Run").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("hero");
    project_io::export_animation_set(&dest, &project, hero, &project.preset, |_, _| true).unwrap();
    let data = metadata(&dest);
    assert_eq!(data["format_version"], 3);
    assert_eq!(data["name"], "Hero");
    let animations = data["animations"].as_array().unwrap();
    assert_eq!(animations.len(), 2);
    let (run, idle) = (&animations[0], &animations[1]);
    assert_eq!(run["name"], "Run");
    assert_eq!(idle["name"], "Idle");
    assert_eq!(idle["fps"], 8);
    assert_eq!(idle["looping"], false);
    assert_eq!(idle["events"], serde_json::json!([]));
    assert_eq!(run["events"].as_array().unwrap().len(), 2);
    assert_eq!(idle["frames"][0]["pivot"], serde_json::json!([1., 2.]));
    for (clip, red, green) in [(run, 240, 20), (idle, 0, 161)] {
        let frame = &clip["frames"][0];
        let page = data["images"][n(frame, "page") as usize].as_str().unwrap();
        let image = Pixmap::load_png(dest.join(page)).unwrap();
        let pixel = image.pixel(n(frame, "x"), n(frame, "y")).unwrap();
        assert_eq!((pixel.red(), pixel.green()), (red, green));
    }
    for key in ["page", "x", "y", "w", "h"] {
        assert_eq!(run["frames"][3][key], idle["frames"][3][key]);
    }
    let cancelled = temp.path().join("cancelled");
    assert!(project_io::export_animation_set(
        &cancelled,
        &project,
        hero,
        &project.preset,
        |done, total| done < total
    )
    .is_err());
    assert!(!cancelled.exists());
}
