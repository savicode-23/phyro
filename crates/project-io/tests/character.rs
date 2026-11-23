use document_core::{Document, Shape};
use project_io::{export_frame, export_svg, import_svg, load_project, save_project};
use std::path::{Path, PathBuf};
use tiny_skia::Pixmap;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn mean_channel_difference(a: &Pixmap, b: &Pixmap) -> f64 {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let delta: u64 = a
        .data()
        .iter()
        .zip(b.data())
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    delta as f64 / a.data().len() as f64
}

fn layer_names(doc: &Document) -> Vec<&str> {
    doc.layers.iter().map(|layer| layer.name.as_str()).collect()
}

#[test]
fn example_keeps_editable_shapes_and_matches_inkscape_reference() {
    let doc = import_svg(&example("lantern-keeper.svg")).unwrap();
    assert_eq!(
        layer_names(&doc),
        ["Boots", "Coat", "Hood", "Lantern", "Sleeve"]
    );
    assert!(doc.objects.iter().any(|o| o.shape == Shape::Ellipse));
    assert!(doc.objects.iter().any(|o| o.shape == Shape::Rectangle));
    assert!(doc.objects.iter().all(|o| o.group.is_some()));
    // The original archive represented SVG's "none" as transparent black.
    let mut legacy = doc.clone();
    for object in &mut legacy.objects {
        if object.no_fill {
            object.no_fill = false;
            object.fill = [0; 4];
        }
        if object.no_stroke {
            object.no_stroke = false;
            object.stroke = [0; 4];
        }
    }
    assert_eq!(
        load_project(&example("lantern-keeper.vanim")).unwrap(),
        legacy
    );

    // Inkscape 1.4.2, 96 dpi, transparent background. Allow a small difference
    // on antialiased edges between Cairo and tiny-skia.
    let reference =
        Pixmap::decode_png(include_bytes!("fixtures/lantern-keeper-inkscape.png")).unwrap();
    let actual = renderer::render(&doc, 0).unwrap();
    let delta = mean_channel_difference(&actual, &reference);
    assert!(delta < 1.8, "mean premultiplied RGBA error: {delta}");
}

#[test]
fn edited_character_roundtrips_through_native_svg_and_png() {
    let mut doc = load_project(&example("lantern-keeper.vanim")).unwrap();
    let original = renderer::render(&doc, 0).unwrap();
    let layer = doc
        .layers
        .iter()
        .find(|layer| layer.name == "Hood")
        .unwrap()
        .id;
    let hood = doc.objects.iter_mut().find(|o| o.layer == layer).unwrap();
    let Shape::Path { nodes, .. } = &mut hood.shape else {
        panic!("hood must have editable anchors")
    };
    // Pull the tip of the hood outward, moving its cubic handles with it.
    let tip = &mut nodes[0];
    for point in [&mut tip.point, &mut tip.incoming, &mut tip.outgoing] {
        point[0] -= 7.0 / hood.size[0];
    }
    hood.fill = [135, 101, 154, 255];
    let dir = tempfile::tempdir().unwrap();
    let native = dir.path().join("keeper.vanim");
    let svg = dir.path().join("keeper.svg");
    let png = dir.path().join("keeper.png");
    save_project(&native, &doc).unwrap();
    let reopened = load_project(&native).unwrap();
    assert_eq!(reopened, doc);
    export_svg(&svg, &reopened, 0, false).unwrap();
    export_frame(&png, &reopened, 0).unwrap();
    let png = Pixmap::load_png(&png).unwrap();
    assert!(mean_channel_difference(&original, &png) > 0.5);

    let imported = import_svg(&svg).unwrap();
    assert_eq!(layer_names(&imported), layer_names(&reopened));
    let before = reopened.paint_order();
    let after = imported.paint_order();
    assert_eq!(before.len(), after.len());
    for (&a, &b) in before.iter().zip(&after) {
        assert_eq!(
            (a.fill, a.stroke, a.cap, a.join),
            (b.fill, b.stroke, b.cap, b.join)
        );
        assert!((a.stroke_width - b.stroke_width).abs() < 0.001);
        match (&a.shape, &b.shape) {
            (Shape::Rectangle, Shape::Rectangle) | (Shape::Ellipse, Shape::Ellipse) => {}
            (
                Shape::Path {
                    nodes: first,
                    closed,
                },
                Shape::Path {
                    nodes: second,
                    closed: again,
                },
            ) => {
                assert_eq!(closed, again);
                assert_eq!(first.len(), second.len());
                for (u, v) in first.iter().zip(second) {
                    for (left, right) in [
                        (u.point, v.point),
                        (u.incoming, v.incoming),
                        (u.outgoing, v.outgoing),
                    ] {
                        let [x, y] = a.local_to_world(left, 0);
                        let [rx, ry] = b.local_to_world(right, 0);
                        assert!(
                            (x - rx).hypot(y - ry) < 0.001,
                            "path node or control point moved"
                        );
                    }
                }
            }
            _ => panic!("SVG changed the editable shape type"),
        }
    }
    let delta = mean_channel_difference(&renderer::render(&imported, 0).unwrap(), &png);
    assert!(
        delta < 0.1,
        "SVG changed the exported character pixels: {delta}"
    );
}
