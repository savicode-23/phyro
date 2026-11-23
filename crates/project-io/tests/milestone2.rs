use project_io::{export_sprite_bundle, load_animation};
use std::path::Path;
use tiny_skia::Pixmap;

#[test]
fn native_sample_recreates_the_godot_bundle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = load_animation(&root.join("examples/lantern-watch.vanim")).unwrap();
    let swing = &project.clips[0];
    assert_eq!(swing.name, "Lantern swing");
    assert_eq!((swing.frames, swing.fps, swing.cels.len()), (8, 8, 8));
    assert_eq!(swing.origin, [56., 114.]);
    assert_eq!(project.presets["Godot four columns"], project.preset);

    let temp = tempfile::tempdir().unwrap();
    let exported = temp.path().join("bundle");
    export_sprite_bundle(&exported, swing, &project.preset, true).unwrap();
    let shipped = root.join("integrations/godot/art/lantern-swing");
    let read_json = |path| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    };
    assert_eq!(
        read_json(exported.join("sheet.json")),
        read_json(shipped.join("sheet.json"))
    );
    for name in std::iter::once("sheet.png".to_owned())
        .chain((0..8).map(|i| format!("frames/frame_{i:04}.png")))
    {
        let actual = Pixmap::load_png(exported.join(&name)).unwrap();
        let reference = Pixmap::load_png(shipped.join(&name)).unwrap();
        assert_eq!(
            (actual.width(), actual.height()),
            (reference.width(), reference.height())
        );
        // Small differences on antialiased edges can occur across CPU architectures.
        let total: u64 = actual
            .data()
            .iter()
            .zip(reference.data())
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        assert!(
            (total as f64 / actual.data().len() as f64) < 0.01,
            "{name} differs from the sample"
        );
    }

    let signal = &project.clips[1];
    assert_eq!(signal.name, "Signal");
    assert_eq!(signal.cels.len(), 1);
    let flame = signal
        .drawing_at(0)
        .objects
        .iter()
        .find(|o| !o.opacity_keys.is_empty())
        .unwrap();
    assert!(
        !flame.scale_keys.is_empty() && !flame.rotation_keys.is_empty() && !flame.keys.is_empty()
    );
    let first = renderer::render(signal.drawing_at(0), 0).unwrap();
    let peak = renderer::render(signal.drawing_at(3), 3).unwrap();
    assert!(first.data() != peak.data());
}
