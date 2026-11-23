#[path = "../examples/edit_godot_icon/workflow.rs"]
mod workflow;

use document_core::Shape;
use project_io::{import_svg, load_animation};
use std::path::Path;
use tiny_skia::Pixmap;

fn assert_fidelity(render: &Pixmap, reference: &Pixmap, name: &str) {
    assert_eq!(
        (render.width(), render.height()),
        (reference.width(), reference.height())
    );
    let total: u64 = render
        .data()
        .iter()
        .zip(reference.data())
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let error = total as f64 / render.data().len() as f64;
    println!("{name}: mean premultiplied RGBA error {error:.3} / 255");
    assert!(error < 1.8, "{name}: mean error {error}");
}

#[test]
fn masked_pressure_stroke_from_a0615ee_matches_inkscape() {
    // Keep the SVG produced by the older build so a change to the exporter
    // cannot hide an incompatibility with drawings already on disk.
    let svg = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/masked-pressure-stroke-a0615ee.svg");
    let drawing = import_svg(&svg).unwrap();
    let actual = renderer::render(&drawing, 0).unwrap();
    let expected = Pixmap::decode_png(include_bytes!(
        "fixtures/masked-pressure-stroke-inkscape.png"
    ))
    .unwrap();
    assert_fidelity(&actual, &expected, "masked pressure stroke");
    // Sample inside the translucent stroke, in the overlapping fill, and outside the mask.
    for (x, y) in [(50, 55), (72, 52), (85, 55), (100, 44)] {
        let pixel = actual.pixel(x, y).unwrap();
        let reference = expected.pixel(x, y).unwrap();
        for (a, b) in [
            (pixel.red(), reference.red()),
            (pixel.green(), reference.green()),
            (pixel.blue(), reference.blue()),
            (pixel.alpha(), reference.alpha()),
        ] {
            assert!(
                a.abs_diff(b) <= 6,
                "pixel ({x}, {y}): {pixel:?} vs {reference:?}"
            );
        }
    }
}

#[test]
fn nested_inkscape_artwork_preserves_editable_parts_and_interleaved_paint_order() {
    let svg = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nested-binoculars.svg");
    let mut drawing = import_svg(&svg).unwrap();
    // The two lens groups are interleaved with loose pieces from Housing.
    // Reflections is nested one level deeper inside Right lens.
    let groups = |doc: &document_core::Document| {
        doc.paint_order()
            .iter()
            .map(|o| o.group_name.as_deref().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let expected_groups = [
        "Housing",
        "Left lens",
        "Left lens",
        "Left lens",
        "Housing",
        "Right lens",
        "Right lens",
        "Reflections",
        "Reflections",
        "Housing",
    ];
    assert_eq!(groups(&drawing), expected_groups);
    assert!(drawing.objects.iter().all(|o| !o.is_instance()));
    assert_eq!(drawing.objects[0].group, drawing.objects[9].group);
    let reference =
        Pixmap::decode_png(include_bytes!("fixtures/nested-binoculars-inkscape.png")).unwrap();
    assert_fidelity(
        &renderer::render(&drawing, 0).unwrap(),
        &reference,
        "nested binoculars",
    );
    let focus = drawing.hit_test([55., 24.], 0).unwrap();
    assert_eq!(focus, drawing.objects[9].id);
    let Shape::Path { nodes, closed } = &mut drawing.objects[9].shape else {
        panic!("the foreground path must be directly editable")
    };
    assert!(document_core::path_edit::subdivide(nodes, *closed, 0));
    drawing.objects[9].fill = [250, 31, 24, 255];
    let edited = renderer::render(&drawing, 0).unwrap();
    assert_eq!(edited.pixel(55, 24).unwrap().red(), 250);

    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("binoculars.vanim");
    project_io::save_project(&native, &drawing).unwrap();
    let restored = project_io::load_project(&native).unwrap();
    assert_eq!(restored, drawing);
    let reexport = dir.path().join("binoculars.svg");
    project_io::export_svg(&reexport, &restored, 0, false).unwrap();
    let imported = import_svg(&reexport).unwrap();
    assert_eq!(groups(&imported), expected_groups);
    assert_eq!(imported.objects[0].group, imported.objects[9].group);
    assert!(imported.objects.iter().all(|o| !o.is_instance()));
    assert_fidelity(
        &renderer::render(&imported, 0).unwrap(),
        &edited,
        "edited nested re-export",
    );
}

#[test]
fn godot_icon_matches_independent_source_and_edited_renders() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sample = root.join("examples/godot-icon");
    let original = import_svg(&sample.join("source.svg")).unwrap();
    assert_eq!(original.objects.len(), 8);
    assert_eq!(original.objects[0].shape.contours()[0].len(), 8);
    assert!(
        matches!(&original.objects[2].shape, Shape::Compound { contours } if contours.len() == 2)
    );
    let initial = renderer::render(&original, 0).unwrap();
    let expected = Pixmap::decode_png(include_bytes!("fixtures/godot-icon-inkscape.png")).unwrap();
    assert_fidelity(&initial, &expected, "original");

    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("regenerated");
    workflow::generate(&output).unwrap();
    let project = load_animation(&output.join("edited.vanim")).unwrap();
    let edited = project.clips[0].drawing_at(0);
    assert_eq!(edited.objects.len(), 8);
    assert_eq!(edited.objects[0].fill, [49, 86, 87, 255]);
    assert_ne!(edited.objects[3].shape, original.objects[3].shape);
    assert!(matches!(&edited.objects[7].shape, Shape::Compound { contours } if contours.len()==2));
    let reference =
        Pixmap::decode_png(include_bytes!("fixtures/godot-icon-edit-inkscape.png")).unwrap();
    let native = renderer::render(edited, 0).unwrap();
    let exported_svg = import_svg(&output.join("edited.svg")).unwrap();
    let vector = renderer::render(&exported_svg, 0).unwrap();
    let png = Pixmap::load_png(output.join("edited.png")).unwrap();
    let shipped = load_animation(&sample.join("edited.vanim")).unwrap();
    let bundled = renderer::render(shipped.clips[0].drawing_at(0), 0).unwrap();
    for (name, result) in [
        ("native", &native),
        ("SVG", &vector),
        ("PNG", &png),
        ("bundled project", &bundled),
    ] {
        assert_fidelity(result, &reference, name);
        // The catchlight should be white, with the original dark pupil visible on its right.
        assert!(
            result.pixel(83, 64).unwrap().red() > 245,
            "{name} lost the hole"
        );
        assert!(
            result.pixel(87, 66).unwrap().red() < 80,
            "{name} lost the pupil"
        );
    }
}

#[test]
fn successive_ellipse_cutouts_keep_editable_curves_in_native_and_svg_files() {
    use document_core::{path_edit, BooleanOp, Document};
    let mut drawing = Document::default();
    let ellipse = path_edit::primitive_path(&Shape::Ellipse).unwrap();
    let base = drawing.add_shape(
        [8., 16.],
        [100., 82.],
        [52, 119, 87, 255],
        ellipse.clone(),
        1,
    );
    for x in [28., 67.] {
        let cutter = drawing.add_shape([x, 45.], [16., 14.], [255; 4], ellipse.clone(), 1);
        assert_eq!(
            drawing
                .boolean(&[cutter, base], 0, BooleanOp::Difference)
                .unwrap(),
            [base]
        );
    }
    let folder = tempfile::tempdir().unwrap();
    let project = folder.path().join("two-holes.vanim");
    project_io::save_project(&project, &drawing).unwrap();
    let saved = project_io::load_project(&project).unwrap();
    assert_eq!(saved, drawing);
    let svg = folder.path().join("two-holes.svg");
    project_io::export_svg(&svg, &saved, 0, false).unwrap();
    let imported = import_svg(&svg).unwrap();
    for doc in [&saved, &imported] {
        assert_eq!(doc.objects.len(), 1);
        let contours = doc.objects[0].shape.contours();
        assert_eq!(contours.len(), 3);
        for nodes in contours {
            assert_eq!(
                nodes.len(),
                4,
                "each ellipse stays editable at its four anchors"
            );
            assert!(nodes
                .iter()
                .all(|n| n.incoming != n.point && n.outgoing != n.point));
        }
        let rendered = renderer::render(doc, 0).unwrap();
        for x in [36, 75] {
            assert_eq!(rendered.pixel(x, 52).unwrap().alpha(), 0);
        }
        assert_eq!(rendered.pixel(55, 52).unwrap().alpha(), 255);
    }
}
