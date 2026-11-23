use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use tiny_skia::{IntRect, Pixmap};

const INPUT: &str = "Lantern Walk.vanim";
const EDITED: &str = "Edited Walk.vanim";
const CLI: &str = env!("CARGO_BIN_EXE_phyro-cli");

fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    fs::copy(example, temp.path().join(INPUT)).unwrap();
    temp
}

#[test]
fn export_by_set_id_packs_named_actions_and_rejects_conflicting_selectors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let project_name = "Actors.vanim";
    let mut project =
        document_core::AnimationProject::from_document(document_core::Document::default());
    project.rename_clip(0, "Idle").unwrap();
    let player = project.add_animation_set("Player").unwrap();
    let idle = project.add_clip_in_set(player, "Idle").unwrap();
    let jump = project.add_clip_in_set(player, "Jump").unwrap();
    project.clips[jump].set_fps(20).unwrap();
    project_io::save_animation(&dir.join(project_name), &project, false).unwrap();
    let discovered = invoke(dir, &["inspect", project_name], true);
    let sets = discovered["result"]["animation_sets"].as_array().unwrap();
    let id = sets.iter().find(|s| s["name"] == "Player").unwrap()["id"].to_string();
    let args = ["export", project_name, "--set", &id, "--out", "atlas"];
    let exported = invoke(dir, &args, true);
    let output = &exported["result"];
    assert_eq!(
        output["clip_ids"],
        json!([project.clips[idle].id, project.clips[jump].id])
    );
    assert_eq!(output["set_name"], "Player");
    let manifest = fs::read(dir.join("atlas/animation.phyro")).unwrap();
    assert_eq!(
        manifest,
        fs::read(output["metadata"].as_str().unwrap()).unwrap()
    );
    let metadata: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(metadata["format_version"], 3);
    assert_eq!(metadata["animations"][0]["name"], "Idle");
    assert_eq!(metadata["animations"][1]["name"], "Jump");
    assert_eq!(metadata["animations"][1]["fps"], 20);
    assert_eq!(invoke(dir, &args, false)["error"]["code"], "output_exists");
    for (flags, code) in [
        (vec!["--set", &id, "--clip", "1"], "invalid_arguments"),
        (vec![], "invalid_arguments"),
        (vec!["--set", "999"], "unknown_set"),
        (
            vec!["--set", &id, "--format", "sheet-and-frames"],
            "invalid_format",
        ),
        (
            vec!["--set", &id, "--format", "packed", "--columns", "2"],
            "invalid_argument",
        ),
    ] {
        let mut args = vec!["export", project_name, "--out", "bad"];
        args.extend(flags);
        assert_eq!(invoke(dir, &args, false)["error"]["code"], code);
    }
    assert!(!dir.join("bad").exists());
}

#[test]
fn clip_and_set_exports_offer_the_same_named_layouts_and_respect_their_scope() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut project = document_core::AnimationProject::from_document(document_core::Document {
        width: 20,
        height: 16,
        frames: 2,
        ..Default::default()
    });
    let hero = project.clips[0].set_id;
    project.rename_animation_set(hero, "Hero").unwrap();
    project.rename_clip(0, "Walk").unwrap();
    let jump = project.add_clip_in_set(hero, "Jump").unwrap();
    let boot = project.add_clip_in_set(hero, "Boot poses").unwrap();
    let enemy = project.add_animation_set("Enemy").unwrap();
    project.add_clip_in_set(enemy, "Walk").unwrap();
    let boot_id = project.clips[boot].id;
    project.clips[boot]
        .drawing_at_mut(0)
        .add_rectangle([3.; 2], [6.; 2], [92, 112, 175, 255]);
    project.clips[boot].include_in_set_export = false;
    project.place_clip(boot_id, 0, 0, 1, [0.; 2]).unwrap();
    let file = "Characters.vanim";
    project_io::save_animation(&dir.join(file), &project, false).unwrap();
    let hero_id = hero.to_string();
    let boot_id_string = boot_id.to_string();

    for (format, set_pngs, solo_pngs) in [
        (
            "png-sequence",
            vec!["hero_walk_1.png", "hero_walk_2.png", "hero_jump_1.png"],
            vec!["hero_boot_poses_1.png"],
        ),
        (
            "separate-sheets",
            vec!["hero_walk.png", "hero_jump.png"],
            vec!["hero_boot_poses.png"],
        ),
        ("shared-sheet", vec!["hero.png"], vec!["hero.png"]),
    ] {
        for (selector, id, expected_pngs, names) in [
            ("--set", &hero_id, set_pngs, vec!["Walk", "Jump"]),
            ("--clip", &boot_id_string, solo_pngs, vec!["Boot poses"]),
        ] {
            let folder = format!("{}-{format}", selector.trim_start_matches('-'));
            let reply = invoke(
                dir,
                &[
                    "export", file, selector, id, "--out", &folder, "--format", format,
                ],
                true,
            );
            let result = &reply["result"];
            assert_eq!(result["format"], format);
            let ids = if selector == "--clip" {
                vec![boot_id]
            } else {
                vec![project.clips[0].id, project.clips[jump].id]
            };
            assert_eq!(result["clip_ids"], json!(ids));
            let metadata: Value =
                serde_json::from_slice(&fs::read(result["metadata"].as_str().unwrap()).unwrap())
                    .unwrap();
            assert_eq!(
                metadata["animations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a["name"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                names
            );
            let pngs = metadata["images"].as_array().unwrap();
            assert_eq!(pngs.len(), expected_pngs.len());
            for png in expected_pngs {
                assert!(pngs.contains(&json!(png)), "{folder}/{png}");
                assert!(dir.join(&folder).join(png).is_file());
            }
        }
    }
}

#[test]
fn png_resolution_and_sheet_layout_follow_overrides_and_saved_project_settings() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut doc = document_core::Document {
        width: 5,
        height: 3,
        frames: 2,
        ..Default::default()
    };
    let id = doc
        .add_rectangle([0., 0.], [5., 3.], [0, 100, 220, 255])
        .to_string();
    let mut project = document_core::AnimationProject::from_document(doc);
    project.clips[0].origin = [5., 3.];
    project.preset.set_raster_ratio(7, 10);
    project.preset.columns = 1;
    project.preset.padding = 0;
    project.preset.margin = 0;
    project.preset.packed.enabled = true;
    project_io::save_animation(&dir.join(INPUT), &project, false).unwrap();

    let named = |selector, folder, extra: &[&str], ok| {
        let mut args = vec![
            "export",
            INPUT,
            selector,
            "1",
            "--format",
            "shared-sheet",
            "--out",
            folder,
        ];
        args.extend_from_slice(extra);
        invoke(dir, &args, ok)
    };
    let first = named("--set", "saved-layout", &[], true);
    assert_eq!(first["result"]["layout"], "packed");
    assert_eq!(first["result"]["scale"], 0.7);
    let grid = named(
        "--clip",
        "grid",
        &["--layout", "grid", "--columns", "2"],
        true,
    );
    assert_eq!(grid["result"]["frame_size"], json!([4, 2]));
    assert_eq!(grid["result"]["pivot"], json!([4., 2.]));
    let sheet = Pixmap::load_png(dir.join("grid/animations.png")).unwrap();
    assert_eq!((sheet.width(), sheet.height()), (8, 2));
    let schema = invoke(dir, &["describe"], true);
    assert_eq!(
        schema["result"]["options"]["layout"]["enum"],
        json!(["grid", "packed"])
    );
    assert_eq!(schema["result"]["options"]["scale"]["type"], "number");

    let render = |cmd, name, extra: &[&str]| {
        let mut args = vec![
            cmd, INPUT, "--clip", "1", "--frame", "0", "--out", name, "--scale", "0.256",
        ];
        args.extend_from_slice(extra);
        let result = invoke(dir, &args, true);
        assert_eq!(result["result"]["scale"], 0.256);
        result
    };
    let png = render("preview", "preview.png", &[]);
    assert_eq!(png["result"]["frame_size"], json!([1, 1]));
    render("export-artboard", "pair", &["--format", "both"]);
    assert_eq!(
        fs::read(dir.join("preview.png")).unwrap(),
        fs::read(dir.join("pair/artboard.png")).unwrap()
    );
    let native = project_io::import_svg(&dir.join("pair/artboard.svg")).unwrap();
    assert_eq!((native.width, native.height), (5, 3));
    render("export-selection", "part.png", &["--object-ids", &id]);
    render(
        "export-selection-animation",
        "moving",
        &["--object-ids", &id, "--format", "frames"],
    );
    for (selector, flags) in [
        ("--set", vec!["--columns", "4"]),
        ("--clip", vec!["--layout", "bad"]),
        ("--set", vec!["--scale", "NaN"]),
        ("--clip", vec!["--layout", "grid", "--scale", "0"]),
    ] {
        assert_eq!(
            named(selector, "invalid", &flags, false)["error"]["code"],
            "invalid_argument"
        );
        assert!(!dir.join("invalid").exists());
    }
}

#[test]
fn new_advertises_and_saves_the_default_artboard_and_honors_independent_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let schema = invoke(dir, &["describe"], true);
    let options = &schema["result"]["options"];
    assert_eq!(options["width"]["default"], 1000);
    assert_eq!(options["height"]["default"], 1000);

    for (file, overrides, width, height) in [
        ("default.vanim", &[][..], 1000, 1000),
        ("wide.vanim", &["--width", "1600"][..], 1600, 1000),
        ("short.vanim", &["--height", "240"][..], 1000, 240),
    ] {
        let mut args = vec!["new", "--out", file];
        args.extend_from_slice(overrides);
        let size = json!({"width":width,"height":height});
        assert_eq!(invoke(dir, &args, true)["result"]["artboard"], size);
        let stored = invoke(dir, &["inspect", file], true);
        let clip = &stored["result"]["clips"][0];
        assert_eq!(clip["artboard"], size);
        assert_eq!(clip["pivot"], json!([width as f32 / 2., height as f32]));
    }
}

#[test]
fn new_project_can_be_authored_from_stdin_and_existing_projects_are_preserved() {
    use std::{io::Write, process::Stdio};
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let path = "Blank animation.vanim";
    let command = [
        "new", "--out", path, "--width", "320", "--height", "180", "--frames", "10", "--fps", "24",
    ];
    let new = invoke(dir, &command, true);
    assert_eq!(new["result"]["artboard"], json!({"width":320,"height":180}));
    let original = fs::read(dir.join(path)).unwrap();
    assert_eq!(
        invoke(dir, &command, false)["error"]["code"],
        "output_exists"
    );
    assert_eq!(original, fs::read(dir.join(path)).unwrap());

    let summary = invoke(dir, &["inspect", path, "--frame", "0"], true);
    let clip = &summary["result"]["clips"][0];
    let batch = json!({
        "version":1, "revision":summary["revision"], "operations":[{
            "op":"create_text","clip_id":clip["clip_id"],"exposure_start":0,
            "layer_id":clip["drawing"]["layers"][0]["layer_id"],"name":"Title",
            "position":[12,16],"fill":[30,150,255,255],"content":"Phyro"
        }]
    });
    let mut child = Command::new(CLI)
        .current_dir(dir)
        .args(["apply", path, "--batch", "-", "--out", "Lettering.vanim"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&[0xef, 0xbb, 0xbf]).unwrap();
    serde_json::to_writer(&mut input, &batch).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let title = invoke(dir, &["lookup", "Lettering.vanim", "--name", "Title"], true);
    assert_eq!(title["result"]["text"]["content"], "Phyro");
    assert_eq!(fs::read(dir.join(path)).unwrap(), original);
    assert!(
        !invoke(dir, &["new", "--out", "bad.vanim", "--fps", "0"], false)["ok"]
            .as_bool()
            .unwrap()
    );
    assert!(!dir.join("bad.vanim").exists());
}

fn invoke(dir: &Path, args: &[&str], success: bool) -> Value {
    let output = Command::new(CLI)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{args:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["version"], 1);
    assert_eq!(response["ok"], success);
    response
}

fn sleeve(dir: &Path) -> Value {
    invoke(
        dir,
        &["lookup", INPUT, "--name", "Sleeve fabric", "--frame", "0"],
        true,
    )
}

fn operation(found: &Value, op: &str) -> Value {
    let reference = &found["result"];
    json!({"op": op, "clip_id": reference["clip_id"], "exposure_start": reference["exposure_start"], "object_id": reference["object_id"]})
}

fn write_batch(dir: &Path, found: &Value, operations: &[Value]) {
    let batch = json!({"revision": found["revision"], "version":1, "operations": operations});
    fs::write(dir.join("edit.json"), serde_json::to_vec(&batch).unwrap()).unwrap();
}

#[test]
fn agent_recolors_animates_previews_and_exports_the_editable_character() {
    let temp = workspace();
    let dir = temp.path();
    let original = fs::read(dir.join(INPUT)).unwrap();
    let help = Command::new(CLI)
        .arg("--help")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("describe"));
    let schema = invoke(dir, &["describe"], true);
    assert_eq!(schema["result"]["protocol_version"], 1);
    let commands = schema["result"]["commands"].as_array().unwrap();
    for name in ["lookup", "inspect", "apply", "preview", "export"] {
        assert!(commands.iter().any(|c| c["name"] == name));
    }

    let found = sleeve(dir);
    let target = &found["result"];
    assert_eq!(target["kind"], "path");
    assert!(target["exposure_duration"].as_u64().unwrap() >= 5);
    let clip_id = target["clip_id"].as_u64().unwrap();
    let clip_arg = clip_id.to_string();
    let mut recolor = operation(&found, "set_fill");
    let color = [230, 80, 31, 255];
    recolor["fill"] = json!(color);
    let mut operations = vec![recolor];
    let x = target["position"][0].as_f64().unwrap() as f32;
    let y = target["position"][1].as_f64().unwrap() as f32;
    let positions: Vec<_> = (0..5)
        .map(|frame| [x + frame as f32 * 2., y + frame as f32])
        .collect();
    for (frame, position) in positions.iter().enumerate() {
        let mut key = operation(&found, "set_position_key");
        key["frame"] = json!(frame);
        key["position"] = json!(position);
        operations.push(key);
    }
    write_batch(dir, &found, &operations);
    let saved = invoke(
        dir,
        &["apply", INPUT, "--batch", "edit.json", "--out", EDITED],
        true,
    );
    assert_eq!(saved["revision"], found["revision"]);
    let results = saved["result"]["operations"].as_array().unwrap();
    assert_eq!(results.len(), operations.len());
    for (index, item) in results.iter().enumerate() {
        assert_eq!(item["operation_index"], index);
        assert_eq!(item["affected_ids"], json!([target["object_id"]]));
    }
    assert_eq!(fs::read(dir.join(INPUT)).unwrap(), original);

    let preview = invoke(
        dir,
        &[
            "preview", EDITED, "--clip", &clip_arg, "--frame", "4", "--out", "pose.png",
        ],
        true,
    );
    let exported = invoke(
        dir,
        &[
            "export",
            EDITED,
            "--clip",
            &clip_arg,
            "--format",
            "sheet",
            "--columns",
            "4",
            "--out",
            "sheet",
        ],
        true,
    );
    assert_ne!(preview["revision"], found["revision"]);
    assert_eq!(exported["revision"], preview["revision"]);
    let sheet = Pixmap::load_png(dir.join("sheet/sheet.png")).unwrap();
    let pose = Pixmap::load_png(dir.join("pose.png")).unwrap();
    let metadata: Value =
        serde_json::from_slice(&fs::read(dir.join("sheet/sheet.json")).unwrap()).unwrap();
    // The desktop editor opens projects and samples pixels through these same entry points.
    let reopened = project_io::load_animation(&dir.join(EDITED)).unwrap();
    let clip = &reopened.clips[reopened.clip_index(clip_id).unwrap()];
    let frames = metadata["frames"].as_array().unwrap();
    assert_eq!(frames.len() as u32, clip.frames);
    assert_eq!(metadata["fps"], clip.fps);
    assert_eq!(metadata["looping"], clip.looping);
    assert_eq!(exported["result"]["pivot"], json!(clip.origin));
    for (index, frame) in frames.iter().enumerate() {
        let tick = index as u32;
        assert_eq!(frame["index"], tick);
        assert_eq!(frame["duration_ticks"], 1);
        assert_eq!(frame["pivot"], json!(clip.origin));
        let drawing = clip.drawing_at(tick);
        let sleeve = drawing
            .objects
            .iter()
            .find(|o| o.name == "Sleeve fabric")
            .unwrap();
        assert_eq!(sleeve.fill_at(tick), color);
        if let Some(expected) = positions.get(index) {
            assert!(sleeve.keys.contains_key(&tick));
            let actual = sleeve.position_at(tick);
            assert!((actual[0] - expected[0]).hypot(actual[1] - expected[1]) < 0.001);
        }
        let rendered = renderer::render(drawing, tick).unwrap();
        let expected = Pixmap::decode_png(&rendered.encode_png().unwrap()).unwrap();
        assert_eq!(frame["w"], expected.width());
        assert_eq!(frame["h"], expected.height());
        let rect = IntRect::from_xywh(
            frame["x"].as_i64().unwrap() as i32,
            frame["y"].as_i64().unwrap() as i32,
            expected.width(),
            expected.height(),
        )
        .unwrap();
        assert!(
            sheet.clone_rect(rect).unwrap().data() == expected.data(),
            "sheet pixels at {tick}"
        );
        if tick == 4 {
            assert!(pose.data() == expected.data(), "preview pixels");
        }
    }
}

#[test]
fn failed_and_stale_batches_preserve_input_and_existing_outputs() {
    let temp = workspace();
    let dir = temp.path();
    let input = dir.join(INPUT);
    let original = fs::read(&input).unwrap();
    let found = sleeve(dir);
    let mut recolor = operation(&found, "set_fill");
    recolor["fill"] = json!([33, 62, 95, 255]);
    let mut missing = recolor.clone();
    missing["object_id"] = json!(999_999);
    write_batch(dir, &found, &[recolor.clone(), missing]);
    let apply = ["apply", INPUT, "--batch", "edit.json", "--out", EDITED];
    let failed = invoke(dir, &apply, false);
    assert_eq!(failed["error"]["code"], "invalid_operation");
    let details = &failed["error"]["details"];
    assert_eq!(details["operation_index"], 1);
    assert_eq!(details["object_id"], 999_999);
    assert_eq!(details["clip_id"], found["result"]["clip_id"]);
    assert_eq!(details["exposure_start"], found["result"]["exposure_start"]);
    assert_eq!(original, fs::read(&input).unwrap());
    assert!(!dir.join(EDITED).exists());

    write_batch(dir, &found, &[recolor]);
    let mut changed = project_io::load_animation(&input).unwrap();
    changed.clips[0].origin[0] += 2.;
    project_io::save_animation(&input, &changed, true).unwrap();
    let current = fs::read(&input).unwrap();
    let stale = invoke(dir, &apply, false);
    assert_eq!(stale["error"]["code"], "stale_revision");
    assert_eq!(stale["error"]["details"]["requested"], found["revision"]);
    assert_eq!(current, fs::read(&input).unwrap());
    assert!(!dir.join(EDITED).exists());

    let found = sleeve(dir);
    let mut op = operation(&found, "set_fill");
    op["fill"] = json!([32, 10, 50, 255]);
    write_batch(dir, &found, &[op]);
    fs::write(dir.join(EDITED), b"artist's existing file").unwrap();
    assert_eq!(invoke(dir, &apply, false)["error"]["code"], "output_exists");
    let in_place = ["apply", INPUT, "--batch", "edit.json", "--out", INPUT];
    assert_eq!(
        invoke(dir, &in_place, false)["error"]["code"],
        "output_exists"
    );
    assert_eq!(
        fs::read(dir.join(EDITED)).unwrap(),
        b"artist's existing file"
    );
    assert_eq!(current, fs::read(input).unwrap());
}

#[test]
fn ambiguous_ids_list_selectors_that_each_resolve_the_intended_object() {
    let temp = workspace();
    let dir = temp.path();
    // ID 3 appears in both the main drawing and a later Boot poses exposure.
    let response = invoke(dir, &["lookup", INPUT, "--object-id", "3"], false);
    assert_eq!(response["error"]["code"], "ambiguous_object");
    let matches = response["error"]["details"]["matches"].as_array().unwrap();
    assert!(matches.len() >= 2);
    for candidate in matches {
        let clip = candidate["clip_id"].as_u64().unwrap().to_string();
        let start = candidate["exposure_start"].as_u64().unwrap().to_string();
        let response = invoke(
            dir,
            &[
                "lookup",
                INPUT,
                "--object-id",
                "3",
                "--clip",
                &clip,
                "--frame",
                &start,
            ],
            true,
        );
        for field in [
            "object_id",
            "name",
            "layer_id",
            "clip_id",
            "exposure_start",
            "exposure_duration",
        ] {
            assert_eq!(response["result"][field], candidate[field]);
        }
    }
}

#[test]
fn selection_exports_report_the_scaled_pasteboard_position_and_group_members() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut drawing = document_core::Document::default();
    let red = drawing.add_rectangle([-6., 3.], [3., 2.], [255, 0, 0, 255]);
    let green = drawing.add_rectangle([-3., 4.], [2., 2.], [0, 255, 0, 255]);
    let blue = drawing.add_rectangle([9., 3.], [2., 1.], [0, 0, 255, 255]);
    drawing.objects[0].group = Some(14);
    drawing.objects[1].group = Some(14);
    drawing.objects[2].keys.insert(0, [9., 3.]);
    drawing.objects[2].keys.insert(1, [0., 3.]);
    // This foreground object covers both chosen assets. It must stay out of their PNGs.
    drawing.add_rectangle([-9., 0.], [30., 15.], [0, 0, 0, 255]);
    let project = document_core::AnimationProject::from_document(drawing);
    let clip = project.clips[0].id.to_string();
    project_io::save_animation(&dir.join(INPUT), &project, false).unwrap();
    let data = fs::read(dir.join(INPUT)).unwrap();
    let ids = format!("{red},{blue}");
    let export = |mode: &str, ids: &str, out: &str, ok| {
        invoke(
            dir,
            &[
                "export-selection",
                INPUT,
                "--clip",
                &clip,
                "--frame",
                "1",
                "--object-ids",
                ids,
                "--mode",
                mode,
                "--scale",
                "2",
                "--out",
                out,
            ],
            ok,
        )
    };
    let result = export("combined", &ids, "combined.png", true);
    assert_eq!(result["result"]["format"], "png");
    let combined = &result["result"]["assets"][0];
    assert_eq!(combined["origin"], json!([-12, 6]));
    assert_eq!(combined["size"], json!([16, 6]));
    let image = Pixmap::load_png(combined["path"].as_str().unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (16, 6));
    assert_eq!(image.pixel(0, 0).unwrap().red(), 255);
    assert_eq!(image.pixel(8, 4).unwrap().green(), 255);
    assert_eq!(image.pixel(10, 0).unwrap().alpha(), 0);
    assert_eq!(image.pixel(13, 0).unwrap().blue(), 255);

    let result = export("individual", &ids, "parts", true);
    let parts = result["result"]["assets"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    let group = parts
        .iter()
        .find(|asset| {
            asset["object_ids"]
                .as_array()
                .unwrap()
                .contains(&json!(green))
        })
        .unwrap();
    assert!(group["object_ids"]
        .as_array()
        .unwrap()
        .contains(&json!(red)));
    assert_eq!(group["origin"], json!([-12, 6]));
    assert_eq!(group["size"], json!([10, 6]));
    let single = parts
        .iter()
        .find(|asset| asset["object_ids"] == json!([blue]))
        .unwrap();
    assert_eq!(single["origin"], json!([0, 6]));
    assert_eq!(single["size"], json!([4, 2]));
    for asset in parts {
        assert!(Path::new(asset["path"].as_str().unwrap()).is_file());
    }
    assert_eq!(
        export("individual", &ids, "parts", false)["error"]["code"],
        "output_exists"
    );
    assert_eq!(
        export("combined", "1,,3", "bad.png", false)["error"]["code"],
        "invalid_arguments"
    );
    let invalid = export("individual", &format!("{red},999999"), "missing", false);
    assert_eq!(
        invalid["error"]["details"]["object_ids"],
        json!([red, 999999])
    );
    assert!(!dir.join("missing").exists());
    assert_eq!(fs::read(dir.join(INPUT)).unwrap(), data);
}

#[test]
fn individual_svg_export_is_discoverable_and_preserves_groups_at_the_chosen_frame() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let schema = invoke(dir, &["describe"], true);
    let contract = &schema["result"]["options"]["format"]["by_command"]["export-selection"];
    assert_eq!(contract["default"], "png");
    assert_eq!(contract["requires"]["svg"], json!({"scale":1}));
    let mut drawing = document_core::Document::default();
    let red = drawing.add_rectangle([-8., 2.], [4., 4.], [255, 0, 0, 255]);
    let green = drawing.add_rectangle([-4., 2.], [2., 2.], [0, 255, 0, 255]);
    let blue = drawing.add_rectangle([3., 9.], [3., 2.], [0, 0, 255, 255]);
    drawing.objects[0].group = Some(14);
    drawing.objects[1].group = Some(14);
    drawing.objects[2]
        .keys
        .extend([(0, [3., 9.]), (1, [11., 5.])]);
    let project = document_core::AnimationProject::from_document(drawing);
    let clip = project.clips[0].id.to_string();
    project_io::save_animation(&dir.join(INPUT), &project, false).unwrap();
    let original = fs::read(dir.join(INPUT)).unwrap();
    let ids = format!("{red},{blue}");
    let export = |mode: &str, format: &str, scale: &str, out: &str, ok| {
        invoke(
            dir,
            &[
                "export-selection",
                INPUT,
                "--clip",
                &clip,
                "--frame",
                "1",
                "--object-ids",
                &ids,
                "--out",
                out,
                "--mode",
                mode,
                "--format",
                format,
                "--scale",
                scale,
            ],
            ok,
        )
    };
    let result = export("individual", "svg", "1", "parts", true);
    let result = &result["result"];
    assert_eq!(result["format"], "svg");
    assert_eq!(result["mode"], "individual");
    assert_eq!(result["scale"], 1);
    let assets = result["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 2);
    assert_eq!(assets[0]["object_ids"], json!([red, green]));
    assert_eq!(assets[0]["origin"], json!([-8, 2]));
    assert_eq!(assets[0]["size"], json!([6, 4]));
    assert_eq!(assets[1]["object_ids"], json!([blue]));
    assert_eq!(assets[1]["origin"], json!([11, 5]));
    assert_eq!(assets[1]["size"], json!([3, 2]));
    for (asset, listed) in assets.iter().zip(result["files"].as_array().unwrap()) {
        let path = Path::new(asset["path"].as_str().unwrap());
        assert_eq!(&asset["path"], listed);
        assert_eq!(path.extension().unwrap(), "svg");
        let xml = fs::read_to_string(path).unwrap();
        assert!(xml.contains("<svg") && xml.contains("viewBox="));
        assert!(!xml.contains("<image"));
    }
    assert_eq!(
        export("individual", "svg", "1", "parts", false)["error"]["code"],
        "output_exists"
    );
    let cropped = export("combined", "svg", "1", "parts.svg", true);
    let svg =
        project_io::import_svg(Path::new(cropped["result"]["files"][0].as_str().unwrap())).unwrap();
    assert_eq!(svg.objects.len(), 3);
    let together = export("combined", "both", "2", "selection-pair", true);
    assert_eq!(together["result"]["files"].as_array().unwrap().len(), 2);
    assert!(dir.join("selection-pair/selection.png").is_file());
    assert!(dir.join("selection-pair/selection.svg").is_file());
    let batch = export("individual", "both", "1", "pairs", true);
    assert_eq!(batch["result"]["files"].as_array().unwrap().len(), 4);
    for path in fs::read_dir(dir.join("parts")).unwrap() {
        let file = path.unwrap().file_name();
        let svg = dir.join("pairs").join(file);
        assert!(svg.is_file() && svg.with_extension("png").is_file());
    }
    let page = invoke(
        dir,
        &[
            "export-artboard",
            INPUT,
            "--clip",
            &clip,
            "--frame",
            "1",
            "--format",
            "both",
            "--scale",
            "2",
            "--out",
            "page",
        ],
        true,
    );
    assert_eq!(page["result"]["files"].as_array().unwrap().len(), 2);
    let restored = project_io::import_svg(&dir.join("page/artboard.svg")).unwrap();
    assert_eq!(
        [restored.width, restored.height],
        [
            project.clips[0].drawing_at(0).width,
            project.clips[0].drawing_at(0).height
        ]
    );
    let png = Pixmap::load_png(dir.join("page/artboard.png")).unwrap();
    assert_eq!(
        [png.width(), png.height()],
        [restored.width * 2, restored.height * 2]
    );
    for (mode, format, scale, code) in [
        ("combined", "svg", "2", "invalid_argument"),
        ("individual", "svg", "2", "invalid_argument"),
        ("individual", "jpeg", "1", "invalid_format"),
    ] {
        assert_eq!(
            export(mode, format, scale, "unused", false)["error"]["code"],
            code
        );
        assert!(!dir.join("unused").exists());
    }
    assert_eq!(fs::read(dir.join(INPUT)).unwrap(), original);
}

#[test]
fn animated_selection_reports_common_bounds_preserves_blank_ticks_and_checks_ranges() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-sleeve-reach.vanim");
    fs::copy(source, dir.join(INPUT)).unwrap();
    let frame = invoke(dir, &["inspect", INPUT, "--frame", "0"], true);
    let drawing = &frame["result"]["clips"][0]["drawing"]["objects"];
    let part = drawing
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "Sleeve fabric")
        .unwrap();
    let ids = part["object_id"].to_string();
    let clip = frame["result"]["clips"][0]["clip_id"].to_string();
    let export = |first, last, format, out, ok| {
        invoke(
            dir,
            &[
                "export-selection-animation",
                INPUT,
                "--clip",
                &clip,
                "--frame",
                "0",
                "--object-ids",
                &ids,
                "--scale",
                "2",
                "--from",
                first,
                "--to",
                last,
                "--format",
                format,
                "--out",
                out,
            ],
            ok,
        )
    };
    let result = export("1", "6", "sheet-and-frames", "sleeve", true);
    let data = &result["result"];
    assert_eq!(data["origin"], json!([-59, 14]));
    assert_eq!(data["frame_size"], json!([164, 64]));
    assert_eq!(data["pivot"], json!([70.0, 30.5]));
    assert_eq!(data["frame_count"], 6);
    assert_eq!(data["files"].as_array().unwrap().len(), 8);
    assert_eq!(result["revision"], frame["revision"]);
    let path = Path::new(data["metadata"].as_str().unwrap());
    let metadata: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(metadata["frames"][2]["source_frame"], 3);
    assert_eq!(metadata["frames"][2]["trim_size"], json!([0, 0]));
    let blank = export("3", "4", "frames", "blank", true);
    assert_eq!(blank["result"]["frame_count"], 2);
    assert_eq!(blank["result"]["frame_size"], json!([1, 1]));
    assert_eq!(blank["result"]["files"].as_array().unwrap().len(), 3);
    assert_eq!(
        export("1", "6", "frames", "sleeve", false)["error"]["code"],
        "output_exists"
    );
    assert_eq!(
        export("5", "2", "sheet", "bad", false)["error"]["code"],
        "frame_out_of_range"
    );
    assert_eq!(
        export("0", "5", "packed", "bad", false)["error"]["code"],
        "invalid_format"
    );
    assert!(!dir.join("bad").exists());
}
