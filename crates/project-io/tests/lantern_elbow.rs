use document_core::VectorObject;
use project_io::{export_sprite_bundle, export_svg, import_svg, load_animation, save_animation};
use std::path::Path;
use tiny_skia::{IntRect, Pixmap};

fn rotate(point: [f32; 2], pivot: [f32; 2], degrees: f32) -> [f32; 2] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let [x, y] = [point[0] - pivot[0], point[1] - pivot[1]];
    [pivot[0] + cos * x - sin * y, pivot[1] + sin * x + cos * y]
}

// Check the actual anchor and both Bézier handles against the two rotations drawn in the example.
fn check_sleeve_pose(posed: &VectorObject, tick: u32, strength: f32) {
    let weights = [0., 0.45, 1., 1., 0.65, 0.15, 0.];
    let shape = posed.evaluated_shape_at(tick);
    let rest = &posed.shape.contours()[0];
    let nodes = &shape.contours()[0];
    assert_eq!(nodes.len(), weights.len());
    for ((a, b), w) in rest.iter().zip(nodes).zip(weights) {
        for (rest, evaluated) in [
            (a.point, b.point),
            (a.incoming, b.incoming),
            (a.outgoing, b.outgoing),
        ] {
            let world = posed.local_to_world(rest, 0);
            let upper = rotate(world, [56., 70.], -7. * strength);
            let forearm = rotate(
                rotate(world, [70., 75.], -32. * strength),
                [56., 70.],
                -7. * strength,
            );
            let actual = posed.local_to_world(evaluated, tick);
            for i in 0..2 {
                let expected = upper[i] + (forearm[i] - upper[i]) * w;
                assert!(
                    (actual[i] - expected).abs() < 0.0002,
                    "Tick {tick}: got {actual:?}, expected axis {i} to be {expected}"
                );
            }
        }
    }
}

#[test]
fn skinned_sleeve_roundtrips_and_exports_as_posed_vectors_and_sprite_cells() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let source = load_animation(&examples.join("lantern-elbow.vanim")).unwrap();
    let keeper = load_animation(&examples.join("lantern-keeper.vanim")).unwrap();
    let out = tempfile::tempdir().unwrap();
    let saved = out.path().join("elbow.vanim");
    save_animation(&saved, &source, false).unwrap();
    let reopened = load_animation(&saved).unwrap();
    assert_eq!(reopened, source);
    let clip = &reopened.clips[0];
    assert_eq!((clip.frames, clip.fps, clip.cels.len()), (8, 8, 1));
    let doc = clip.drawing_at(0);
    let sleeve = doc.objects.iter().find(|o| o.skin.is_some()).unwrap();
    let original = keeper.clips[0].drawing_at(0);
    assert_eq!(
        sleeve.shape,
        original
            .objects
            .iter()
            .find(|o| o.id == sleeve.id)
            .unwrap()
            .shape
    );
    let rest = renderer::render(original, 0).unwrap();
    assert!(renderer::render(doc, 0).unwrap().data() == rest.data());
    let bundle = out.path().join("sleeve-sprites");
    export_sprite_bundle(&bundle, clip, &reopened.preset, true).unwrap();
    let sheet = Pixmap::load_png(bundle.join("sheet.png")).unwrap();
    let boots = IntRect::from_xywh(28, 105, 54, 11).unwrap();
    let planted = rest.clone_rect(boots).unwrap();
    for (tick, fraction) in [0., 0.25, 0.5, 0.75, 1., 2. / 3., 1. / 3., 0.]
        .into_iter()
        .enumerate()
    {
        let tick = tick as u32;
        check_sleeve_pose(sleeve, tick, fraction);
        let image = renderer::render(doc, tick).unwrap();
        assert!(image.clone_rect(boots).unwrap().data() == planted.data());
        let png = Pixmap::load_png(bundle.join(format!("frames/frame_{tick:04}.png"))).unwrap();
        let encoded = Pixmap::decode_png(&image.encode_png().unwrap()).unwrap();
        assert!(png.data() == encoded.data(), "Wrong PNG at tick {tick}");
        let cell =
            IntRect::from_xywh(128 * (tick % 4) as i32, 128 * (tick / 4) as i32, 128, 128).unwrap();
        assert!(sheet.clone_rect(cell).unwrap().data() == png.data());
    }
    // Reimport just the sleeve, so rigid hand and lamp artwork cannot mask an export error.
    let mut isolated = doc.clone();
    isolated.objects.retain(|o| o.id == sleeve.id);
    let svg = out.path().join("bent-sleeve.svg");
    export_svg(&svg, &isolated, 4, false).unwrap();
    let svg_doc = import_svg(&svg).unwrap();
    let expected = renderer::render(&isolated, 4).unwrap();
    let imported = renderer::render(&svg_doc, 0).unwrap();
    let difference: usize = expected
        .data()
        .iter()
        .zip(imported.data())
        .map(|(a, b)| usize::from(a.abs_diff(*b)))
        .sum();
    assert!(
        difference < 64,
        "SVG re-import changed the bent sleeve by {difference} channel levels"
    );
    assert!(
        expected
            .pixels()
            .iter()
            .zip(renderer::render(&isolated, 0).unwrap().pixels())
            .filter(|(a, b)| a != b)
            .count()
            > 250
    );
}

#[test]
fn edited_cuff_weight_survives_save_and_changes_only_the_posed_geometry() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-elbow.vanim");
    let mut project = load_animation(&source).unwrap();
    let doc = project.clips[0].drawing_at_mut(4);
    let sleeve = doc.objects.iter().find(|o| o.skin.is_some()).unwrap();
    let id = sleeve.id;
    let source_shape = sleeve.shape.clone();
    let cuff = sleeve.evaluated_shape_at(4).contours()[0][2].point;
    let posed = renderer::render(doc, 4).unwrap();
    doc.set_skin_weight(id, 0, 2, 0.).unwrap();

    let out = tempfile::tempdir().unwrap();
    let edited = out.path().join("loose-cuff.vanim");
    save_animation(&edited, &project, false).unwrap();
    let reopened = load_animation(&edited).unwrap();
    let doc = reopened.clips[0].drawing_at(4);
    let sleeve = doc.objects.iter().find(|o| o.id == id).unwrap();
    assert_eq!(sleeve.skin.as_ref().unwrap().weights[0][2], 0.);
    assert_eq!(sleeve.shape, source_shape);
    assert_ne!(sleeve.evaluated_shape_at(4).contours()[0][2].point, cuff);
    assert_ne!(renderer::render(doc, 4).unwrap().data(), posed.data());
}
