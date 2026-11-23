use project_io::{export_sprite_bundle, load_animation, save_animation};
use std::path::Path;
use tiny_skia::{IntRect, Pixmap};

#[test]
fn lantern_rig_preserves_rest_art_and_saves_the_attached_pose() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let original = load_animation(&samples.join("lantern-keeper.vanim")).unwrap();
    let mut rig = load_animation(&samples.join("lantern-rig.vanim")).unwrap();
    let rest = renderer::render(original.clips[0].drawing_at(0), 0).unwrap();
    let doc = rig.clips[0].drawing_at_mut(0);
    assert_eq!(renderer::render(doc, 0).unwrap().data(), rest.data());
    let shoulder = doc
        .skeleton
        .bones
        .iter()
        .find(|b| b.name == "Sleeve")
        .unwrap()
        .id;
    let wrist = doc
        .skeleton
        .bones
        .iter()
        .find(|b| b.name == "Lantern")
        .unwrap();
    assert_eq!(wrist.parent, Some(shoulder));
    let wrist = wrist.id;
    doc.set_bone_rotation(shoulder, -18.).unwrap();
    doc.set_bone_rotation(wrist, 12.).unwrap();
    let posed = renderer::render(doc, 0).unwrap();
    let changed = rest
        .pixels()
        .iter()
        .zip(posed.pixels())
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 100);
    for y in 94..120 {
        for x in 24..78 {
            assert_eq!(rest.pixel(x, y), posed.pixel(x, y));
        }
    }
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("pose.vanim");
    save_animation(&path, &rig, false).unwrap();
    let reopened = load_animation(&path).unwrap();
    assert_eq!(
        renderer::render(reopened.clips[0].drawing_at(0), 0)
            .unwrap()
            .data(),
        posed.data()
    );
}

#[test]
fn keyed_swing_roundtrips_and_exports_each_pose_from_a_single_drawing() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let keyed = load_animation(&samples.join("lantern-keyed.vanim")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("swing.vanim");
    save_animation(&path, &keyed, false).unwrap();
    let reopened = load_animation(&path).unwrap();
    assert_eq!(keyed, reopened);
    let clip = &reopened.clips[0];
    assert_eq!((clip.frames, clip.fps, clip.cels.len()), (8, 8, 1));
    assert!(clip.looping);

    let bundle = temp.path().join("sprites");
    export_sprite_bundle(&bundle, clip, &reopened.preset, true).unwrap();
    let sheet = Pixmap::load_png(bundle.join("sheet.png")).unwrap();
    assert_eq!((sheet.width(), sheet.height()), (512, 256));
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle.join("sheet.json")).unwrap()).unwrap();
    assert_eq!(metadata["fps"], 8);
    assert_eq!(metadata["frames"].as_array().unwrap().len(), 8);

    // Manually pose the older, unkeyed character as an independent reference for each tick.
    let mut static_project = load_animation(&samples.join("lantern-rig.vanim")).unwrap();
    let reference = static_project.clips[0].drawing_at_mut(0);
    let bone_id = |name| {
        reference
            .skeleton
            .bones
            .iter()
            .find(|b| b.name == name)
            .unwrap()
            .id
    };
    let shoulder = bone_id("Sleeve");
    let wrist = bone_id("Lantern");
    let boot_region = IntRect::from_xywh(28, 106, 54, 10).unwrap();
    let rest = renderer::render(reference, 0).unwrap();
    assert!(rest.pixel(38, 110).unwrap().alpha() > 240);
    let boots = rest.clone_rect(boot_region).unwrap();
    let arm_angles = [-12., -6., 0., 6., 12., 6., 0., -6.];
    let lamp_angles = [5., 10.5, 16., 8., 0., -8., -5.5, -3.];
    for (tick, (arm, lamp)) in arm_angles.into_iter().zip(lamp_angles).enumerate() {
        let frame = tick as u32;
        reference.set_bone_rotation(shoulder, arm).unwrap();
        reference.set_bone_rotation(wrist, lamp).unwrap();
        let expected = renderer::render(reference, 0).unwrap();
        let sampled = renderer::render(clip.drawing_at(frame), frame).unwrap();
        assert!(
            expected.data() == sampled.data(),
            "Wrong joint pose at tick {tick}"
        );
        assert!(sampled.clone_rect(boot_region).unwrap().data() == boots.data());

        let png = Pixmap::load_png(bundle.join(format!("frames/frame_{tick:04}.png"))).unwrap();
        let expected_png = Pixmap::decode_png(&expected.encode_png().unwrap()).unwrap();
        assert!(
            png.data() == expected_png.data(),
            "Wrong exported PNG at tick {tick}"
        );
        let (x, y) = ((frame % 4) * 128, (frame / 4) * 128);
        let cell = IntRect::from_xywh(x as i32, y as i32, 128, 128).unwrap();
        assert!(
            sheet.clone_rect(cell).unwrap().data() == png.data(),
            "Wrong sheet cell at tick {tick}"
        );
        let entry = &metadata["frames"][tick];
        assert_eq!(entry["x"], x);
        assert_eq!(entry["y"], y);
        assert_eq!(entry["duration_ticks"], 1);
        assert_eq!(entry["pivot"], serde_json::json!([56., 114.]));
    }
}
