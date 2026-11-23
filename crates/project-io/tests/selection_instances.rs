use document_core::{AnimationProject, Document, SymbolDefinition};
use project_io::{export_object_pngs, load_animation, save_animation};
use std::sync::Arc;
use tiny_skia::Pixmap;

#[test]
fn exported_symbol_overhang_and_animated_clip_viewport_match_their_native_pixels() {
    let art = Document {
        width: 16,
        height: 16,
        frames: 1,
        ..Default::default()
    };
    let mut stripe = art.clone();
    stripe.add_rectangle([-10., 2.], [7., 8.], [240, 0, 0, 255]);
    stripe.add_rectangle([-2.5, 10.5], [4., 4.], [20, 60, 225, 255]);
    stripe.add_rectangle([4., 1.5], [5., 8.], [0, 165, 74, 220]);
    let mut project = AnimationProject::from_document(Document {
        width: 128,
        height: 128,
        ..Document::default()
    });
    let leaf = project
        .add_symbol(SymbolDefinition {
            name: "Stripe".into(),
            artwork: Arc::new(stripe),
        })
        .unwrap();
    let badge = project
        .add_symbol(SymbolDefinition {
            name: "Badge".into(),
            artwork: Arc::new(art),
        })
        .unwrap();
    project.nest_symbol(badge, leaf).unwrap();
    let child = project.add_clip("Sliding badge").unwrap();
    let clip_id = project.clips[child].id;
    assert_eq!(
        badge, clip_id,
        "symbol and clip IDs are scoped independently"
    );
    project.clips[child].set_length(3).unwrap();
    project.place_symbol(badge, child, 0, 1, [0., 0.]).unwrap();
    project.clips[child].drawing_at_mut(0).objects[0]
        .keys
        .extend([(0, [0., 0.]), (2, [1.25, 5.5])]);

    let symbol = project.place_symbol(badge, 0, 0, 1, [32.5, 31.25]).unwrap();
    let clip = project.place_clip(clip_id, 0, 0, 1, [72.25, 46.5]).unwrap();
    let drawing = project.clips[0].drawing_at_mut(0);
    for instance in &mut drawing.objects {
        instance.scale = [1.25, 0.875];
        instance.rotation = 19.;
    }
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("placements.vanim");
    save_animation(&file, &project, false).unwrap();
    let reopened = load_animation(&file).unwrap();
    let drawing = reopened.clips[0].drawing_at(0);
    for (tick, scale) in [(0, 1), (2, 3)] {
        let folder = temp.path().join(format!("tick-{tick}"));
        let assets =
            export_object_pngs(&folder, drawing, tick, &[clip, symbol], scale, |_, _| true)
                .unwrap();
        assert_eq!(assets.len(), 2);
        for asset in assets {
            let [placed] = asset.object_ids.as_slice() else {
                panic!("expected one placement")
            };
            let mut reference = drawing.clone();
            reference.objects.retain(|o| o.id == *placed);
            let exported = Pixmap::load_png(asset.path).unwrap();
            let red_overhang = exported
                .pixels()
                .iter()
                .any(|p| p.red() > 220 && p.green() == 0);
            assert_eq!(red_overhang, *placed == symbol);

            // Both artboards enclose all the paint. Native clip-mask antialiasing
            // can round an edge pixel differently when the raster extent changes.
            let mut matches_native = false;
            for (width, height) in [(128, 128), (91, 68)] {
                reference.width = width;
                reference.height = height;
                let full = renderer::render_scaled(&reference, tick, scale as f32).unwrap();
                let ink = renderer::ink_bounds(&full).unwrap();
                assert_eq!(asset.origin, [ink.left(), ink.top()]);
                assert_eq!(asset.size, [ink.width(), ink.height()]);
                let png = full.clone_rect(ink).unwrap().encode_png().unwrap();
                let native = Pixmap::decode_png(&png).unwrap();
                matches_native |= exported.pixels() == native.pixels();
            }
            assert!(
                matches_native,
                "placement {placed} at tick {tick}, scale {scale}"
            );
        }
    }
}
