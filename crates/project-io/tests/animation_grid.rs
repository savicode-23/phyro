use document_core::{AnimationProject, Document};
use project_io::{
    animation_grid_layout, export_animation_files,
    AnimationExportTarget::{Clip, Set},
    AnimationSetOutput::{PngSequences, SeparateSheets, SharedSheet},
};
use renderer::Pixmap;
use serde_json::{json, Value};
use std::fs;

#[test]
fn uniform_grids_keep_cell_order_gaps_and_each_animations_clock() {
    let mut project = AnimationProject::from_document(Document {
        width: 8,
        height: 6,
        frames: 3,
        fps: 12,
        ..Default::default()
    });
    project.rename_animation_set(1, "Hero").unwrap();
    project.rename_clip(0, "Walk").unwrap();
    let walk = &mut project.clips[0];
    walk.drawing_at_mut(0)
        .add_rectangle([1., 1.], [2., 2.], [220, 10, 60, 255]);
    let run = project.add_clip("Run").unwrap();
    let run = &mut project.clips[run];
    run.set_length(2).unwrap();
    run.set_fps(8).unwrap();
    run.looping = false;
    run.origin = [3.5, 4.];
    run.add_event(1, "step").unwrap();
    run.drawing_at_mut(0)
        .add_rectangle([0., 0.], [2., 2.], [10, 40, 180, 255]);
    run.drawing_at_mut(0).objects[0].keys.insert(1, [4., 2.]);
    let run_id = run.id;
    project.preset.columns = 2;
    project.preset.margin = 1;
    project.preset.padding = 3;
    project.preset.packed.enabled = false;
    let dir = tempfile::tempdir().unwrap();

    for (folder, direct, mode, images) in [
        ("shared", false, SharedSheet, vec!["hero.png"]),
        (
            "separate",
            false,
            SeparateSheets,
            vec!["hero_walk.png", "hero_run.png"],
        ),
        ("run", true, SharedSheet, vec!["hero.png"]),
    ] {
        let path = dir.path().join(folder);
        let target = if direct { Clip(run_id) } else { Set(1) };
        export_animation_files(
            &path,
            &project,
            target,
            &project.preset,
            mode,
            None,
            |_, _| true,
        )
        .unwrap();
        let bytes = fs::read(path.join("sheet.json")).unwrap();
        assert_eq!(bytes, fs::read(path.join("animation.phyro")).unwrap());
        let data: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(data["layout"], "grid");
        assert_eq!(data["images"], json!(images));
        let pages: Vec<_> = images
            .iter()
            .map(|file| Pixmap::load_png(path.join(file)).unwrap())
            .collect();
        let mut total_cells = 0;
        for action in data["animations"].as_array().unwrap() {
            let clip = project
                .clips
                .iter()
                .find(|c| c.name == action["name"].as_str().unwrap())
                .unwrap();
            assert_eq!(action["fps"], clip.fps);
            assert_eq!(action["looping"], clip.looping);
            assert_eq!(
                action["frames"].as_array().unwrap().len(),
                clip.frames as usize
            );
            if clip.id == run_id {
                assert_eq!(action["events"], json!([{"frame": 1, "name": "step"}]));
            }
            let shared = mode == SharedSheet;
            let page_index = usize::from(!direct && !shared && clip.id == run_id);
            let page = &pages[page_index];
            let clips: Vec<_> = if shared && !direct {
                project.clips.iter().collect()
            } else {
                vec![clip]
            };
            let layout = animation_grid_layout(&clips, &project.preset).unwrap();
            assert_eq!((page.width(), page.height()), (layout.width, layout.height));
            assert_eq!(page.pixel(0, 0).unwrap().alpha(), 0);
            for (tick, frame) in action["frames"].as_array().unwrap().iter().enumerate() {
                let tick = tick as u32;
                let cell = if shared { total_cells + tick } else { tick };
                let (x, y) = (1 + cell % 2 * 11, 1 + cell / 2 * 9);
                assert_eq!(frame["page"], page_index);
                assert_eq!(
                    (frame["x"].as_u64(), frame["y"].as_u64()),
                    (Some(x.into()), Some(y.into()))
                );
                assert_eq!(frame["index"], tick);
                assert_eq!(frame["duration_ticks"], 1);
                assert_eq!(frame["pivot"], json!(clip.origin));
                let image = renderer::render_for_export(clip.drawing_at(tick), tick, 1.).unwrap();
                for row in 0..6 {
                    for col in 0..8 {
                        assert_eq!(page.pixel(x + col, y + row), image.pixel(col, row));
                    }
                }
            }
            total_cells += clip.frames;
        }
    }
}

#[test]
fn exceeding_one_grid_fails_before_publishing_any_files() {
    let mut project = AnimationProject::from_document(Document {
        width: 1024,
        height: 1024,
        frames: 16,
        ..Default::default()
    });
    project.add_clip("Extra").unwrap();
    project.preset.columns = 4;
    project.preset.margin = 0;
    project.preset.padding = 0;
    project.preset.packed.enabled = false;
    let clips: Vec<_> = project.clips.iter().collect();
    assert!(animation_grid_layout(&clips[..1], &project.preset).is_ok());
    assert!(animation_grid_layout(&clips, &project.preset).is_err());
    let path = tempfile::tempdir().unwrap();
    let bundle = path.path().join("oversized");
    let error = export_animation_files(
        &bundle,
        &project,
        Set(1),
        &project.preset,
        SharedSheet,
        None,
        |_, _| true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("16 megapixels"));
    assert!(!bundle.exists());
}

#[test]
fn fractional_resolution_aligns_pivots_to_the_rounded_png_dimensions() {
    let mut project = AnimationProject::from_document(Document {
        width: 5,
        height: 3,
        frames: 1,
        ..Default::default()
    });
    project.preset.set_raster_ratio(7, 10);
    project.preset.columns = 1;
    project.preset.padding = 0;
    project.preset.margin = 0;
    project.preset.packed.extrude = 0;
    project.preset.packed.trim = false;
    project.clips[0].origin = [5., 3.];
    project.clips[0]
        .drawing_at_mut(0)
        .add_rectangle([0.; 2], [5., 3.], [0, 100, 240, 255]);
    let dir = tempfile::tempdir().unwrap();
    for (name, mode, packed) in [
        ("grid", SharedSheet, false),
        ("packed", SharedSheet, true),
        ("sequence", PngSequences, false),
    ] {
        project.preset.packed.enabled = packed;
        let path = dir.path().join(name);
        export_animation_files(
            &path,
            &project,
            Set(1),
            &project.preset,
            mode,
            None,
            |_, _| true,
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(path.join("animation.phyro")).unwrap()).unwrap();
        let frame = &manifest["animations"][0]["frames"][0];
        assert_eq!(manifest["scale"], 0.7);
        assert_eq!(frame["pivot"], json!([4., 2.]));
        assert_eq!(frame["w"], 4);
        assert_eq!(frame["h"], 2);
        let image = Pixmap::load_png(path.join(manifest["images"][0].as_str().unwrap())).unwrap();
        assert_eq!((image.width(), image.height()), (4, 2));
        assert_eq!(image.pixel(3, 1).unwrap().alpha(), 255);
    }
}
