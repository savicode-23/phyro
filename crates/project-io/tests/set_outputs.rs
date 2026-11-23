use document_core::{AnimationProject, Document};
use project_io::{
    animation_clip_example_filename, animation_set_example_filename, animation_set_file_stem,
    export_animation_files, export_animation_set_with_format, export_clip_with_format,
    AnimationExportTarget, AnimationSetOutput, AnimationSheetPreviewPlan,
};
use renderer::Pixmap;
use serde_json::Value;
use std::{fs, path::Path};

const MODES: [AnimationSetOutput; 3] = [
    AnimationSetOutput::PngSequences,
    AnimationSetOutput::SeparateSheets,
    AnimationSetOutput::SharedSheet,
];

fn hero() -> AnimationProject {
    let mut project = AnimationProject::from_document(Document {
        width: 16,
        height: 12,
        frames: 3,
        ..Default::default()
    });
    project.rename_animation_set(1, "Hero").unwrap();
    project.rename_clip(0, "Walk").unwrap();
    let run = project.add_clip("Run").unwrap();
    project.clips[run].set_fps(8).unwrap();
    project.clips[run].origin = [3., 4.];
    project.clips[run].looping = false;
    project.clips[run].add_event(0, "step").unwrap();
    project.clips[run]
        .drawing_at_mut(0)
        .add_rectangle([0.; 2], [2.; 2], [20, 200, 40, 255]);
    let part = project.add_clip("Boot").unwrap();
    project.clips[part]
        .drawing_at_mut(0)
        .add_rectangle([0.; 2], [2.; 2], [255, 0, 0, 255]);
    project
        .place_clip(project.clips[part].id, 0, 0, 1, [2., 2.])
        .unwrap();
    project.clips[part].include_in_set_export = false;
    // The set exporter must resolve the edited source before sampling Walk.
    project.clips[part].drawing_at_mut(0).objects[0].fill = [30, 120, 190, 255];
    let enemy = project.add_animation_set("Enemy").unwrap();
    project.add_clip_in_set(enemy, "Walk").unwrap();
    project
}

fn manifest(path: &Path) -> Value {
    let bytes = fs::read(path.join("sheet.json")).unwrap();
    assert_eq!(bytes, fs::read(path.join("animation.phyro")).unwrap());
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn all_formats_honor_selected_actions_and_sample_unchecked_children() {
    let project = hero();
    let snapshot = project.resolved_snapshot().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    for mode in MODES {
        let dest = tmp.path().join(format!("{mode:?}"));
        export_animation_set_with_format(&dest, &project, 1, &project.preset, mode, |_, _| true)
            .unwrap();
        let data = manifest(&dest);
        let clips = data["animations"].as_array().unwrap();
        assert_eq!(
            clips
                .iter()
                .map(|c| c["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Walk", "Run"]
        );
        assert_eq!(clips[1]["fps"], 8);
        assert_eq!(clips[1]["looping"], false);
        assert_eq!(clips[1]["events"][0]["name"], "step");
        let files: Vec<_> = data["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected: &[&str] = match mode {
            AnimationSetOutput::PngSequences => &[
                "hero_walk_1.png",
                "hero_walk_2.png",
                "hero_walk_3.png",
                "hero_run_1.png",
            ],
            AnimationSetOutput::SeparateSheets => &["hero_walk.png", "hero_run.png"],
            AnimationSetOutput::SharedSheet => &["hero.png"],
        };
        assert_eq!(files, expected);
        assert_eq!(
            animation_set_example_filename(&project, 1, mode).as_deref(),
            Some(files[0])
        );
        let pages: Vec<_> = files
            .iter()
            .map(|name| Pixmap::load_png(dest.join(name)).unwrap())
            .collect();
        for (clip, action) in snapshot.clips.iter().zip(clips) {
            for (tick, frame) in action["frames"].as_array().unwrap().iter().enumerate() {
                let index = |name: &str| frame[name].as_u64().unwrap_or(0) as u32;
                let source =
                    renderer::render_for_export(clip.drawing_at(tick as u32), tick as u32, 1.)
                        .unwrap();
                let page = &pages[index("page") as usize];
                let (left, top) = (index("trim_x"), index("trim_y"));
                let (w, h) = (index("w"), index("h"));
                for y in 0..source.height() {
                    for x in 0..source.width() {
                        if (left..left + w).contains(&x) && (top..top + h).contains(&y) {
                            assert_eq!(
                                source.pixel(x, y),
                                page.pixel(index("x") + (x - left), index("y") + (y - top))
                            );
                        } else {
                            assert_eq!(source.pixel(x, y).unwrap().alpha(), 0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn explicit_clip_ignores_its_set_checkbox_and_matches_one_member_set() {
    let mut project = hero();
    project.clips[0].include_in_set_export = false;
    let run = project.clips[1].id;
    let tmp = tempfile::tempdir().unwrap();
    for mode in MODES {
        let set = tmp.path().join(format!("set-{mode:?}"));
        let only = tmp.path().join(format!("clip-{mode:?}"));
        export_animation_set_with_format(&set, &project, 1, &project.preset, mode, |_, _| true)
            .unwrap();
        project.clips[1].include_in_set_export = false;
        let output =
            export_clip_with_format(&only, &project, run, &project.preset, mode, |_, _| true)
                .unwrap();
        for file in output.files() {
            assert_eq!(
                fs::read(set.join(file)).unwrap(),
                fs::read(only.join(file)).unwrap()
            );
        }
        let data = manifest(&only);
        assert_eq!(
            animation_clip_example_filename(&project, run, mode).as_deref(),
            data["images"][0].as_str()
        );
        assert!(animation_set_example_filename(&project, 1, mode).is_none());
        let empty = tmp.path().join("no-actions");
        assert!(export_animation_set_with_format(
            &empty,
            &project,
            1,
            &project.preset,
            mode,
            |_, _| true
        )
        .is_err());
        assert!(!empty.exists());
        project.clips[1].include_in_set_export = true;
    }
}

#[test]
fn filenames_are_portable_and_output_collisions_keep_both_actions() {
    let mut project = hero();
    project.rename_animation_set(1, "CON").unwrap();
    project.rename_clip(0, "Slide / Stop").unwrap();
    project.rename_clip(1, "Slide : Stop").unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("safe");
    let mode = AnimationSetOutput::SeparateSheets;
    export_animation_set_with_format(&dest, &project, 1, &project.preset, mode, |_, _| true)
        .unwrap();
    let data = manifest(&dest);
    assert_eq!(data["images"][0], "_con_slide_stop.png");
    let suffix = format!("_con_slide_stop-{}.png", project.clips[1].id);
    assert_eq!(data["images"][1], suffix);
    project.clips[0].include_in_set_export = false;
    assert_eq!(
        animation_set_example_filename(&project, 1, mode),
        Some(suffix)
    );
    assert_eq!(animation_set_file_stem("../"), "animation");
    assert_eq!(animation_set_file_stem("LPT²"), "_lpt²");
    assert!(animation_set_file_stem(&"腕".repeat(80)).len() < 96);
}

#[test]
fn packed_set_and_clip_exports_grow_to_one_sheet_and_match_previews() {
    let mut project = hero();
    project.preset.packed.enabled = true;
    project.preset.packed.deduplicate = false;
    project.preset.packed.max_size = 8;
    let target = AnimationExportTarget::Set(1);
    let tmp = tempfile::tempdir().unwrap();
    for (mode, names) in [
        (AnimationSetOutput::SharedSheet, vec!["hero.png"]),
        (
            AnimationSetOutput::SeparateSheets,
            vec!["hero_walk.png", "hero_run.png"],
        ),
    ] {
        let preview =
            AnimationSheetPreviewPlan::prepare(&project, target, &project.preset, mode, |_, _| {
                true
            })
            .unwrap();
        let out = tmp.path().join(format!("{mode:?}"));
        preview.write(&out, None, |_, _| true).unwrap();
        let data = manifest(&out);
        assert_eq!(data["images"], serde_json::json!(names));
        for (i, (name, size)) in preview.sheets().iter().enumerate() {
            let image = Pixmap::load_png(out.join(name)).unwrap();
            assert_eq!(size, &[image.width(), image.height()]);
            assert_eq!(preview.render(i, |_, _| true).unwrap().data(), image.data());
            assert!(size.iter().all(|n| *n <= 8));
        }
        for (i, action) in data["animations"].as_array().unwrap().iter().enumerate() {
            let expected_page = if names.len() == 1 { 0 } else { i };
            assert!(action["frames"]
                .as_array()
                .unwrap()
                .iter()
                .all(|frame| frame["page"] == expected_page));
        }
    }
    project.clips[1].set_length(1000).unwrap();
    let out = tmp.path().join("oversized");
    for (target, filename) in [
        (target, "hero_run.png"),
        (
            AnimationExportTarget::Clip(project.clips[1].id),
            "hero_run.png",
        ),
    ] {
        let mode = AnimationSetOutput::SeparateSheets;
        let expected = AnimationSheetPreviewPlan::prepare(
            &project,
            target,
            &project.preset,
            mode,
            |done, _| {
                assert!(
                    done < 12,
                    "An oversized clip continued sampling after it ran out of sheet space."
                );
                true
            },
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            expected.contains(filename) && expected.contains("one 8 × 8 px sprite sheet"),
            "{expected}"
        );
        let error = export_animation_files(
            &out,
            &project,
            target,
            &project.preset,
            mode,
            None,
            |_, _| true,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert!(!out.exists());
    }
}

#[test]
fn preview_reuses_hydrated_artwork_with_new_membership_and_settings() {
    use std::sync::Arc;
    let artwork = Arc::new(hero().resolved_snapshot().unwrap());
    let boot = artwork.clips[2].id;
    let run = artwork.clips[1].id;
    let enemy = artwork.clips[3].id;
    let mut preset = artwork.preset;
    let prepare = |included: &[u64], preset: &document_core::ExportPreset| {
        AnimationSheetPreviewPlan::prepare_snapshot(
            Arc::clone(&artwork),
            AnimationExportTarget::Set(1),
            preset,
            AnimationSetOutput::SeparateSheets,
            included,
            |_, _| true,
        )
    };
    let first = prepare(&[run], &preset).unwrap();
    assert!(!artwork.clips[2].include_in_set_export);
    preset.packed.enabled = true;
    let second = prepare(&[boot, run], &preset).unwrap();
    assert_eq!(Arc::strong_count(&artwork), 3);
    assert_eq!(first.sheets()[0].0, "hero_run.png");
    assert_eq!(first.sheets().len(), 1);
    assert_eq!(
        second
            .sheets()
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        ["hero_run.png", "hero_boot.png"]
    );
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("rerun");
    second.write(&dest, None, |_, _| true).unwrap();
    assert_eq!(manifest(&dest)["animations"][1]["name"], "Boot");
    assert!(prepare(&[enemy], &preset).is_err());
    assert!(prepare(&[run, run], &preset).is_err());
    assert!(prepare(&[], &preset).is_err());
}

#[test]
fn smaller_packed_resolution_fits_and_uses_the_actual_rounded_frame_dimensions() {
    let mut doc = Document {
        width: 15,
        height: 9,
        frames: 1,
        ..Default::default()
    };
    doc.add_rectangle([1., 2.], [7., 6.], [43, 240, 120, 255]);
    let mut project = AnimationProject::from_document(doc);
    project.clips[0].origin = [7.5, 4.5];
    let target = AnimationExportTarget::Clip(project.clips[0].id);
    let mut preset = project.preset;
    preset.packed.enabled = true;
    preset.packed.trim = false;
    preset.packed.extrude = 0;
    preset.packed.max_size = 12;
    let mode = AnimationSetOutput::SeparateSheets;
    let plan = |p: &document_core::ExportPreset| {
        AnimationSheetPreviewPlan::prepare(&project, target, p, mode, |_, _| true)
    };
    assert!(plan(&preset).is_err());
    preset.set_raster_ratio(1, 2);
    let ready = plan(&preset).unwrap();
    assert_eq!(ready.sheets()[0].1, [8, 5]);
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("half-size");
    ready.write(&dest, None, |_, _| true).unwrap();
    let manifest = manifest(&dest);
    assert_eq!(manifest["scale"], 0.5);
    let frame = &manifest["animations"][0]["frames"][0];
    assert_eq!(
        (&frame["source_w"], &frame["source_h"]),
        (&Value::from(8), &Value::from(5))
    );
    assert_eq!(frame["pivot"], serde_json::json!([4., 2.5]));
    let sprite = Pixmap::load_png(dest.join(&ready.sheets()[0].0)).unwrap();
    let expected = renderer::render_for_export(project.clips[0].drawing_at(0), 0, 0.5).unwrap();
    assert_eq!(sprite.data(), expected.data());
}

#[test]
fn packed_export_rerun_replaces_only_its_own_files_after_new_settings_fit() {
    let mut project = hero();
    project.preset.packed.enabled = true;
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("assets");
    let target = AnimationExportTarget::Set(1);
    let mode = AnimationSetOutput::SharedSheet;
    let first = export_animation_files(
        &out,
        &project,
        target,
        &project.preset,
        mode,
        None,
        |_, _| true,
    )
    .unwrap();
    let old_png = fs::read(out.join("hero.png")).unwrap();
    project.preset.packed.max_size = 2;
    assert!(export_animation_files(
        &out,
        &project,
        target,
        &project.preset,
        mode,
        Some(&first),
        |_, _| true
    )
    .is_err());
    assert_eq!(fs::read(out.join("hero.png")).unwrap(), old_png);
    project.preset.packed.max_size = 64;
    project.preset.scale = 2;
    let new = export_animation_files(
        &out,
        &project,
        target,
        &project.preset,
        mode,
        Some(&first),
        |_, _| true,
    )
    .unwrap();
    assert_ne!(fs::read(out.join("hero.png")).unwrap(), old_png);
    fs::write(out.join("hero.png"), b"hand-edited").unwrap();
    assert!(export_animation_files(
        &out,
        &project,
        target,
        &project.preset,
        mode,
        Some(&new),
        |_, _| true
    )
    .is_err());
    assert_eq!(fs::read(out.join("hero.png")).unwrap(), b"hand-edited");
}

#[test]
fn cancelled_exports_publish_no_partial_actions_and_refuse_to_replace_files() {
    let project = hero();
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("hero");
    for mode in MODES {
        let mut previous = 0;
        let result = export_animation_set_with_format(
            &dest,
            &project,
            1,
            &project.preset,
            mode,
            |done, total| {
                assert!(done >= previous && done <= total);
                previous = done;
                done < total
            },
        );
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(!dest.exists());
    }
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("hero.png"), b"existing artwork").unwrap();
    assert!(export_animation_set_with_format(
        &dest,
        &project,
        1,
        &project.preset,
        AnimationSetOutput::SharedSheet,
        |_, _| true
    )
    .is_err());
    assert_eq!(
        fs::read(dest.join("hero.png")).unwrap(),
        b"existing artwork"
    );
    assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
}
