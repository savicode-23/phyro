use document_core::{
    Document, FillRule, Gradient, GradientKind, GradientStop, Layer, Shape, Stencil,
};
use project_io::{export_object_pngs, export_object_svgs, export_selection_png, import_svg};
use std::{fs, io::ErrorKind};
use tiny_skia::Pixmap;

#[test]
fn references_remain_visible_in_the_editor_and_stay_out_of_nested_exports() {
    use document_core::{AnimationProject, RasterImage};
    use project_io::{
        export_new_frame, export_svg, load_animation, render_sheet_preview, save_animation,
    };
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document {
        width: 16,
        height: 16,
        frames: 1,
        ..Default::default()
    };
    let reference = doc.add_rectangle([0., 0.], [8., 8.], [0; 4]);
    let ink = doc.add_rectangle([9., 10.], [2., 3.], [0, 255, 0, 255]);
    doc.objects[0].image = Some(Arc::new(RasterImage::from_base64(
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAF0lEQVR4nGP4z8DwnwEM/jM0MDD8/w8AK2kFfIvAcEgAAAAASUVORK5CYII="
    ).unwrap()));
    doc.objects[0].reference_only = true;
    assert_eq!(doc.hit_test([1., 1.], 0), Some(reference));
    let batch = export_object_svgs(
        &dir.path().join("batch"),
        &doc,
        0,
        &[reference, ink],
        |_, _| true,
    )
    .unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].object_ids, [ink]);
    assert!(
        export_selection_png(&dir.path().join("ref.png"), &doc, 0, &[reference], 1, false).is_err()
    );

    let mut project = AnimationProject::from_document(doc);
    let (_, placed) = project
        .convert_to_symbol(0, 0, &[reference, ink], "Paint")
        .unwrap();
    let path = dir.path().join("mixed.vanim");
    save_animation(&path, &project, false).unwrap();
    let reopened = load_animation(&path).unwrap();
    let clip = &reopened.clips[0];
    let scene = clip.drawing_at(0);
    assert!(
        renderer::render(scene, 0)
            .unwrap()
            .pixel(1, 1)
            .unwrap()
            .alpha()
            > 0
    );
    let frame = dir.path().join("frame.png");
    export_new_frame(&frame, scene, 0).unwrap();
    let png = Pixmap::load_png(frame).unwrap();
    assert_eq!(png.pixel(1, 1).unwrap().alpha(), 0);
    assert_eq!(png.pixel(10, 11).unwrap().green(), 255);
    let sheet = render_sheet_preview(clip, &reopened.preset, |_, _| true).unwrap();
    assert_eq!(sheet.pixels().iter().filter(|p| p.alpha() != 0).count(), 6);
    let crop =
        export_selection_png(&dir.path().join("paint.png"), scene, 0, &[placed], 1, false).unwrap();
    assert_eq!((crop.origin, crop.size), ([9, 10], [2, 3]));
    let svg = dir.path().join("frame.svg");
    export_svg(&svg, scene, 0, false).unwrap();
    assert!(!fs::read_to_string(svg).unwrap().contains("data:image/"));
}

#[test]
fn pasteboard_selection_preserves_the_native_painted_crop_at_the_requested_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::default();
    let base = doc.add_rectangle([-14.25, -4.5], [15., 11.], [28, 194, 128, 201]);
    doc.objects[0].group = Some(4);
    doc.objects[0]
        .keys
        .extend([(0, [-14.25, -4.5]), (2, [-11.25, -2.5])]);
    let middle = doc.add_rectangle([-7., -1.], [10., 10.], [0, 0, 180, 221]);
    doc.add_shape([-9.5, -2.25], [15., 14.], [0; 4], Shape::Ellipse, 1);
    let masked = &mut doc.objects[2];
    masked.group = Some(4);
    masked.opacity = 0.8;
    masked.gradient = Some(Gradient {
        kind: GradientKind::Linear {
            start: [0., 0.],
            end: [1., 1.],
        },
        stops: vec![
            GradientStop {
                offset: 0.,
                color: [200, 80, 220, 210],
            },
            GradientStop {
                offset: 1.,
                color: [240, 110, 40, 255],
            },
        ],
    });
    masked.stroke = [205, 100, 127, 240];
    masked.stroke_width = 3.;
    masked.stencil = Some(Stencil {
        shape: Shape::Rectangle,
        fill_rule: FillRule::NonZero,
        origin: [-0.2, 0.1],
        size: [0.94, 0.97],
        alpha: Some([255, 32]),
    });
    let masked_id = masked.id;
    // This object overlaps the selection but belongs to neither requested asset.
    doc.add_rectangle([-5., -1.], [7., 8.], [255, 0, 255, 255]);
    // Moving native Bézier controls by an integer can quantize one edge sample differently.
    // Two full native renders measure that variation independently of the cropped painter.
    let on_page = |offset: f32| {
        let mut copy = doc.clone();
        copy.objects.pop();
        for object in &mut copy.objects {
            object.position = object.position.map(|x| x + offset);
            for key in object.keys.values_mut() {
                *key = key.map(|x| x + offset);
            }
        }
        copy
    };
    let native_crop = |doc: &Document, scale: u32| {
        let page = renderer::render_scaled(doc, 2, scale as f32).unwrap();
        let bounds = renderer::ink_bounds(&page).unwrap();
        let png = page.clone_rect(bounds).unwrap().encode_png().unwrap();
        (Pixmap::decode_png(&png).unwrap(), bounds)
    };
    let reference = on_page(32.);
    for scale in [1, 2] {
        let path = dir.path().join(format!("selected-{scale}.png"));
        let asset = export_selection_png(&path, &doc, 2, &[middle, base], scale, false).unwrap();
        let (native, bounds) = native_crop(&reference, scale);
        let (translated, second_bounds) = native_crop(&on_page(63.), scale);
        assert_eq!(asset.path, path);
        assert_eq!(asset.object_ids, [base, middle, masked_id]);
        assert_eq!(
            asset.origin,
            [
                bounds.x() - 32 * scale as i32,
                bounds.y() - 32 * scale as i32
            ]
        );
        assert_eq!(asset.size, [bounds.width(), bounds.height()]);
        assert_eq!(asset.size, [second_bounds.width(), second_bounds.height()]);
        let actual = Pixmap::load_png(&path).unwrap();
        let on_page_path = dir.path().join(format!("on-page-{scale}.png"));
        let same =
            export_selection_png(&on_page_path, &reference, 2, &[base, middle], scale, false)
                .unwrap();
        assert_eq!(same.origin, [bounds.x(), bounds.y()]);
        assert_eq!(same.size, asset.size);
        let same = Pixmap::load_png(&on_page_path).unwrap();

        for (index, (&a, &b)) in native.data().iter().zip(translated.data()).enumerate() {
            let value = actual.data()[index];
            // Allow a single gradient rounding level beyond the range of the two
            // native samples. At identical coordinates the silhouette is exact.
            assert!(a.abs_diff(b) <= 8);
            assert!(value >= a.min(b).saturating_sub(1) && value <= a.max(b).saturating_add(1));
            let tolerance = if index % 4 == 3 { 0 } else { 1 };
            assert!(same.data()[index].abs_diff(a) <= tolerance);
        }
    }
}

#[test]
fn batches_expand_groups_once_per_layer_and_disambiguate_portable_filenames() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sprites");
    let mut doc = Document::default();
    doc.layers.push(Layer {
        id: 2,
        name: "Foreground".into(),
        locked: true,
        visible: true,
    });
    let ids: Vec<_> = (0..7)
        .map(|i| doc.add_rectangle([-24. + i as f32 * 3., 140.], [3., 3.], [255; 4]))
        .collect();
    for (o, name) in doc
        .objects
        .iter_mut()
        .zip(["CON.txt", "../Arm?", "arm", "../ARM?", "", "", ""])
    {
        o.name = name.into();
    }
    doc.objects[4].group = Some(7);
    doc.objects[5].group = Some(7);
    doc.objects[6].group = Some(7);
    doc.objects[6].layer = 2;
    doc.rename_group(1, 7, "Arm").unwrap();
    let mut progress = Vec::new();
    let assets = export_object_pngs(
        &out,
        &doc,
        0,
        &[ids[6], ids[4], ids[3], ids[1], ids[0], ids[2], ids[4]],
        2,
        |done, total| {
            progress.push((done, total));
            true
        },
    )
    .unwrap();
    assert_eq!(assets.len(), 6);
    assert_eq!(progress, (0..=6).map(|n| (n, 6)).collect::<Vec<_>>());
    assert_eq!(
        assets
            .iter()
            .map(|a| a.path.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "_CON.txt.png",
            "_Arm_.png",
            "arm.png",
            "_ARM_ (2).png",
            "Arm (2).png",
            "Group 7.png"
        ]
    );
    assert_eq!(assets[4].object_ids, [ids[4], ids[5]]);
    assert_eq!(assets[5].object_ids, [ids[6]]);
    for (i, asset) in assets.iter().enumerate() {
        assert_eq!(asset.path.parent(), Some(out.as_path()));
        assert_eq!(asset.origin[1], 280);
        assert_eq!(asset.size, if i == 4 { [12, 6] } else { [6, 6] });
        assert!(Pixmap::load_png(&asset.path)
            .unwrap()
            .pixels()
            .iter()
            .all(|p| p.alpha() == 255));
    }
}

#[test]
fn svg_batch_uses_layer_names_for_each_selection_unit_and_keeps_curves_off_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::default();
    let body = doc.add_rectangle([-60., -28.], [12., 14.], [26, 35, 74, 255]);
    let unrelated = doc.add_rectangle([-28., -16.], [20., 25.], [255, 0, 255, 255]);
    let mut nodes =
        [[0., 0.], [1., 0.2], [0.6, 1.], [0., 0.7]].map(document_core::PathNode::corner);
    nodes[0].outgoing = [0.3, 0.];
    nodes[1].incoming = [0.95, 0.];
    let wing = doc.add_shape(
        [-43., -24.],
        [10., 10.],
        [5, 145, 67, 255],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: true,
        },
        1,
    );
    doc.objects[0].group = Some(5);
    doc.objects[2].group = Some(5);
    doc.rename_group(1, 5, "Wing & trim").unwrap();
    doc.objects[0]
        .keys
        .extend([(0, [-60., -28.]), (2, [-56., -31.])]);
    doc.objects[0].stroke = [240, 47, 29, 255];
    doc.objects[0].stroke_width = 2.;
    let single = doc.add_rectangle([210., 80.], [4., 4.], [80; 4]);
    doc.objects.last_mut().unwrap().name = "Wing & trim".into();
    let portable = doc.add_rectangle([200., 80.], [3., 3.], [99; 4]);
    doc.objects.last_mut().unwrap().name = "Bell/left?".into();
    let unnamed = doc.add_rectangle([8., 12.], [40., 10.], [12; 4]);
    let destination = dir.path().join("objects");
    let mut progress = Vec::new();
    let results = export_object_svgs(
        &destination,
        &doc,
        2,
        &[single, wing, portable, unnamed, wing],
        |done, total| {
            progress.push((done, total));
            true
        },
    )
    .unwrap();
    assert_eq!(progress, (0..=4).map(|done| (done, 4)).collect::<Vec<_>>());
    assert!(results
        .iter()
        .all(|asset| !asset.object_ids.contains(&unrelated)));
    assert_eq!(results[0].object_ids, [body, wing]);
    assert_eq!(results[0].path, destination.join("Wing & trim.svg"));
    assert_eq!(results[1].path, destination.join("Wing & trim (2).svg"));
    assert_eq!(results[2].path, destination.join("Bell_left_.svg"));
    assert_eq!(
        results[3].path,
        destination.join(format!("Rectangle {unnamed}.svg"))
    );
    assert_eq!(results[0].origin, [-57, -32]);
    assert_eq!(results[0].size, [24, 18]);
    let svg = fs::read_to_string(&results[0].path).unwrap();
    assert!(svg.contains(r#"viewBox="-57 -32 24 18""#));
    assert!(svg.contains(r#"inkscape:label="Wing &amp; trim""#));
    assert!(!svg.contains("<image") && !svg.contains("#ff00ff"));
    let imported = import_svg(&results[0].path).unwrap();
    assert_eq!([imported.width, imported.height], results[0].size);
    assert_eq!(imported.objects.len(), 2);
    assert!(imported
        .objects
        .iter()
        .all(|o| o.group_name.as_deref() == Some("Wing & trim")));
    assert!(
        matches!(&imported.objects[1].shape, Shape::Path { nodes, .. } if nodes.len() == 4 && nodes.iter().any(|n| n.outgoing != n.point))
    );
    assert_eq!(imported.objects[0].position, [1., 1.]);
}

#[test]
fn svg_assets_keep_shared_masks_and_batch_publication_is_all_or_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::default();
    let a = doc.add_rectangle([8., 10.], [37., 35.], [22, 105, 138, 255]);
    let b = doc.add_rectangle([32., 13.], [24., 20.], [200, 72, 50, 255]);
    for object in &mut doc.objects {
        object.group = Some(3);
    }
    doc.rename_group(1, 3, "Glow").unwrap();
    let mask = doc.add_shape([17., 15.], [32., 20.], [255; 4], Shape::Ellipse, 1);
    doc.set_clip(&[a, b, mask], 0, Some([127, 255])).unwrap();
    let destination = dir.path().join("parts");
    let result = export_object_svgs(&destination, &doc, 0, &[b], |_, _| true).unwrap();
    let asset = &result[0];
    // Capturing the ellipse in each target's coordinates can move its
    // mathematical edge by one floating-point ULP before outward rounding.
    assert!((16..=17).contains(&asset.origin[0]) && (14..=15).contains(&asset.origin[1]));
    let svg = fs::read_to_string(&asset.path).unwrap();
    assert_eq!(svg.matches(" mask=\"url(").count(), 1);
    let imported = import_svg(&asset.path).unwrap();
    assert_eq!(imported.objects.len(), 2);
    assert!(imported.objects.iter().all(|o| o.stencil.is_some()));
    let native = renderer::render(&doc, 0)
        .unwrap()
        .clone_rect(
            tiny_skia::IntRect::from_xywh(
                asset.origin[0],
                asset.origin[1],
                asset.size[0],
                asset.size[1],
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!([imported.width, imported.height], asset.size);
    // Use a spare raster column and row to measure the imported outline. A
    // sub-pixel rounding difference on the ellipse's final control point can
    // otherwise change tiny-skia's clipping samples right at the canvas edge.
    let mut padded = imported;
    padded.width += 1;
    padded.height += 1;
    let image = renderer::render(&padded, 0).unwrap();
    let [w, h] = asset.size;
    assert!((0..=h).all(|y| image.pixel(w, y).unwrap().alpha() == 0));
    assert!((0..=w).all(|x| image.pixel(x, h).unwrap().alpha() == 0));
    let roundtrip = image
        .clone_rect(tiny_skia::IntRect::from_xywh(0, 0, w, h).unwrap())
        .unwrap();
    assert_eq!(native.data(), roundtrip.data());
    let fresh = dir.path().join("canceled");
    let huge = doc.add_rectangle([-20_000., -2_000.], [10_000., 8_000.], [42; 4]);
    assert_eq!(
        export_object_svgs(&fresh, &doc, 0, &[a, huge], |done, _| done < 1)
            .unwrap_err()
            .kind(),
        ErrorKind::Interrupted
    );
    assert!(!fresh.exists());
    let large = export_object_svgs(&fresh, &doc, 0, &[huge], |_, _| true).unwrap();
    assert_eq!(large[0].size, [10_000, 8_000]);
    assert!(fs::metadata(&large[0].path).unwrap().len() < 1024);
    assert_eq!(
        export_object_svgs(&destination, &doc, 0, &[a], |_, _| true)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
}

#[test]
fn cancellation_hidden_artwork_and_failed_writes_preserve_existing_assets() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sprites");
    let mut doc = Document::default();
    let a = doc.add_rectangle([-12., -8.], [8., 6.], [255; 4]);
    let b = doc.add_rectangle([250., 202.], [8., 4.], [160; 4]);
    doc.layers.push(Layer {
        id: 2,
        name: "Hidden".into(),
        locked: false,
        visible: false,
    });
    doc.objects[1].layer = 2;
    doc.objects[1].name = "Invisible".into();
    let err = export_object_pngs(&out, &doc, 0, &[a, b], 1, |_, _| true).unwrap_err();
    assert!(err.to_string().contains("Invisible") && !out.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    let single = dir.path().join("single.png");
    let asset = export_selection_png(&single, &doc, 0, &[a, b], 1, false).unwrap();
    assert_eq!(asset.origin, [-12, -8]);
    assert_eq!(asset.size, [8, 6]);
    let original = fs::read(&single).unwrap();
    assert!(export_selection_png(&single, &doc, 0, &[b], 1, true).is_err());
    assert_eq!(
        export_selection_png(&single, &doc, 0, &[a], 1, false)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&single).unwrap(), original);
    assert!(export_selection_png(&single, &doc, 0, &[9999], 1, true)
        .unwrap_err()
        .to_string()
        .contains("9999"));

    doc.layers[1].visible = true;
    assert_eq!(
        export_object_pngs(&out, &doc, 0, &[a, b], 1, |done, _| done < 1)
            .unwrap_err()
            .kind(),
        ErrorKind::Interrupted
    );
    assert!(!out.exists());
    fs::create_dir(&out).unwrap();
    fs::write(out.join("mine.png"), b"already here").unwrap();
    assert_eq!(
        export_object_pngs(&out, &doc, 0, &[a, b], 1, |_, _| true)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
    assert_eq!(fs::read(out.join("mine.png")).unwrap(), b"already here");
}

#[test]
fn large_pasteboard_selections_allocate_from_their_actual_masked_extent() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::default();
    let id = doc.add_rectangle([-5000., -5000.], [10000.; 2], [255; 4]);
    let file = dir.path().join("masked.png");
    let too_large = export_selection_png(&file, &doc, 0, &[id], 1, false).unwrap_err();
    assert!(too_large.to_string().contains("16 megapixel") && !file.exists());
    doc.objects[0].stencil = Some(Stencil {
        shape: Shape::Rectangle,
        fill_rule: FillRule::NonZero,
        origin: [0.25, 0.75],
        size: [0.001, 0.002],
        alpha: None,
    });
    let asset = export_selection_png(&file, &doc, 0, &[id], 1, false).unwrap();
    assert_eq!(asset.origin, [-2500, 2500]);
    assert_eq!(asset.size, [10, 20]);
    assert!(Pixmap::load_png(&file)
        .unwrap()
        .data()
        .iter()
        .all(|&b| b == 255));
}
