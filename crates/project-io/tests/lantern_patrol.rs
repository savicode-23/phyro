use document_core::{Easing, Property};
use project_io::{export_packed_atlas, load_animation};
use serde_json::Value;
use std::path::Path;
use tiny_skia::Pixmap;

#[test]
fn patrol_mixes_held_drawings_and_curved_motion_and_recreates_the_godot_atlas() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = load_animation(&root.join("examples/lantern-patrol.vanim")).unwrap();
    let clip = &project.clips[0];
    assert_eq!(
        (clip.name.as_str(), clip.frames, clip.fps),
        ("Lantern patrol", 12, 12)
    );
    assert_eq!(
        clip.cels.keys().copied().collect::<Vec<_>>(),
        [0, 2, 3, 4, 6, 7, 9, 10]
    );
    assert_eq!(clip.events[&2], ["swing_left", "bell"]);
    assert_eq!(clip.origin, [56., 114.]);
    assert!(project.preset.packed.enabled && project.preset.packed.allow_rotation);
    let scene = clip.drawing_at(0);
    let flame = scene
        .objects
        .iter()
        .find(|o| o.fill == [255, 240, 166, 255])
        .unwrap();
    assert!(flame.motion.is_some());
    assert!(matches!(
        Property::MotionProgress.easing_at(flame, 0),
        Easing::Cubic { .. }
    ));
    assert!(flame.animated.geometry_keys.len() >= 3);
    assert!(!flame.animated.progress_keys.is_empty());
    assert!(std::ptr::eq(scene, clip.drawing_at(1)));
    assert_ne!(flame.motion_offset_at(0), flame.motion_offset_at(1));
    let first = renderer::render(scene, 0).unwrap();
    let second = renderer::render(scene, 1).unwrap();
    assert!(first.data() != second.data());

    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("atlas");
    export_packed_atlas(&bundle, clip, &project.preset).unwrap();
    let shipped = root.join("integrations/godot/art/lantern-patrol");
    let read = |path| -> Value { serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() };
    let meta = read(bundle.join("animation.phyro"));
    assert_eq!(meta, read(shipped.join("animation.phyro")));
    assert_eq!(meta, read(shipped.join("sheet.json")));
    let pages = meta["images"].as_array().unwrap();
    assert!(pages.len() > 1);
    assert!(meta["frames"]
        .as_array()
        .unwrap()
        .iter()
        .any(|frame| frame["rotated"] == true));
    for page in pages {
        let name = page.as_str().unwrap();
        let actual = Pixmap::load_png(bundle.join(name)).unwrap();
        let reference = Pixmap::load_png(shipped.join(name)).unwrap();
        assert_eq!(
            (actual.width(), actual.height()),
            (reference.width(), reference.height())
        );
        let total: u64 = actual
            .data()
            .iter()
            .zip(reference.data())
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        assert!(
            (total as f64 / actual.data().len() as f64) < 0.01,
            "{name} changed"
        );
    }
}
