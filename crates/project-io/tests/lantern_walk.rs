use document_core::{AnimationProject, Document, LoopMode, VectorObject};
use project_io::{export_sprite_bundle, load_animation, save_animation};
use std::path::Path;
use tiny_skia::{IntRect, Pixmap};

fn project() -> AnimationProject {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    load_animation(&sample).unwrap()
}

fn joint(doc: &Document, name: &str) -> u64 {
    doc.skeleton
        .bones
        .iter()
        .find(|b| b.name == name)
        .unwrap()
        .id
}

fn close(a: [f32; 2], b: [f32; 2]) {
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 0.0001, "{a:?} vs {b:?}");
}

// The artist's contact chart in artboard pixels. Each stance lasts through toe-off.
fn boot_pose(far: bool, tick: u32) -> ([f32; 2], f32, u32) {
    let x = if far { 50. } else { 58. };
    let phase = (tick + if far { 4 } else { 0 }) % 8;
    let (dx, y, roll, drawing) = match phase {
        5 => (-5., 104., 18., 1),
        6 => (0., 100., -9., 1),
        7 => (5., 104., -12., 1),
        contact => (6. - 3. * contact as f32, 108., 0., 0),
    };
    ([x + dx, y], roll, drawing)
}

#[test]
fn full_stride_levels_each_planted_sole_and_holds_contact_across_the_loop() {
    let project = project();
    let clip = &project.clips[0];
    let doc = clip.drawing_at(0);
    assert_eq!((clip.frames, clip.fps, clip.cels.len()), (8, 8, 1));
    assert!(clip.looping);
    assert_eq!(clip.origin, [56., 113.]);
    assert_eq!(clip.events[&0], ["near_contact"]);
    assert_eq!(clip.events[&4], ["far_contact"]);
    assert_eq!(project.clips[1].cels.len(), 2);
    for (side, far, contact) in [("Near", false, 0), ("Far", true, 4)] {
        let shin = joint(doc, &format!("{side} shin"));
        let sole = joint(doc, &format!("{side} sole"));
        let boot = doc
            .objects
            .iter()
            .find(|o| o.name == format!("{side} boot"))
            .unwrap();
        let trousers = doc
            .objects
            .iter()
            .find(|o| o.name == format!("{side} trousers"))
            .unwrap();
        assert_eq!(boot.rigid_bone, Some(sole));
        assert_eq!(trousers.skin.as_ref().unwrap().child, shin);
        let mut planted = None;
        for time in contact..contact + 8 {
            let tick = time % 8;
            let (ankle, angle, shape) = boot_pose(far, tick);
            close(doc.bone_segment_at(shin, true, tick).unwrap().1, ankle);
            let (head, tip) = doc.bone_segment_at(sole, true, tick).unwrap();
            close(head, ankle);
            close(boot.local_to_world([15. / 32., 0.5], tick), ankle);
            let (sin, cos) = angle.to_radians().sin_cos();
            close(tip, [ankle[0] + 8. * cos, ankle[1] + 8. * sin]);
            assert_eq!(boot.instance_at(tick).unwrap().1, shape);
            if shape == 0 {
                let contact_in_world = [tip[0] + time as f32 * 3., tip[1]];
                close(contact_in_world, *planted.get_or_insert(contact_in_world));
            } else {
                assert!(ankle[1] <= 104.);
            }
        }
        assert_ne!(trousers.skinned_shape_at(0), trousers.skinned_shape_at(2));
    }
    for name in ["Far sleeve", "Sleeve fabric"] {
        let sleeve = doc.objects.iter().find(|o| o.name == name).unwrap();
        assert_ne!(sleeve.skinned_shape_at(0), sleeve.skinned_shape_at(4));
    }
}

#[test]
fn planted_ankle_survives_hip_motion_native_reload_and_selected_sprite_export() {
    use document_core::ExportPreset;
    use project_io::{export_selection_animation, SelectionRange, SpriteOutput};
    let mut walk = project();
    let doc = walk.clips[0].drawing_at_mut(0);
    let thigh = joint(doc, "Near thigh");
    let shin = joint(doc, "Near shin");
    let sole = joint(doc, "Near sole");
    let hip = doc
        .add_bone("Pelvis", None, [54., 82.], [58., 87.])
        .unwrap();
    doc.skeleton
        .bones
        .iter_mut()
        .find(|b| b.id == thigh)
        .unwrap()
        .parent = Some(hip);
    doc.set_bone_rotation_key(hip, 0, 0.).unwrap();
    doc.set_bone_rotation_key(hip, 4, 18.).unwrap();
    let anchor = doc.bone_segment_at(shin, true, 1).unwrap().1;
    let toe = doc.bone_segment_at(sole, true, 1).unwrap().1;
    doc.pin_foot(shin, 1).unwrap();
    doc.release_foot_pin(shin, 5).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("contact.vanim");
    save_animation(&file, &walk, false).unwrap();
    let saved = load_animation(&file).unwrap();
    let clip = &saved.clips[0];
    let doc = clip.drawing_at(0);
    let boot = doc.objects.iter().find(|o| o.name == "Near boot").unwrap();
    let pants = doc
        .objects
        .iter()
        .find(|o| o.name == "Near trousers")
        .unwrap();
    for tick in 1..5 {
        close(doc.foot_pin_at(shin, tick).unwrap(), anchor);
        close(doc.bone_segment_at(shin, true, tick).unwrap().1, anchor);
        close(doc.bone_segment_at(sole, true, tick).unwrap().1, toe);
        close(boot.local_to_world([15. / 32., 0.5], tick), anchor);
    }
    assert!(doc.foot_pin_at(shin, 0).is_none());
    assert!(doc.foot_pin_at(shin, 5).is_none());
    let lifted = boot.local_to_world([15. / 32., 0.5], 6);
    assert!((lifted[0] - anchor[0]).hypot(lifted[1] - anchor[1]) > 1.);
    assert_ne!(pants.skinned_shape_at(1), pants.skinned_shape_at(4));

    let selected = [boot.id, pants.id];
    let preset = ExportPreset {
        columns: 4,
        ..Default::default()
    };
    let folder = tmp.path().join("selection");
    let result = export_selection_animation(
        &folder,
        clip,
        &selected,
        SelectionRange {
            reference: 0,
            first: 0,
            last: 7,
        },
        &preset,
        SpriteOutput::Sheet,
        |_, _| true,
    )
    .unwrap();
    let sheet = Pixmap::load_png(folder.join("sheet.png")).unwrap();
    let [w, h] = result.size;
    let [x, y] = result.origin;
    let bounds = IntRect::from_xywh(x, y, w, h).unwrap();
    let mut isolated = doc.clone();
    isolated.objects.retain(|o| selected.contains(&o.id));
    for tick in 0..8 {
        let render = renderer::render(&isolated, tick)
            .unwrap()
            .clone_rect(bounds)
            .unwrap();
        let expected = Pixmap::decode_png(&render.encode_png().unwrap()).unwrap();
        let cell = IntRect::from_xywh((tick % 4 * w) as i32, (tick / 4 * h) as i32, w, h).unwrap();
        let actual = sheet.clone_rect(cell).unwrap();
        let difference: u32 = actual
            .data()
            .iter()
            .zip(expected.data())
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum();
        assert!(
            difference < 255,
            "Tick {tick}: selected and native renders differ by {difference}"
        );
    }
}

// Replace the rigged, switched boot with a plain placement. This oracle positions
// the heel directly from the contact chart and selects an ordinary child-clip frame.
fn reference_boot(plain: &VectorObject, far: bool, tick: u32, id: u64) -> VectorObject {
    let (ankle, angle, drawing) = boot_pose(far, tick);
    let mut boot = plain.clone();
    boot.id = id;
    boot.size = [32.; 2];
    boot.rotation = angle;
    let (sin, cos) = angle.to_radians().sin_cos();
    // The ankle in the 32px placement is (15,16), one pixel left of its center.
    boot.position = [ankle[0] + cos - 16., ankle[1] + sin - 16.];
    let instance = boot.clip_instance.as_mut().unwrap();
    instance.timing.mode = LoopMode::Clamp;
    instance.timing.offset = drawing as i32;
    instance.timing.rate_denominator = 4096;
    boot
}

#[test]
fn reopened_walk_exports_all_eight_sprites_with_the_authored_boot_drawings() {
    let source = project();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("walk.vanim");
    save_animation(&file, &source, false).unwrap();
    let mut reference = load_animation(&file).unwrap();
    assert_eq!(source, reference);
    let bundle = tmp.path().join("walk");
    export_sprite_bundle(&bundle, &reference.clips[0], &reference.preset, true).unwrap();
    let sheet = Pixmap::load_png(bundle.join("sheet.png")).unwrap();
    assert_eq!((sheet.width(), sheet.height()), (512, 256));
    let child = reference.clips[1].id;
    let layer = reference.clips[0].drawing_at(0).layers[0].id;
    let fresh = reference.place_clip(child, 0, 0, layer, [0., 0.]).unwrap();
    let doc = reference.clips[0].drawing_at_mut(0);
    let template = doc.objects.pop().unwrap();
    assert_eq!(template.id, fresh);
    let slots =
        ["Far boot", "Near boot"].map(|s| doc.objects.iter().position(|o| o.name == s).unwrap());
    let mut poses = Vec::new();
    for tick in 0..8 {
        for (slot, far) in slots.into_iter().zip([true, false]) {
            doc.objects[slot] = reference_boot(&template, far, tick, doc.objects[slot].id);
        }
        let expected = renderer::render(doc, tick).unwrap();
        let expected = Pixmap::decode_png(&expected.encode_png().unwrap()).unwrap();
        let frame = Pixmap::load_png(bundle.join(format!("frames/frame_{tick:04}.png"))).unwrap();
        let diff: u64 = expected
            .data()
            .iter()
            .zip(frame.data())
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        // Float IK and direct placement can round opposite sides of an antialias
        // sample. Allow less than four fully changed channel values per frame.
        assert!(
            diff < 4 * 255,
            "Tick {tick}: independent boot render differed by {diff} channel levels"
        );
        let cell =
            IntRect::from_xywh((tick % 4) as i32 * 128, (tick / 4) as i32 * 128, 128, 128).unwrap();
        assert!(sheet.clone_rect(cell).unwrap().data() == frame.data());
        assert!(
            !poses.contains(&frame),
            "Duplicate stride pose at tick {tick}"
        );
        poses.push(frame);
    }
}
