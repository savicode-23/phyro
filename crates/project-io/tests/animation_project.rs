use document_core::AnimationProject;
use project_io::{load_animation, load_project, save_animation};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

#[test]
fn opening_legacy_projects_preserves_artwork_and_keys_and_leaves_input_untouched() {
    for name in ["moving-rectangle.vanim", "lantern-keeper.vanim"] {
        let path = example(name);
        let bytes = fs::read(&path).unwrap();
        let old = load_project(&path).unwrap();
        let upgraded = load_animation(&path).unwrap();
        assert_eq!(upgraded.clips.len(), 1);
        let clip = &upgraded.clips[0];
        assert_eq!(clip.frames, old.frames);
        assert_eq!(clip.fps, old.fps);
        assert_eq!(clip.cels.len(), 1);
        assert_eq!(clip.drawing_at(old.frames - 1), &old);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn new_paths_use_miter_and_legacy_missing_joins_keep_round() {
    use document_core::{Document, LineJoin, PathNode, FORMAT_VERSION};

    let mut doc = Document::default();
    let nodes = [[16., 16.], [48., 32.], [64., 16.]].map(PathNode::corner);
    for _ in 0..2 {
        doc.add_artboard_path(nodes.to_vec(), false, 1, [255; 4])
            .unwrap();
    }
    assert!(doc.objects.iter().all(|o| o.join == LineJoin::Miter));

    let mut data = serde_json::to_value(&doc).unwrap();
    data["objects"][0].as_object_mut().unwrap().remove("join");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("joins.vanim");
    write_fixture(
        &path,
        FORMAT_VERSION,
        &serde_json::to_vec(&data).unwrap(),
        false,
    );
    let loaded = load_project(&path).unwrap();
    assert_eq!(loaded.objects[0].join, LineJoin::Round);
    assert_eq!(loaded.objects[1].join, LineJoin::Miter);
}

#[test]
fn roundtrip_keeps_clips_cel_independence_pivot_and_preset() {
    let mut project = load_animation(&example("lantern-keeper.vanim")).unwrap();
    project.duplicate_clip(0, "Blink").unwrap();
    let blink = &mut project.clips[1];
    blink.origin = [42.5, 100.25];
    blink.looping = false;
    blink.duplicate_frame(0).unwrap();
    blink.drawing_at_mut(1).objects[0].fill = [1, 2, 3, 255];
    project.preset.columns = 3;
    project.preset.margin = 2;
    project.presets.insert("Godot 3x".into(), project.preset);
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("keeper.vanim");
    save_animation(&native, &project, false).unwrap();
    let reopened = load_animation(&native).unwrap();
    assert_eq!(reopened, project);
    assert_ne!(
        reopened.clips[1].drawing_at(0).objects[0].fill,
        [1, 2, 3, 255]
    );
    assert_eq!(
        reopened.clips[1].drawing_at(1).objects[0].fill,
        [1, 2, 3, 255]
    );
    assert!(
        load_project(&native).is_err(),
        "A scene-only reader cannot discard other clips"
    );
    let bytes = fs::read(&native).unwrap();
    assert_eq!(
        save_animation(&native, &project, false).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
    project.clips.clear();
    assert!(save_animation(&native, &project, true).is_err());
    assert_eq!(fs::read(&native).unwrap(), bytes);
}

fn write_fixture(path: &Path, version: u32, data: &[u8], unknown_entry: bool) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("manifest.json", options).unwrap();
    serde_json::to_writer(
        &mut zip,
        &serde_json::json!({"format":"Loopline","format_version":version}),
    )
    .unwrap();
    zip.start_file("document.json", options).unwrap();
    zip.write_all(data).unwrap();
    if unknown_entry {
        zip.start_file("future-data.json", options).unwrap();
        zip.write_all(b"{}").unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn animation_reader_rejects_unknown_versions_extra_entries_and_invalid_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("invalid.vanim");
    let valid = serde_json::to_vec(&AnimationProject::default()).unwrap();
    let mut invalid_project = AnimationProject::default();
    invalid_project.clips[0].cels.clear();
    let invalid_cels = serde_json::to_vec(&invalid_project).unwrap();
    for (version, data, unknown_entry) in [
        (u32::MAX, valid.as_slice(), false),
        (3, valid.as_slice(), true),
        (3, b"broken".as_slice(), false),
        (3, b"{}".as_slice(), false),
        (3, invalid_cels.as_slice(), false),
        (1, valid.as_slice(), false),
    ] {
        write_fixture(&native, version, data, unknown_entry);
        assert_eq!(
            load_animation(&native).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}

#[test]
fn older_multiclip_archives_migrate_once_and_keep_clip_ids_after_reorder_and_reopen() {
    let mut original = AnimationProject::default();
    original.add_clip("Rest").unwrap();
    original.duplicate_clip(0, "Wave").unwrap();
    let mut legacy = serde_json::to_value(&original).unwrap();
    legacy.as_object_mut().unwrap().remove("next_clip_id");
    for clip in legacy["clips"].as_array_mut().unwrap() {
        clip.as_object_mut().unwrap().remove("id");
    }

    let dir = tempfile::tempdir().unwrap();
    for version in [3, 4] {
        let source = dir.path().join(format!("legacy-{version}.vanim"));
        write_fixture(
            &source,
            version,
            &serde_json::to_vec(&legacy).unwrap(),
            false,
        );
        let bytes = fs::read(&source).unwrap();
        let mut upgraded = load_animation(&source).unwrap();
        assert_eq!(
            upgraded.clips.iter().map(|c| c.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(upgraded.next_clip_id, 4);
        assert_eq!(load_animation(&source).unwrap(), upgraded);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        upgraded.clips.swap(0, 2);
        let wave = upgraded.clip_index(2).unwrap();
        upgraded.rename_clip(wave, "Greeting").unwrap();
        upgraded
            .remove_clip(upgraded.clip_index(3).unwrap())
            .unwrap();
        let duplicate = upgraded
            .duplicate_clip(upgraded.clip_index(2).unwrap(), "Another")
            .unwrap();
        assert_eq!(upgraded.clips[duplicate].id, 4);
        assert_eq!(upgraded.next_clip_id, 5);
        let saved = dir.path().join(format!("new-{version}.vanim"));
        save_animation(&saved, &upgraded, false).unwrap();
        assert_eq!(load_animation(&saved).unwrap(), upgraded);
    }
}

#[test]
fn format_five_rejects_missing_duplicate_and_reusable_clip_ids() {
    let mut project = AnimationProject::default();
    project.add_clip("Walk").unwrap();
    let original = serde_json::to_value(&project).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad-id.vanim");
    for (field, value) in [
        ("missing ID", 0),
        ("duplicate ID", 1),
        ("already allocated", 3),
        ("missing cursor", 0),
    ] {
        let mut data = original.clone();
        match field {
            "missing ID" => {
                data["clips"][1].as_object_mut().unwrap().remove("id");
            }
            "missing cursor" => {
                data.as_object_mut().unwrap().remove("next_clip_id");
            }
            _ => data["clips"][1]["id"] = serde_json::json!(value),
        }
        write_fixture(&file, 5, &serde_json::to_vec(&data).unwrap(), false);
        assert_eq!(
            load_animation(&file).unwrap_err().kind(),
            ErrorKind::InvalidData,
            "{field}"
        );
    }
}

fn archive_version(path: &Path) -> u32 {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
    manifest["format_version"].as_u64().unwrap() as u32
}

#[test]
fn document_grids_round_trip_and_require_the_new_format() {
    use document_core::{GridKind, GridSettings, GridVisibility};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grid.vanim");
    let mut project = AnimationProject::default();
    save_animation(&path, &project, false).unwrap();
    assert_eq!(archive_version(&path), 5);
    assert_eq!(load_animation(&path).unwrap().grid, GridSettings::default());

    project.grid = GridSettings {
        kind: GridKind::Axonometric,
        visibility: GridVisibility::Visible,
        origin: [-7.5, 36.25],
        spacing: 22.5,
        angle_x: 26.5,
        angle_z: 52.,
    };
    project.clips[0].drawing_at_mut(0).guides_x.push(7.);
    save_animation(&path, &project, true).unwrap();
    assert_eq!(archive_version(&path), 27);
    assert_eq!(load_animation(&path).unwrap(), project);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(project_io::load_animation_bytes(&bytes).unwrap(), project);

    let payload = serde_json::json!({"project": &project, "artwork": {}});
    let old = dir.path().join("old.vanim");
    write_fixture(&old, 26, &serde_json::to_vec(&payload).unwrap(), false);
    assert!(load_animation(&old)
        .unwrap_err()
        .to_string()
        .contains("format 27"));

    project.grid.angle_x = 90.;
    assert!(save_animation(&path, &project, true).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let payload = serde_json::json!({"project": &project, "artwork": {}});
    write_fixture(&old, 27, &serde_json::to_vec(&payload).unwrap(), false);
    assert!(load_animation(&old)
        .unwrap_err()
        .to_string()
        .contains("Grid angles"));
}

#[test]
fn artwork_locks_survive_native_save_and_reopen_with_fractional_exports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locked.vanim");
    let mut project = AnimationProject::default();
    project.preset.scale_divisor = 2;
    let doc = project.clips[0].drawing_at_mut(0);
    let a = doc.add_rectangle([20.; 2], [24.; 2], [255; 4]);
    let b = doc.add_rectangle([65.; 2], [12.; 2], [255; 4]);
    for o in &mut doc.objects {
        o.group = Some(a);
    }
    save_animation(&path, &project, false).unwrap();
    assert_eq!(archive_version(&path), 25);
    assert!(load_animation(&path).unwrap().clips[0]
        .drawing_at(0)
        .objects
        .iter()
        .all(|o| !o.locked));
    document_core::arrange::set_object_locks(project.clips[0].drawing_at_mut(0), &[b], true);
    save_animation(&path, &project, true).unwrap();
    assert_eq!(archive_version(&path), 26);
    let mut reopened = load_animation(&path).unwrap();
    assert_eq!(reopened, project);
    let doc = reopened.clips[0].drawing_at_mut(0);
    assert!(doc.objects.iter().all(|o| o.locked));
    assert_eq!(doc.hit_test([25.; 2], 0), None);
    document_core::arrange::set_object_locks(doc, &[a], false);
    assert_eq!(doc.hit_test([25.; 2], 0), Some(a));
    assert!(doc.objects.iter().all(|o| !o.locked));
}

#[test]
fn animation_sets_load_by_default_and_roundtrip_membership_names_and_empty_sets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sets.vanim");
    let mut project = AnimationProject::default();
    save_animation(&path, &project, false).unwrap();
    assert_eq!(archive_version(&path), 5);
    let reopened = load_animation(&path).unwrap();
    assert_eq!(reopened.animation_sets.len(), 1);
    assert_eq!(
        reopened
            .animation_set(reopened.clips[0].set_id)
            .unwrap()
            .name,
        "Animations"
    );
    let set = project.add_animation_set("Runner").unwrap();
    project.add_clip_in_set(set, "Animation").unwrap();
    project.rename_animation_set(set, "Hero").unwrap();
    project.add_animation_set("Unused").unwrap();
    save_animation(&path, &project, true).unwrap();
    assert_eq!(archive_version(&path), 23);
    assert_eq!(load_animation(&path).unwrap(), project);
    let mut archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    let packed: serde_json::Value =
        serde_json::from_reader(archive.by_name("document.json").unwrap()).unwrap();
    drop(archive);
    write_fixture(&path, 22, &serde_json::to_vec(&packed).unwrap(), false);
    assert!(load_animation(&path)
        .unwrap_err()
        .to_string()
        .contains("format 23"));
    for (pointer, invalid) in [
        ("/project/clips/1/set_id", serde_json::json!(999)),
        ("/project/animation_sets/1/id", serde_json::json!(1)),
        (
            "/project/animation_sets/1/name",
            serde_json::json!("animations"),
        ),
        ("/project/next_set_id", serde_json::json!(2)),
    ] {
        let mut malformed = packed.clone();
        *malformed.pointer_mut(pointer).unwrap() = invalid;
        write_fixture(&path, 23, &serde_json::to_vec(&malformed).unwrap(), false);
        assert_eq!(
            load_animation(&path).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}

#[test]
fn excluding_a_nested_clip_from_set_export_roundtrips_with_its_placement_intact() {
    let source = example("lantern-walk.vanim");
    let original = load_animation(&source).unwrap();
    assert!(original.clips.iter().all(|clip| clip.include_in_set_export));
    let mut project = original.clone();
    let child = project
        .clips
        .iter_mut()
        .find(|c| c.name == "Boot poses")
        .unwrap();
    child.include_in_set_export = false;
    let child_id = child.id;
    assert_ne!(original, project);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("helper.vanim");
    save_animation(&path, &project, false).unwrap();
    assert_eq!(archive_version(&path), 24);
    let mut reopened = load_animation(&path).unwrap();
    assert_eq!(reopened, project);
    assert!(reopened.clip_usage(child_id) > 0);
    let parent = &reopened.clips[0];
    for frame in 0..parent.frames {
        assert_eq!(
            renderer::render(parent.drawing_at(frame), frame)
                .unwrap()
                .data(),
            renderer::render(original.clips[0].drawing_at(frame), frame)
                .unwrap()
                .data()
        );
    }
    let mut archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    let packed: serde_json::Value =
        serde_json::from_reader(archive.by_name("document.json").unwrap()).unwrap();
    drop(archive);
    assert!(packed["project"]["clips"][0]
        .get("include_in_set_export")
        .is_none());
    write_fixture(&path, 23, &serde_json::to_vec(&packed).unwrap(), false);
    assert!(load_animation(&path)
        .unwrap_err()
        .to_string()
        .contains("format 24"));
    let helper = reopened.clip_index(child_id).unwrap();
    reopened.clips[helper].include_in_set_export = true;
    save_animation(&path, &reopened, true).unwrap();
    assert_eq!(archive_version(&path), archive_version(&source));
}

#[test]
fn native_guided_morphs_require_version_twenty_two_and_validate_their_landmarks() {
    use document_core::{Document, PathNode, Shape};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("guided.vanim");
    let outline = |points: &[[f32; 2]]| Shape::Path {
        closed: false,
        nodes: points.iter().copied().map(PathNode::corner).collect(),
    };
    let left = outline(&[[0., 0.], [1., 0.]]);
    let right = outline(&[[0., 1.], [0.5, 0.], [1., 1.]]);
    let mut doc = Document {
        frames: 6,
        ..Default::default()
    };
    doc.add_shape([0.; 2], [10.; 2], [255; 4], left.clone(), 1);
    doc.objects[0]
        .set_guided_morph(0, 5, left, right, vec![])
        .unwrap();
    project_io::save_project(&file, &doc).unwrap();
    assert_eq!(archive_version(&file), 22);
    assert_eq!(load_project(&file).unwrap(), doc);
    let mut zip = zip::ZipArchive::new(fs::File::open(&file).unwrap()).unwrap();
    let packed: serde_json::Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    drop(zip);
    write_fixture(&file, 21, &serde_json::to_vec(&packed).unwrap(), false);
    assert!(load_animation(&file)
        .unwrap_err()
        .to_string()
        .contains("format 22"));
    for malformed in [
        serde_json::json!({"spans":{"0":{"end":4,"guides":[]}}}),
        serde_json::json!({"spans":{"0":{"end":5,"guides":[{
            "from":{"segment":1,"t":0.5},"to":{"segment":0,"t":0.5}
        }]}}}),
        serde_json::json!({"spans":{}}),
    ] {
        let mut edited = packed.clone();
        *edited
            .pointer_mut("/project/clips/0/cels/0/objects/0/animated/guided_geometry")
            .unwrap() = malformed;
        write_fixture(&file, 22, &serde_json::to_vec(&edited).unwrap(), false);
        assert_eq!(
            load_animation(&file).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}

#[test]
fn disabled_paints_keep_authored_keys_and_require_version_twenty_one_only_when_used() {
    use document_core::Document;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("paint.vanim");
    let mut doc = Document::default();
    doc.add_rectangle([8., 8.], [12., 16.], [72, 90, 0, 0]);
    let mut project = AnimationProject::from_document(doc);
    save_animation(&file, &project, false).unwrap();
    assert_eq!(archive_version(&file), 5);
    for (fill, stroke) in [(true, false), (false, true)] {
        let drawing = project.clips[0].drawing_at_mut(0);
        let o = &mut drawing.objects[0];
        o.no_fill = fill;
        o.no_stroke = stroke;
        o.stroke_width = 2.;
        o.animated.fill_keys.insert(1, [80; 4]);
        o.animated.stroke_keys.insert(1, [90; 4]);
        assert_eq!(o.fill_at(1), [80; 4]);
        assert_eq!(o.stroke_at(1), [90; 4]);
        save_animation(&file, &project, true).unwrap();
        assert_eq!(archive_version(&file), 21);
        assert_eq!(load_animation(&file).unwrap(), project);
        let drawing = project.clips[0].drawing_at(0);
        let standalone = dir.path().join(format!("drawing-{fill}.vanim"));
        project_io::save_project(&standalone, drawing).unwrap();
        assert_eq!(archive_version(&standalone), 21);
        assert_eq!(load_project(&standalone).unwrap(), *drawing);
    }
}

#[test]
fn pin_contacts_require_format_nineteen_and_older_walk_cycles_keep_their_format() {
    use std::io::Read;
    let source = example("lantern-walk.vanim");
    let mut walk = load_animation(&source).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("pinned.vanim");
    save_animation(&file, &walk, false).unwrap();
    assert_eq!(archive_version(&file), archive_version(&source));

    let doc = walk.clips[0].drawing_at_mut(0);
    let shin = doc
        .skeleton
        .bones
        .iter()
        .find(|b| b.name == "Near shin")
        .unwrap()
        .id;
    doc.pin_foot(shin, 1).unwrap();
    doc.release_foot_pin(shin, 5).unwrap();
    save_animation(&file, &walk, true).unwrap();
    assert_eq!(archive_version(&file), 19);
    assert_eq!(load_animation(&file).unwrap(), walk);

    let mut data = Vec::new();
    zip::ZipArchive::new(fs::File::open(&file).unwrap())
        .unwrap()
        .by_name("document.json")
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    write_fixture(&file, 18, &data, false);
    assert!(load_animation(&file)
        .unwrap_err()
        .to_string()
        .contains("format 19"));
}

#[test]
fn held_drawing_keys_use_format_nine_and_roundtrip_independently_of_normal_playback() {
    let mut project = AnimationProject::default();
    let child = project.add_clip("Mouth shapes").unwrap();
    let source = &mut project.clips[child];
    source
        .drawing_at_mut(0)
        .add_rectangle([3.; 2], [6.; 2], [30, 120, 70, 255]);
    source.duplicate_frame(0).unwrap();
    source.drawing_at_mut(1).objects[0].fill = [240, 100, 20, 255];
    let id = project
        .place_clip(project.clips[child].id, 0, 0, 1, [0.; 2])
        .unwrap();
    let doc = project.clips[0].drawing_at_mut(0);
    doc.set_drawing_key(id, 1, 1).unwrap();
    doc.set_drawing_key(id, 4, 0).unwrap();

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("talk.vanim");
    save_animation(&file, &project, false).unwrap();
    assert_eq!(archive_version(&file), 9);
    let mut loaded = load_animation(&file).unwrap();
    for (frame, red) in [30, 240, 240, 240, 30].into_iter().enumerate() {
        let frame = frame as u32;
        let output = renderer::render(loaded.clips[0].drawing_at(frame), frame).unwrap();
        assert_eq!(output.pixel(4, 4).unwrap().red(), red);
    }
    write_fixture(&file, 8, &serde_json::to_vec(&project).unwrap(), false);
    assert!(load_animation(&file)
        .unwrap_err()
        .to_string()
        .contains("format 9"));

    loaded.clips[0]
        .drawing_at_mut(0)
        .clear_drawing_keys(id)
        .unwrap();
    save_animation(&file, &loaded, true).unwrap();
    assert_eq!(archive_version(&file), 5);
    assert_eq!(load_animation(&file).unwrap(), loaded);
}

fn posed_arm() -> (document_core::Document, u64) {
    let mut doc = document_core::Document::default();
    let id = doc.add_rectangle([16., 9.], [6., 2.], [44, 130, 204, 255]);
    let bone = doc.add_bone("Arm", None, [10., 10.], [20., 10.]).unwrap();
    doc.bind_rigid(id, Some(bone)).unwrap();
    doc.set_bone_rotation(bone, 90.).unwrap();
    (doc, id)
}

#[test]
fn rigged_cels_and_symbols_require_version_six_and_standalone_writes_preserve_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("pose.vanim");
    let mut project = AnimationProject::default();
    project.clips[0]
        .drawing_at_mut(0)
        .add_rectangle([4., 8.], [3., 2.], [255; 4]);
    save_animation(&archive, &project, false).unwrap();
    assert_eq!(archive_version(&archive), 5);
    {
        // The v5 reader rejects unknown fields, including empty v6 defaults.
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let plain: serde_json::Value =
            serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
        let cel = &plain["clips"][0]["cels"]["0"];
        assert!(cel.get("skeleton").is_none());
        assert!(cel["objects"][0].get("rigid_bone").is_none());
        assert!(cel["objects"][0].get("rig_offset").is_none());
        assert!(cel["objects"][0].get("skin").is_none());
    }
    project.clips[0].duplicate_frame(0).unwrap();
    let d = project.clips[0].drawing_at_mut(1);
    d.add_bone("Spine", None, [3., 4.], [3., 8.]).unwrap();
    save_animation(&archive, &project, true).unwrap();
    assert_eq!(archive_version(&archive), 6);
    assert_eq!(load_animation(&archive).unwrap(), project);

    let (mut artwork, _) = posed_arm();
    let previous = fs::read(&archive).unwrap();
    assert!(project_io::save_project(&archive, &artwork).is_err());
    assert_eq!(previous, fs::read(&archive).unwrap());
    let masquerade = dir.path().join("old.vanim");
    let scene = serde_json::to_vec(&artwork).unwrap();
    write_fixture(&masquerade, 2, &scene, false);
    assert!(load_project(&masquerade).is_err());
    assert!(load_animation(&masquerade).is_err());

    artwork.frames = 1;
    let mut library = AnimationProject::default();
    library
        .add_symbol(document_core::SymbolDefinition {
            name: "Arm".into(),
            artwork: std::sync::Arc::new(artwork),
        })
        .unwrap();
    save_animation(&archive, &library, true).unwrap();
    assert_eq!(archive_version(&archive), 6);
    assert_eq!(load_animation(&archive).unwrap(), library);
    for rigged in [project, library] {
        write_fixture(&masquerade, 5, &serde_json::to_vec(&rigged).unwrap(), false);
        assert_eq!(
            load_animation(&masquerade).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}

#[test]
fn bone_tracks_require_version_seven_and_can_be_removed_for_older_rig_readers() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keyed-arm.vanim");
    let (doc, _) = posed_arm();
    let bone = doc.skeleton.bones[0].id;
    let mut project = AnimationProject::from_document(doc);
    project.clips[0]
        .drawing_at_mut(0)
        .set_bone_rotation_key(bone, 2, 45.)
        .unwrap();
    save_animation(&file, &project, false).unwrap();
    assert_eq!(archive_version(&file), 7);
    assert_eq!(load_animation(&file).unwrap(), project);
    let invalid_file = dir.path().join("mislabelled.vanim");
    write_fixture(
        &invalid_file,
        6,
        &serde_json::to_vec(&project).unwrap(),
        false,
    );
    assert_eq!(
        load_animation(&invalid_file).unwrap_err().kind(),
        ErrorKind::InvalidData
    );

    let doc = project.clips[0].drawing_at_mut(0);
    doc.remove_bone_rotation_key(bone, 2).unwrap();
    doc.set_bone_interpolation(bone, document_core::Interpolation::Hold)
        .unwrap();
    save_animation(&file, &project, true).unwrap();
    assert_eq!(archive_version(&file), 7);
    project.clips[0]
        .drawing_at_mut(0)
        .set_bone_interpolation(bone, document_core::Interpolation::Linear)
        .unwrap();
    save_animation(&file, &project, true).unwrap();
    assert_eq!(archive_version(&file), 6);
}

#[test]
fn skinned_cels_and_symbols_require_format_eight() {
    let source = load_animation(&example("lantern-elbow.vanim")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("skin.vanim");
    let mut artwork = source.clips[0].drawing_at(0).clone();
    let ids: Vec<_> = artwork.skeleton.bones.iter().map(|bone| bone.id).collect();
    for id in ids {
        for tick in [0, 4, 7] {
            artwork.remove_bone_rotation_key(id, tick).unwrap();
        }
    }
    artwork.frames = 1;
    let mut library = AnimationProject::default();
    library
        .add_symbol(document_core::SymbolDefinition {
            name: "Lift".into(),
            artwork: std::sync::Arc::new(artwork),
        })
        .unwrap();
    for project in [&source, &library] {
        save_animation(&archive, project, true).unwrap();
        assert_eq!(archive_version(&archive), 8);
        assert_eq!(&load_animation(&archive).unwrap(), project);
        write_fixture(&archive, 7, &serde_json::to_vec(project).unwrap(), false);
        assert!(load_animation(&archive)
            .unwrap_err()
            .to_string()
            .contains("format 8"));
    }

    let mut invalid = serde_json::to_value(&source).unwrap();
    let skin = invalid["clips"][0]["cels"]["0"]["objects"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find_map(|object| object.get_mut("skin"))
        .unwrap();
    skin["weights"][0].as_array_mut().unwrap().pop();
    write_fixture(&archive, 8, &serde_json::to_vec(&invalid).unwrap(), false);
    assert_eq!(
        load_animation(&archive).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[test]
fn reopened_rigid_pose_matches_raster_picking_and_exported_svg() {
    let (source, object) = posed_arm();
    let project = AnimationProject::from_document(source.clone());
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("arm.vanim");
    save_animation(&archive, &project, false).unwrap();
    let mut loaded = load_animation(&archive).unwrap();
    assert_eq!(loaded, project);
    let doc = loaded.clips[0].drawing_at(0);
    let image = renderer::render(doc, 0).unwrap();
    assert_eq!(image.data(), renderer::render(&source, 0).unwrap().data());
    assert_eq!(doc.hit_test([10., 19.], 0), Some(object));
    assert_eq!(doc.hit_test([19., 10.], 0), None);
    assert_eq!(image.pixel(10, 19).unwrap().alpha(), 255);
    assert_eq!(image.pixel(19, 10).unwrap().alpha(), 0);
    let svg = dir.path().join("posed.svg");
    project_io::export_svg(&svg, doc, 0, false).unwrap();
    let reimported = project_io::import_svg(&svg).unwrap();
    assert_eq!(
        renderer::render(&reimported, 0).unwrap().data(),
        image.data()
    );
    let doc = loaded.clips[0].drawing_at_mut(0);
    let bone = doc.skeleton.bones[0].id;
    doc.set_bone_rotation(bone, 0.).unwrap();
    assert_eq!(doc.hit_test([19., 10.], 0), Some(object));
    assert_eq!(doc.hit_test([10., 19.], 0), None);
}

#[test]
fn invalid_native_bone_reference_and_parent_cycle_are_rejected_on_open() {
    let (scene, _) = posed_arm();
    let mut encoded = serde_json::to_value(AnimationProject::from_document(scene)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("broken.vanim");
    let original = encoded.clone();
    encoded["clips"][0]["cels"]["0"]["objects"][0]["rigid_bone"] = serde_json::json!(999);
    write_fixture(&archive, 6, &serde_json::to_vec(&encoded).unwrap(), false);
    assert_eq!(
        load_animation(&archive).unwrap_err().kind(),
        ErrorKind::InvalidData
    );

    let mut encoded = original;
    let bone = &mut encoded["clips"][0]["cels"]["0"]["skeleton"]["bones"][0];
    bone["parent"] = bone["id"].clone();
    write_fixture(&archive, 6, &serde_json::to_vec(&encoded).unwrap(), false);
    assert_eq!(
        load_animation(&archive).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[test]
fn imported_multilayer_clip_remains_one_independent_instance_after_reopening() {
    use document_core::{Layer, Shape};
    let mut source = AnimationProject::default();
    let run = &mut source.clips[0];
    run.set_length(8).unwrap();
    run.drawing_at_mut(0).layers.push(Layer {
        id: 2,
        name: "Dust".into(),
        ..Layer::default()
    });
    run.drawing_at_mut(0)
        .add_rectangle([1.; 2], [16.; 2], [130; 4]);
    run.drawing_at_mut(0)
        .add_shape([12.; 2], [4.; 2], [190; 4], Shape::Ellipse, 2);
    run.blank_layer_frame(4, 2).unwrap();
    let source_id = run.id;
    let mut scene = AnimationProject::default();
    let host = &mut scene.clips[0];
    host.set_length(8).unwrap();
    host.drawing_at_mut(0).layers.push(Layer {
        id: 2,
        name: "Characters".into(),
        ..Layer::default()
    });
    scene
        .place_external_clip(&source, source_id, "Hero", (0, 0, 2), [50.; 2])
        .unwrap();
    drop(source);
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("scene.vanim");
    project_io::save_animation(&path, &scene, false).unwrap();
    let scene = project_io::load_animation(&path).unwrap();
    let parent = scene.clips[0].drawing_at(5);
    assert_eq!(parent.layers.len(), 2);
    assert_eq!(parent.layers[1].name, "Characters");
    assert_eq!(parent.objects.len(), 1);
    assert_eq!(parent.objects[0].layer, 2);
    let instance = parent.objects[0].clip_instance.as_ref().unwrap();
    let nested = instance.shared_clip().unwrap();
    assert_eq!(nested.drawing_at(5).layers[1].name, "Dust");
    assert_eq!(nested.layer_exposure(5, 1), (0, 8));
    assert_eq!(nested.layer_exposure(5, 2), (4, 4));
    assert_eq!(nested.drawing_at(0).objects.len(), 2);
    assert_eq!(nested.drawing_at(5).objects.len(), 1);
}

#[test]
fn fractional_png_resolution_round_trips_in_current_and_named_presets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pixel-size.vanim");
    let mut project = AnimationProject::default();
    let baseline = project.preset;
    project.preset.set_raster_ratio(256, 1000);
    assert_eq!(project.preset.raster_size([1000, 500]), Ok([256, 128]));
    for named in [false, true] {
        if named {
            project.presets.insert("Game".into(), project.preset);
            project.preset = baseline;
        }
        save_animation(&path, &project, true).unwrap();
        assert_eq!(archive_version(&path), 25);
        assert_eq!(load_animation(&path).unwrap(), project);
    }
    project.presets.clear();
    save_animation(&path, &project, true).unwrap();
    assert!(archive_version(&path) < 25);
    project.preset.scale_divisor = 0;
    assert!(project.preset.validate().is_err());
}
