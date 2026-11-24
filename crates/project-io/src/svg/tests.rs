use super::*;
use document_core::{LineCap, LineJoin, Shape};
use std::fs;

#[test]
fn svg_blending_keeps_layers_objects_and_editable_paint_channels() {
    use document_core::{BlendMode, Document, Gradient, GradientKind, Layer};

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("blend.svg");
    let mut doc = Document::default();
    doc.add_rectangle([0.; 2], [96.; 2], [160, 111, 145, 255]);
    doc.layers.push(Layer {
        id: 2,
        name: "Lighting".into(),
        blend_mode: BlendMode::Screen,
        ..Layer::default()
    });
    doc.add_shape([20.; 2], [40.; 2], [180, 175, 91, 255], Shape::Rectangle, 2);
    let object = &mut doc.objects[1];
    object.name = "Light".into();
    object.blend_mode = BlendMode::Overlay;
    object.fill_blend_mode = BlendMode::Multiply;
    object.stroke_blend_mode = BlendMode::Difference;
    object.opacity = 0.56;
    object.stroke_width = 6.;
    object.stroke = [61, 15, 10, 255];
    object.gradient = Some(Gradient::from_color(
        GradientKind::Linear {
            start: [0.; 2],
            end: [1.; 2],
        },
        [120, 60, 200, 245],
    ));
    export_svg(&file, &doc, 0, false).unwrap();
    let svg = fs::read_to_string(&file).unwrap();
    for mode in ["screen", "overlay", "multiply", "difference"] {
        assert!(svg.contains(&format!("style=\"mix-blend-mode:{mode}\"")));
    }
    let restored = import_svg(&file).unwrap();
    assert_eq!(restored.layers[1].blend_mode, BlendMode::Screen);
    assert_eq!(restored.objects.len(), 2);
    let light = &restored.objects[1];
    assert_eq!(light.name, "Light");
    assert_eq!(light.blend_mode, BlendMode::Overlay);
    assert_eq!(light.fill_blend_mode, BlendMode::Multiply);
    assert_eq!(light.stroke_blend_mode, BlendMode::Difference);
    assert_eq!(light.opacity, 0.56);
    assert!(light.gradient.is_some());
    assert_eq!(
        renderer::render(&restored, 0).unwrap().data(),
        renderer::render(&doc, 0).unwrap().data()
    );
}

#[test]
fn svg_preserves_none_and_fully_transparent_paint_independently() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("paints.svg");
    fs::write(
        &file,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24">
        <g fill="none" stroke="#135790" stroke-opacity="0" stroke-width="4">
          <rect width="6" height="6"/>
          <rect x="8" width="6" height="6" fill="#12345600" stroke="none"/>
          <rect x="16" width="6" height="6" style="fill:transparent; stroke:initial"/>
        </g></svg>"##,
    )
    .unwrap();
    let mut doc = import_svg(&file).unwrap();
    let paints = |doc: &document_core::Document| {
        assert!(doc.objects[0].no_fill && !doc.objects[0].no_stroke);
        assert_eq!(doc.objects[0].stroke, [0x13, 0x57, 0x90, 0]);
        assert!(!doc.objects[1].no_fill && doc.objects[1].no_stroke);
        assert_eq!(doc.objects[1].fill, [0x12, 0x34, 0x56, 0]);
        assert_eq!(doc.objects[1].stroke_width, 4.);
        assert!(!doc.objects[2].no_fill && doc.objects[2].no_stroke);
        assert_eq!(doc.objects[2].fill, [0; 4]);
    };
    paints(&doc);
    let gradient = document_core::Gradient::from_color(
        document_core::GradientKind::Radial {
            center: [0.5; 2],
            radius: 0.5,
        },
        [255; 4],
    );
    doc.objects[0].gradient = Some(gradient.clone());
    doc.objects[1].stroke_gradient = Some(gradient);
    export_svg(&file, &doc, 0, true).unwrap();
    let svg = fs::read_to_string(&file).unwrap();
    assert!(!svg.contains("<radialGradient"));
    assert!(svg.contains(r##"fill="none" stroke="#135790" stroke-opacity="0""##));
    assert!(svg.contains(r##"fill="#123456" fill-opacity="0" stroke="none""##));
    paints(&import_svg(&file).unwrap());
}

#[test]
fn gradient_strokes_roundtrip_on_filled_shapes_and_on_zero_height_paths() {
    use document_core::{Document, Gradient, GradientKind, PathNode};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("stroke.svg");
    let mut d = Document::default();
    d.add_rectangle([17., 20.], [80., 32.], [0; 4]);
    d.add_shape(
        [13., 82.],
        [92., 6.],
        [0; 4],
        Shape::Path {
            closed: false,
            nodes: [[0., 0.5], [1., 0.5]].map(PathNode::corner).into(),
        },
        1,
    );
    let fill = Gradient::from_color(
        GradientKind::Radial {
            center: [0.42, 0.61],
            radius: 0.52,
        },
        [29, 191, 131, 218],
    );
    let box_fill = Gradient {
        kind: GradientKind::Rectangular {
            center: [0.42, 0.61],
            radius: 0.52,
        },
        ..fill.clone()
    };
    for o in &mut d.objects {
        o.stroke_width = 12.;
        o.stroke = [0; 4];
        o.group = Some(12);
    }
    for kind in [
        GradientKind::Linear {
            start: [0., 0.2],
            end: [1., 0.7],
        },
        GradientKind::Radial {
            center: [0.3, 0.5],
            radius: 0.8,
        },
        GradientKind::Rectangular {
            center: [0.3, 0.5],
            radius: 0.8,
        },
    ] {
        let mut paint = Gradient::from_color(kind, [214, 143, 22, 224]);
        paint.stops[1].color = [124, 36, 201, 63];
        for o in &mut d.objects {
            o.stroke_gradient = Some(paint.clone())
        }
        for fill_paint in [&fill, &box_fill] {
            d.objects[0].gradient = Some(fill_paint.clone());
            export_svg(&file, &d, 0, true).unwrap();
            let restored = import_svg(&file).unwrap();
            assert_eq!(restored.objects[0].gradient.as_ref(), Some(fill_paint));
            for o in &restored.objects {
                assert_eq!(o.stroke_gradient.as_ref(), Some(&paint));
                assert!(o.group.is_some());
            }
            assert_eq!(
                renderer::render(&d, 0).unwrap().data(),
                renderer::render(&restored, 0).unwrap().data()
            );
        }
    }
}

#[test]
fn pressure_gradient_preserves_the_original_centerline_paint_box_after_svg_export() {
    use document_core::{Document, Gradient, GradientKind, PathNode};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pressure.svg");
    let mut d = Document::default();
    d.add_shape(
        [15., 42.],
        [91., 42.],
        [0; 4],
        Shape::Path {
            closed: false,
            nodes: [[0., 0.1], [0.38, 0.15], [0.76, 0.7], [1., 1.]]
                .map(PathNode::corner)
                .into(),
        },
        1,
    );
    let o = &mut d.objects[0];
    o.stroke_width = 18.;
    o.cap = LineCap::Round;
    o.rotation = 21.;
    o.stroke_profile = vec![[0., 0.8], [0.5, 1.], [1., 0.2]];
    for kind in [
        GradientKind::Linear {
            start: [0., 0.],
            end: [1., 0.3],
        },
        GradientKind::Radial {
            center: [0.35, 0.4],
            radius: 0.8,
        },
        GradientKind::Rectangular {
            center: [0.35, 0.4],
            radius: 0.8,
        },
    ] {
        let mut paint = Gradient::from_color(kind, [215, 133, 34, 234]);
        paint.stops[1].color = [88, 36, 178, 80];
        d.objects[0].stroke_gradient = Some(paint);
        export_svg(&file, &d, 0, true).unwrap();
        let restored = import_svg(&file).unwrap();
        let original = renderer::render(&d, 0).unwrap();
        let result = renderer::render(&restored, 0).unwrap();
        let error = pixel_difference(&original, &result);
        assert!(error < 2500, "summed channel difference {error}");
    }
}

fn pixel_difference(expected: &tiny_skia::Pixmap, actual: &tiny_skia::Pixmap) -> u32 {
    assert_eq!(
        (expected.width(), expected.height()),
        (actual.width(), actual.height())
    );
    expected
        .data()
        .iter()
        .zip(actual.data())
        .map(|(a, b)| u32::from(a.abs_diff(*b)))
        .sum()
}

#[test]
fn pressure_gradient_roundtrips_through_an_outer_clip_or_alpha_mask() {
    assert_clipped_pressure_gradient_roundtrip(pressure_path([0; 4], 0.76));
}

#[test]
fn pressure_gradient_with_a_fill_composites_opacity_once_under_the_outer_clip_or_mask() {
    for opacity in [0.76, 1.] {
        assert_clipped_pressure_gradient_roundtrip(pressure_path([239, 164, 48, 182], opacity));
    }
}

#[test]
fn pressure_gradient_and_its_group_sibling_share_a_recoverable_outer_stencil() {
    for fill in [[0; 4], [239, 164, 48, 182]] {
        let mut doc = pressure_path(fill, 0.76);
        doc.add_rectangle([37., 46.], [43., 26.], [208, 53, 150, 144]);
        for object in &mut doc.objects {
            object.group = Some(7);
        }
        doc.rename_group(1, 7, "Brush and wash").unwrap();
        assert_clipped_pressure_gradient_roundtrip(doc);
    }
}

fn pressure_path(fill: [u8; 4], opacity: f32) -> document_core::Document {
    use document_core::{Document, PathNode};
    let mut source = Document::default();
    source.add_shape(
        [16., 32.],
        [90., 55.],
        fill,
        Shape::Path {
            closed: false,
            nodes: [[0., 0.1], [0.3, 0.2], [0.64, 0.74], [1., 1.]]
                .map(PathNode::corner)
                .into(),
        },
        1,
    );
    let o = &mut source.objects[0];
    o.stroke_width = 17.;
    o.stroke_profile = vec![[0., 0.3], [0.52, 1.], [1., 0.48]];
    o.cap = LineCap::Round;
    o.join = LineJoin::Round;
    o.opacity = opacity;
    o.scale = [-0.9, 1.04];
    o.rotation = 19.;
    source
}

fn assert_clipped_pressure_gradient_roundtrip(mut source: document_core::Document) {
    use document_core::{AnimationProject, Document, Gradient, GradientKind};

    fn artwork_depth(doc: &Document) -> usize {
        doc.objects
            .iter()
            .filter_map(|o| o.embedded_artwork.as_ref())
            .map(|art| 1 + artwork_depth(art))
            .max()
            .unwrap_or(0)
    }

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("clipped-pressure.svg");
    let native = dir.path().join("imported.vanim");
    let reexport = dir.path().join("reexport.svg");
    for kind in [
        GradientKind::Radial {
            center: [0.32, 0.55],
            radius: 0.76,
        },
        GradientKind::Rectangular {
            center: [0.32, 0.55],
            radius: 0.76,
        },
    ] {
        let mut gradient = Gradient::from_color(kind.clone(), [35, 184, 216, 242]);
        gradient.stops[1].color = [229, 71, 39, 103];
        source.objects[0].stroke_gradient = Some(gradient);
        let unclipped = renderer::render(&source, 0).unwrap();
        for alpha in [None, Some([41, 208])] {
            let mut doc = source.clone();
            let clip = doc.add_shape([29., 33.], [63., 44.], [255; 4], Shape::Ellipse, 1);
            doc.objects.last_mut().unwrap().rotation = -16.;
            doc.set_clip(&[source.objects[0].id, clip], 0, alpha)
                .unwrap();
            let original = renderer::render(&doc, 0).unwrap();
            assert!(original.pixels().iter().filter(|p| p.alpha() > 25).count() > 100);
            export_svg(&file, &doc, 0, true).unwrap();
            let imported =
                import_svg(&file).unwrap_or_else(|e| panic!("{kind:?}, mask {alpha:?}: {e}"));
            let rendered = renderer::render(&imported, 0).unwrap();
            // Edges may differ slightly after the SVG outline has been serialized and imported.
            let compare = |expected: &tiny_skia::Pixmap, actual: &tiny_skia::Pixmap| {
                let error = pixel_difference(expected, actual);
                assert!(
                    error < 2500,
                    "{kind:?}, mask {alpha:?}: summed channel difference {error}"
                );
            };
            compare(&original, &rendered);
            let depth = artwork_depth(&imported);
            crate::save_animation(&native, &AnimationProject::from_document(imported), true)
                .unwrap();
            let project = crate::load_animation(&native).unwrap();
            let reloaded = project.clips[0].drawing_at(0);
            assert_eq!(
                rendered.data(),
                renderer::render(reloaded, 0).unwrap().data()
            );
            export_svg(&reexport, reloaded, 0, true).unwrap();
            let mut imported = import_svg(&reexport).unwrap();
            assert!(
                artwork_depth(&imported) <= depth,
                "Re-export added another layer of embedded artwork"
            );
            compare(&original, &renderer::render(&imported, 0).unwrap());
            let clipped = imported
                .objects
                .iter()
                .find(|o| o.clip_source.is_some())
                .unwrap();
            assert_eq!(clipped.stencil.as_ref().unwrap().alpha, alpha);
            let selected = clipped.id;
            let contents: Vec<_> = imported.objects.iter().map(|o| o.id).collect();
            imported.release_clip(&[selected], 0).unwrap();
            assert_eq!(imported.objects.len(), contents.len() + 1);
            imported.objects.retain(|o| contents.contains(&o.id));
            if let Some(name) = source.objects[0].group_name.as_ref() {
                assert_eq!(imported.objects.len(), source.objects.len());
                let group = imported.objects[0].group.unwrap();
                assert!(imported.objects.iter().all(|o| o.group == Some(group)
                    && o.group_name.as_ref() == Some(name)
                    && o.stencil.is_none()
                    && o.clip_source.is_none()));
            }
            compare(&unclipped, &renderer::render(&imported, 0).unwrap());
        }
    }
}

#[test]
fn rectangular_fill_reimports_as_one_editable_gradient_with_translucent_stops() {
    use document_core::{Document, Gradient, GradientKind, GradientStop};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("box.svg");
    let mut doc = Document::default();
    doc.add_rectangle([9., 18.], [85., 36.], [0; 4]);
    let object = &mut doc.objects[0];
    object.rotation = 27.;
    object.scale = [-1., 1.];
    object.gradient = Some(Gradient {
        kind: GradientKind::Rectangular {
            center: [-0.125, 0.31],
            radius: 0.73,
        },
        stops: [
            (0., [255, 20, 5, 30]),
            (0.41, [0, 231, 7, 255]),
            (0.41, [77, 93, 0, 23]),
            (1., [5, 67, 210, 0]),
        ]
        .map(|(offset, color)| GradientStop { offset, color })
        .to_vec(),
    });
    export_svg(&path, &doc, 0, true).unwrap();
    let imported = import_svg(&path).unwrap();
    assert_eq!(imported.objects.len(), 1);
    assert_eq!(imported.objects[0].gradient, doc.objects[0].gradient);
    assert_eq!(
        renderer::render(&doc, 0).unwrap().data(),
        renderer::render(&imported, 0).unwrap().data()
    );
    let svg = fs::read_to_string(&path).unwrap();
    assert!(svg.contains("data-phyro-gradient"));
    fs::write(&path, svg.replace("data-phyro-", "data-loopline-")).unwrap();
    assert_eq!(import_svg(&path).unwrap(), imported);
    // A changed SVG pattern must not be silently imported as a different fill.
    let modified = svg.replace("matrix(0 1 -1 0 0 0)", "matrix(1 0 0 1 0 0)");
    fs::write(&path, modified).unwrap();
    assert!(import_svg(&path).is_err());
}

#[test]
fn flipped_gradient_and_rounded_rectangle_keep_their_paint_in_svg() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("source.svg");
    let result = dir.path().join("flipped.svg");
    fs::write(&original, r##"<svg width="48" height="32"><defs><linearGradient id="paint" x1="0" y1="0" x2="1" y2="0.5"><stop offset="0" stop-color="#da2244"/><stop offset="1" stop-color="#2277bb"/></linearGradient></defs><rect x="8" y="6" width="32" height="20" rx="4" fill="url(#paint)"/></svg>"##).unwrap();
    let mut doc = import_svg(&original).unwrap();
    let source = renderer::render(&doc, 0).unwrap();
    for scale in [[-1., 1.], [1., -1.]] {
        doc.objects[0].scale = scale;
        export_svg(&result, &doc, 0, true).unwrap();
        let roundtrip = import_svg(&result).unwrap();
        let exported = renderer::render(&roundtrip, 0).unwrap();
        let native = renderer::render(&doc, 0).unwrap();
        for y in 0..32 {
            for x in 0..48 {
                let sx = if scale[0] < 0. { 47 - x } else { x };
                let sy = if scale[1] < 0. { 31 - y } else { y };
                let expected = source.pixel(sx, sy).unwrap();
                // The rasterizer can round coverage differently around a reflected curve.
                if (1..255).contains(&expected.alpha()) {
                    continue;
                }
                for actual in [native.pixel(x, y).unwrap(), exported.pixel(x, y).unwrap()] {
                    assert!(
                        actual.red().abs_diff(expected.red()) <= 2
                            && actual.green().abs_diff(expected.green()) <= 2
                            && actual.blue().abs_diff(expected.blue()) <= 2
                            && actual.alpha().abs_diff(expected.alpha()) <= 2,
                        "{scale:?} at ({x}, {y}): {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn rotated_recoverable_masks_export_once_and_reimport_with_the_same_ramp() {
    use document_core::Document;
    let dir = tempfile::tempdir().unwrap();
    for grouped in [false, true] {
        let mut doc = Document::default();
        let a = doc.add_rectangle([6., 28.], [74., 53.], [34, 80, 143, 255]);
        let b = doc.add_rectangle([36., 27.], [62., 55.], [185, 84, 72, 255]);
        doc.objects[0].rotation = -8.;
        let cutter = doc.add_shape([22., 32.], [68., 36.], [255; 4], Shape::Ellipse, 1);
        doc.objects[2].rotation = 28.;
        let ids = if grouped {
            doc.objects[0].group = Some(12);
            doc.objects[1].group = Some(12);
            vec![a, b, cutter]
        } else {
            vec![b, cutter]
        };
        doc.set_clip(&ids, 0, Some([32, 190])).unwrap();
        let path = dir.path().join(format!("mask-{grouped}.svg"));
        export_svg(&path, &doc, 0, false).unwrap();
        let svg = fs::read_to_string(&path).unwrap();
        assert_eq!(svg.matches(" mask=\"url(").count(), 1);
        let mut restored = import_svg(&path).unwrap();
        assert_eq!(restored.objects.len(), 2);
        assert_eq!(restored.objects[1].group.is_some(), grouped);
        let original = renderer::render(&doc, 0).unwrap();
        let imported = renderer::render(&restored, 0).unwrap();
        let worst = original
            .data()
            .iter()
            .zip(imported.data())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            worst <= 2,
            "rotated mask, grouped={grouped}: maximum channel difference {worst}"
        );
        let selected = restored.objects[1].id;
        restored.release_clip(&[selected], 0).unwrap();
        assert_eq!(restored.objects.len(), 3);
        assert!(restored.objects.iter().all(|o| o.stencil.is_none()));
    }
}

#[test]
fn svg_export_preserves_independent_artwork_outside_its_source_page() {
    use document_core::{Document, SymbolInstance};
    use std::sync::Arc;
    let source = Arc::new({
        let mut art = Document {
            width: 32,
            height: 32,
            ..Default::default()
        };
        art.add_rectangle([-20., 8.], [28., 12.], [80, 193, 145, 255]);
        art
    });
    let mut doc = Document::default();
    doc.add_rectangle([136., 22.], [32., 32.], [0; 4]);
    let dir = tempfile::tempdir().unwrap();
    let linked = dir.path().join("linked.svg");
    let independent = dir.path().join("independent.svg");
    doc.objects[0].symbol = Some(SymbolInstance::new(7, source.clone()));
    export_svg(&linked, &doc, 0, false).unwrap();
    doc.objects[0].symbol = None;
    doc.objects[0].embedded_artwork = Some(source.into());
    export_svg(&independent, &doc, 0, false).unwrap();
    assert_eq!(fs::read(linked).unwrap(), fs::read(&independent).unwrap());
    let exported = import_svg(&independent).unwrap();
    let image = renderer::render(&exported, 0).unwrap();
    assert_eq!(image.pixel(118, 32).unwrap().alpha(), 255);
    assert!(image.data() == renderer::render(&doc, 0).unwrap().data());
}

#[test]
fn evenodd_silhouettes_keep_holes_through_picking_raster_and_svg() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("evenodd.svg");
    let exported = dir.path().join("roundtrip.svg");
    fs::write(
        &input,
        r#"<svg width="48" height="48"><defs><clipPath id="c" clipPathUnits="objectBoundingBox">
        <path d="M0 0H1V1H0Z M.25 .25H.75V.75H.25Z" clip-rule="evenodd"/></clipPath></defs>
        <rect x="4" y="4" width="40" height="40" clip-path="url(#c)"/></svg>"#,
    )
    .unwrap();
    let doc = import_svg(&input).unwrap();
    assert_eq!(doc.hit_test([24., 24.], 0), None);
    assert_eq!(doc.hit_test([8., 24.], 0), Some(doc.objects[0].id));
    export_svg(&exported, &doc, 0, false).unwrap();
    let replay = import_svg(&exported).unwrap();
    let image = renderer::render(&replay, 0).unwrap();
    assert_eq!(image.pixel(24, 24).unwrap().alpha(), 0);
    assert_eq!(image.pixel(8, 24).unwrap().alpha(), 255);
}

#[test]
fn editable_text_uses_the_bundled_face_and_alphabetic_baseline() {
    use document_core::typography::{TextAlign, Typeface};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("type.svg");
    let output = dir.path().join("out.svg");
    fs::write(&input,r##"<svg width="160" height="120" font-family="Hack" font-size="12px">
      <g style="font-size:200%;text-anchor:middle;fill:#306b70">
        <text x="48" y="40">H</text>
        <text x="48" y="40" transform="translate(100 0) rotate(90)">H</text>
        <text x="80" y="85" font-size="12" font-family="Ubuntu" font-weight="300"> a  &amp;  b </text>
        <text x="48" y="40" transform="translate(160 0) scale(-1 1)">H</text>
      </g></svg>"##).unwrap();
    let doc = import_svg(&input).unwrap();
    let (h, rotated, caption) = (&doc.objects[0], &doc.objects[1], &doc.objects[2]);
    let style = h.text.as_ref().unwrap();
    assert_eq!(style.face, Typeface::HackRegular);
    assert_eq!((style.size, style.align), (24., TextAlign::Center));
    assert!((h.position[0] + h.size[0] / 2. - 48.).abs() < 0.001);
    assert!(
        (h.bounds(0)[3] - 40.).abs() < 0.01,
        "capital H sits on its alphabetic baseline"
    );
    assert_eq!(h.fill, [48, 107, 112, 255]);
    for n in h.shape.contours().iter().flatten() {
        let [x, y] = h.local_to_world(n.point, 0);
        let r = rotated.local_to_world(n.point, 0);
        assert!((r[0] - (100. - y)).abs() < 0.001 && (r[1] - x).abs() < 0.001);
        let mirrored = doc.objects[3].local_to_world(n.point, 0);
        assert!((mirrored[0] - (160. - x)).abs() < 0.001 && (mirrored[1] - y).abs() < 0.001);
    }
    assert_eq!(caption.text.as_ref().unwrap().face, Typeface::UbuntuLight);
    assert_eq!(caption.text.as_ref().unwrap().content, "a & b");
    export_svg(&output, &doc, 0, false).unwrap();
    let image = renderer::render(&doc, 0).unwrap();
    let recovered = renderer::render(&import_svg(&output).unwrap(), 0).unwrap();
    assert!(image
        .data()
        .iter()
        .zip(recovered.data())
        .all(|(&a, &b)| a.abs_diff(b) <= 2));
}

#[test]
fn editable_text_rejects_missing_faces_variants_and_unrepresentable_transforms() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unsupported-type.svg");
    for attrs in [
        "font-family='Missing, Hack'",
        "font-family='Ubuntu' font-weight='normal'",
        "font-family='Hack' font-style='italic'",
        "font-family='Hack' font-weight='700'",
        "font-family='Hack' transform='matrix(1 0 .2 1 0 0)'",
        "font-family='Hack' transform='scale(0 1)'",
        "font-family='Hack' x='12 24'",
    ] {
        fs::write(&path, format!("<svg><text {attrs}>Hello</text></svg>")).unwrap();
        assert!(import_svg(&path).is_err(), "{attrs}");
    }
}

#[test]
fn evenodd_fill_preserves_same_winding_holes_with_editable_cubics() {
    use document_core::FillRule;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("evenodd.svg");
    let exported = dir.path().join("roundtrip.svg");
    fs::write(&path, r#"<svg width="32" height="32" fill-rule="evenodd"><path d="M1 1H31V31H1z M8 8C11 5 20 5 24 8V24H8z"/></svg>"#).unwrap();
    let mut doc = import_svg(&path).unwrap();
    assert_eq!(doc.objects[0].fill_rule, FillRule::EvenOdd);
    assert_eq!(doc.hit_test([16., 16.], 0), None);
    assert_eq!(
        renderer::render(&doc, 0)
            .unwrap()
            .pixel(16, 16)
            .unwrap()
            .alpha(),
        0
    );
    export_svg(&exported, &doc, 0, false).unwrap();
    let restored = import_svg(&exported).unwrap();
    assert_eq!(restored.objects[0].shape, doc.objects[0].shape);
    assert_eq!(restored.objects[0].fill_rule, FillRule::EvenOdd);
    doc.objects[0].fill_rule = FillRule::NonZero;
    assert_eq!(doc.hit_test([16., 16.], 0), Some(doc.objects[0].id));
    assert_eq!(
        renderer::render(&doc, 0)
            .unwrap()
            .pixel(16, 16)
            .unwrap()
            .alpha(),
        255
    );
}

#[test]
fn forward_gradient_clip_and_mask_definitions_remain_editable_after_export() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("defs.svg");
    let output = dir.path().join("export.svg");
    fs::write(&input,r##"<svg width="80" height="50">
      <rect x="4" y="4" width="30" height="28" fill="url(#ink)" clip-path="url(#window)"/>
      <path d="M45 5 C70 -3 75 15 70 35 L45 28 Z" transform="rotate(11 60 20)" fill="url(#light)" mask="url(#fade)"/>
      <defs>
        <mask id="fade" maskContentUnits="objectBoundingBox"><rect width="1" height="1" fill="url(#alpha)"/></mask>
        <linearGradient id="ink"><stop offset="0" style="stop-color:red"/><stop offset="100%" stop-color="blue"/></linearGradient>
        <clipPath id="window" clipPathUnits="objectBoundingBox"><ellipse cx=".5" cy=".5" rx=".45" ry=".45"/></clipPath>
        <radialGradient id="light" xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="#colors" cx="25%"/>
        <linearGradient id="colors"><stop offset="0" stop-color="red"/><stop offset=".7" stop-color="yellow"/><stop offset="1" stop-color="blue"/></linearGradient>
        <linearGradient id="alpha"><stop offset="0" stop-color="white" stop-opacity="0"/><stop offset="1" stop-color="white"/></linearGradient>
      </defs></svg>"##).unwrap();
    let doc = import_svg(&input).unwrap();
    assert!(doc
        .objects
        .iter()
        .all(|o| o.gradient.is_some() && o.stencil.is_some()));
    assert_eq!(
        doc.objects[1].stencil.as_ref().unwrap().alpha,
        Some([0, 255])
    );
    let reference = renderer::render(&doc, 0).unwrap();
    assert_eq!(reference.pixel(4, 4).unwrap().alpha(), 0);
    assert!(reference.pixel(15, 18).unwrap().red() > 100);
    export_svg(&output, &doc, 0, false).unwrap();
    let reopened = import_svg(&output).unwrap();
    let raster = renderer::render(&reopened, 0).unwrap();
    assert!(reference
        .data()
        .iter()
        .zip(raster.data())
        .all(|(a, b)| a.abs_diff(*b) <= 2));
}

#[test]
fn compound_svg_preserves_hole_winding_and_editable_contours() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("ring.svg");
    let output = dir.path().join("export.svg");
    // The relative second moveto is measured from the first contour's start.
    fs::write(
        &source,
        r##"<svg width="32" height="32">
        <path fill="#6283ef" d="M2 2 H30 V30 H2 z m8 8 v12 h12 v-12 z"/>
        </svg>"##,
    )
    .unwrap();
    let doc = import_svg(&source).unwrap();
    assert_eq!(doc.objects.len(), 1);
    assert!(
        matches!(&doc.objects[0].shape, Shape::Compound { contours } if contours.iter().map(Vec::len).eq([4,4]))
    );
    assert_eq!(doc.hit_test([16., 16.], 0), None);
    assert_eq!(doc.hit_test([5., 16.], 0), Some(doc.objects[0].id));
    let image = renderer::render(&doc, 0).unwrap();
    assert_eq!(image.pixel(16, 16).unwrap().alpha(), 0);
    assert_eq!(image.pixel(5, 16).unwrap().alpha(), 255);
    export_svg(&output, &doc, 0, false).unwrap();
    let restored = import_svg(&output).unwrap();
    assert_eq!(restored.objects[0].shape, doc.objects[0].shape);
    assert_eq!(image.data(), renderer::render(&restored, 0).unwrap().data());
}

#[test]
fn svg_round_trip_keeps_geometry_paint_order_and_logical_groups() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.svg");
    let output = dir.path().join("output.svg");
    fs::write(&source,r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" viewBox="0 0 64 64">
      <g inkscape:groupmode="layer" inkscape:label="Ink &amp; light">
        <g data-loopline-group="7"><rect x="8" y="10" width="24" height="20" fill="#264650" transform="rotate(15 20 20)"/></g>
        <circle cx="20" cy="18" r="10" fill="#fff" stroke="#000" stroke-width="2"/>
        <g data-loopline-group="7" inkscape:label="Lumi &amp; yö" transform="translate(4,3)"><path d="M2 40 C12 11 30 20 44 40 z" style="fill:#fe7;stroke:#345;stroke-width:2;stroke-linejoin:bevel"/></g>
      </g><g inkscape:groupmode="layer" inkscape:label="Hidden" display="none" data-loopline-locked="true">
        <ellipse cx="5" cy="5" rx="2" ry="1"/>
      </g></svg>"##).unwrap();
    let mut doc = import_svg(&source).unwrap();
    assert_eq!(doc.objects.len(), 4);
    assert_eq!(doc.layers[0].name, "Ink & light");
    assert!(doc.layers[1].locked && !doc.layers[1].visible);
    assert!(doc.objects[0].group.is_some());
    assert_eq!(doc.objects[0].group, doc.objects[2].group);
    assert_eq!(doc.objects[0].group_name.as_deref(), Some("Lumi & yö"));
    assert_eq!(doc.objects[2].group_name, doc.objects[0].group_name);
    assert_eq!(doc.objects[1].group, None);
    assert!(matches!(doc.objects[1].shape, Shape::Ellipse));
    assert_eq!(doc.objects[1].cap, LineCap::Butt);
    assert_eq!(doc.objects[1].join, LineJoin::Miter);
    assert_eq!(doc.objects[2].join, LineJoin::Bevel);
    // Export an evaluated pose. Native animation is retained by the original document.
    doc.objects[0]
        .keys
        .extend([(0, [8., 10.]), (4, [16., 18.])]);
    let ellipse = &mut doc.objects[1];
    ellipse.rotation_keys.extend([(0, 0.), (4, 180.)]);
    ellipse.scale_keys.extend([(0, [1., 1.]), (4, [2., 0.5])]);
    ellipse.opacity_keys.extend([(0, 1.), (4, 0.)]);
    export_svg(&output, &doc, 2, false).unwrap();
    assert!(export_svg(&output, &doc, 2, false).is_err());
    let again = import_svg(&output).unwrap();
    assert_eq!(again.objects.len(), doc.objects.len());
    assert_eq!(again.layers, doc.layers);
    assert_eq!(again.objects[0].group, again.objects[2].group);
    assert_eq!(again.objects[0].group_name, doc.objects[0].group_name);
    assert_eq!(again.objects[2].group_name, doc.objects[2].group_name);
    assert_eq!(again.objects[1].group_name, None);
    assert_eq!(again.objects[1].opacity, 0.5);
    assert!(matches!(&again.objects[2].shape,Shape::Path{nodes,closed:true} if nodes.len()==2));
    let original = renderer::render(&doc, 2).unwrap();
    let restored = renderer::render(&again, 0).unwrap();
    assert!(original
        .data()
        .iter()
        .zip(restored.data())
        .all(|(a, b)| a.abs_diff(*b) <= 1));
}

#[test]
fn svg_viewbox_and_group_transforms_apply_in_paint_order() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.svg");
    fs::write(
        &source,
        r#"<svg width="100" height="80" viewBox="5 10 20 20">
        <g transform="translate(2 1) scale(2)">
            <rect x="2" y="5" width="3" height="4"/>
        </g></svg>"#,
    )
    .unwrap();
    let doc = import_svg(&source).unwrap();
    assert_eq!((doc.width, doc.height), (100, 80));
    assert_eq!(doc.objects[0].position, [14., 4.]);
    assert_eq!(doc.objects[0].size, [24., 32.]);
}

#[test]
fn unsupported_svg_is_rejected_without_flattening_or_partial_import() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.svg");
    for body in [
        r#"<image href="https://example.invalid/test.png"/>"#,
        r#"<rect width="4" height="4" style="fill:url(#gradient)"/>"#,
        r#"<g opacity="0.5"><circle r="5"/></g>"#,
        r#"<path d="M0 0L5 5M8 8L10 10"/>"#,
        r#"<rect width="2" height="5" transform="scale(2 1)" stroke="black"/>"#,
        r#"<text>Caption</text>"#,
        r#"<defs><style>rect {fill:red}</style></defs>"#,
        r#"<defs><linearGradient id="ink"/></defs><rect width="5" height="5" fill="url(#ink)"/>"#,
        r#"<metadata><script/></metadata>"#,
        r#"<defs onload="example()"/>"#,
    ] {
        fs::write(
            &path,
            format!(r#"<svg width="16" height="16">{body}</svg>"#),
        )
        .unwrap();
        assert!(import_svg(&path).is_err(), "{body}");
    }
    fs::write(&path, r#"<!DOCTYPE svg><svg/>"#).unwrap();
    assert!(import_svg(&path).is_err());
}

#[test]
fn inkscape_mm_page_opens_as_editable_layers_shapes_and_curves() {
    let source = r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
    <!-- Created with Inkscape (http://www.inkscape.org/) -->
    <svg xmlns="http://www.w3.org/2000/svg"
         xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
         xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
         xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:cc="http://creativecommons.org/ns#" xmlns:dc="http://purl.org/dc/elements/1.1/"
         width="210mm" height="297mm" viewBox="0 0 210 297" version="1.1" id="svg1"
         inkscape:version="1.4" sodipodi:docname="mascot.svg">
      <sodipodi:namedview id="namedview1" pagecolor="#ffffff" inkscape:zoom="0.5" inkscape:current-layer="layer1">
        <inkscape:page x="0" y="0" width="210" height="297" id="page1"/>
      </sodipodi:namedview>
      <defs id="defs1"/>
      <metadata id="metadata1"><rdf:RDF><cc:Work rdf:about=""><dc:format>image/svg+xml</dc:format><dc:type rdf:resource="http://purl.org/dc/dcmitype/StillImage"/></cc:Work></rdf:RDF></metadata>
      <g inkscape:groupmode="layer" inkscape:label="Mascot" id="layer1">
        <rect id="rect1" x="12.7" y="25.4" width="25.4" height="12.7"
          style="fill:#6ccdab;fill-opacity:1;stroke:#222222;stroke-width:0.53;stroke-dasharray:none;stroke-opacity:1"/>
        <path id="path1" sodipodi:nodetypes="ccc" inkscape:label="Smile"
          d="M 8,12 C 10,15 12,15 14,12 L 15,10" style="fill:none;stroke:#222222;stroke-width:0.26;stroke-linecap:round;stroke-linejoin:round"/>
      </g>
    </svg>"##;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inkscape.svg");
    fs::write(&path, source).unwrap();
    let doc = import_svg(&path).unwrap();
    assert_eq!((doc.width, doc.height), (794, 1123));
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name, "Mascot");
    assert!(matches!(doc.objects[0].shape, Shape::Rectangle));
    assert!((doc.objects[0].position[0] - 48.).abs() < 0.0001);
    assert!((doc.objects[0].size[0] - 96.).abs() < 0.0001);
    assert!(
        matches!(&doc.objects[1].shape,Shape::Path{closed:false,nodes} if nodes.len()==3 && nodes[0].outgoing!=nodes[0].point)
    );
}

#[test]
fn pressure_svg_stores_evaluated_stroke_as_a_filled_vector_path() {
    use document_core::{Document, Gradient, GradientKind, PathNode, Stencil};
    let mut doc = Document::default();
    doc.add_shape(
        [15., 18.],
        [84., 55.],
        [0; 4],
        Shape::Path {
            nodes: [[0., 0.], [0.6, 0.22], [0.78, 0.55], [1., 1.]]
                .map(PathNode::corner)
                .into(),
            closed: false,
        },
        1,
    );
    let o = &mut doc.objects[0];
    o.stroke_profile = vec![[0., 0.2], [0.5, 1.], [1., 0.4]];
    o.stroke = [220, 30, 65, 210];
    o.stroke_width = 14.;
    o.cap = LineCap::Round;
    o.join = LineJoin::Round;
    o.rotation = 18.;
    o.scale = [-0.8, 1.1];
    o.opacity = 0.72;
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("pressure.svg");
    for clipped in [false, true] {
        if clipped {
            doc.objects[0].stencil = Some(Stencil {
                shape: Shape::Ellipse,
                fill_rule: Default::default(),
                origin: [0.06, 0.14],
                size: [0.82, 0.70],
                alpha: None,
            });
        }
        export_svg(&svg, &doc, 0, true).unwrap();
        let data = fs::read_to_string(&svg).unwrap();
        assert!(data.contains(r##"fill="#dc1e41""##));
        assert!(!data.contains("stroke-width="));
        let imported = import_svg(&svg).unwrap();
        let native = renderer::render(&doc, 0).unwrap();
        let rendered = renderer::render(&imported, 0).unwrap();
        assert!(native.pixels().iter().any(|px| px.alpha() > 140));
        // SVG import converts quadratic arcs to cubic curves. The two forms
        // can quantize an antialiased edge to slightly different coverages.
        let mut alpha_error = 0u32;
        for (i, (a, b)) in native
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(rendered.data().as_chunks::<4>().0)
            .enumerate()
        {
            alpha_error += u32::from(a[3].abs_diff(b[3]));
            assert!(
                a.iter().zip(b).all(|(l, r)| l.abs_diff(*r) <= 32),
                "clipped={clipped}, pixel {i}: {a:?} vs {b:?}"
            );
        }
        assert!(alpha_error < 512, "unexpected change in stroke coverage");
    }

    doc.objects[0].fill = [17, 98, 121, 255];
    let file = dir.path().join("fill-and-pressure.svg");
    export_svg(&file, &doc, 0, false).unwrap();
    let markup = fs::read_to_string(&file).unwrap();
    // Object opacity and the stencil composite both paints once, after they overlap.
    assert!(markup.contains(r#"<g data-phyro-ungrouped="true" opacity="0.72" clip-path="url("#));
    let base = markup.find(r##"fill="#116279""##).unwrap();
    let ink = markup.find(r##"fill="#dc1e41""##).unwrap();
    assert!(base < ink);
    assert_eq!(markup.matches(r#" opacity="0.72""#).count(), 1);

    for stroke in [
        None,
        Some(Gradient::from_color(
            GradientKind::Linear {
                start: [0., 0.3],
                end: [1., 0.8],
            },
            [30, 184, 203, 192],
        )),
    ] {
        doc.objects[0].stroke_gradient = stroke;
        export_svg(&file, &doc, 0, true).unwrap();
        let reopened = import_svg(&file).unwrap();
        let error = pixel_difference(
            &renderer::render(&doc, 0).unwrap(),
            &renderer::render(&reopened, 0).unwrap(),
        );
        assert!(error < 2500, "summed channel difference {error}");
    }
}
