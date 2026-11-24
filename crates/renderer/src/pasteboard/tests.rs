use super::*;
use document_core::{
    Gradient, GradientKind, GradientStop, Layer, LineCap, PathNode, Shape, Stencil,
};
use std::collections::BTreeMap;
use std::sync::{atomic::AtomicBool, Arc};

fn sample_drawing() -> Document {
    let mut doc = Document::default();
    doc.add_rectangle([-32., -40.], [64., 16.], [255; 4]);
    let bar = &mut doc.objects[0];
    bar.scale = [1.4, 0.75];
    bar.gradient = Some(Gradient {
        kind: GradientKind::Linear {
            start: [0.25, 0.5],
            end: [0.75, 0.5],
        },
        stops: vec![
            GradientStop {
                offset: 0.,
                color: [0, 0, 255, 255],
            },
            GradientStop {
                offset: 1.,
                color: [255, 0, 0, 255],
            },
        ],
    });
    let mut nodes = [[-0.2, 1.2], [0.4, 0.1], [0.7, 0.8], [1.2, -0.1]].map(PathNode::corner);
    nodes[0].outgoing = [0.05, -0.3];
    nodes[1].incoming = [0.3, 1.5];
    doc.add_shape(
        [-21., 29.],
        [51., 28.],
        [0; 4],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        1,
    );
    let stroke = &mut doc.objects[1];
    stroke.scale = [1.36, 0.78];
    stroke.rotation = 27.;
    stroke.animated.skew = -12.;
    stroke.stroke = [221, 129, 27, 255];
    stroke.stroke_width = 5.5;
    stroke.cap = LineCap::Square;
    doc.layers.push(Layer {
        id: 2,
        locked: true,
        ..Default::default()
    });
    doc.add_shape(
        [137., 32.],
        [40., 28.],
        [65, 117, 210, 255],
        Shape::Ellipse,
        2,
    );
    let ellipse = &mut doc.objects[2];
    ellipse.rotation = 19.;
    ellipse.opacity = 0.68;
    ellipse.stroke = [230, 155, 41, 220];
    ellipse.stroke_width = 4.;
    ellipse.stencil = Some(Stencil {
        shape: Shape::Ellipse,
        origin: [-0.1, 0.18],
        size: [1.2, 0.84],
        fill_rule: Default::default(),
        alpha: Some([95, 234]),
    });
    doc
}

fn apply(image: &mut Pixmap, patch: Patch) {
    let stride = image.width() as usize * 4;
    let width = patch.size[0] * 4;
    for y in 0..patch.size[1] {
        let start = (y + patch.origin[1]) * stride + patch.origin[0] * 4;
        let row = &mut image.data_mut()[start..start + width];
        if let Some(pixels) = &patch.pixels {
            row.copy_from_slice(&pixels.data()[y * width..(y + 1) * width]);
        } else {
            row.fill(0);
        }
    }
}

fn full(scene: &Scene, doc: &Document, key: Tile) -> Pixmap {
    let mut image = Pixmap::new(TEXTURE, TEXTURE).unwrap();
    apply(
        &mut image,
        scene
            .plan(key, &Damage::all())
            .unwrap()
            .render(doc)
            .unwrap(),
    );
    image
}

#[test]
fn image_texels_follow_a_rotated_clipped_rectangle_across_the_page_edge() {
    let source = Pixmap::from_vec(
        vec![255, 0, 0, 255, 0, 0, 0, 0, 0, 128, 0, 128, 0, 0, 255, 255],
        tiny_skia::IntSize::from_wh(2, 2).unwrap(),
    )
    .unwrap();
    let mut doc = Document {
        width: 8,
        height: 8,
        ..Default::default()
    };
    let id = doc.add_rectangle([-1., 2.], [2., 2.], [0; 4]);
    doc.objects[0].image = Some(Arc::new(document_core::RasterImage::from_base64(
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAF0lEQVR4nGP4z8DwnwEM/jM0MDD8/w8AK2kFfIvAcEgAAAAASUVORK5CYII=",
    ).unwrap()));
    let (crop, origin) = crate::render_artwork(&[&doc.objects[0]], &doc.layers, 0, 1).unwrap();
    assert_eq!(origin, [-1, 2]);
    assert_eq!(crop.data(), source.data());
    assert_eq!(
        crate::render(&doc, 0).unwrap().pixel(0, 2).unwrap().alpha(),
        0
    );
    let scene = Scene::new(&doc, 0, [8, 8]).unwrap();
    let left_tile = full(&scene, &doc, [-1, 0]);
    assert_eq!(left_tile.pixel(TILE, 3), source.pixel(0, 0));
    assert_eq!(scene.sample(&doc, [-0.5, 3.5]).unwrap(), [0, 128, 0, 128]);

    doc.objects[0].rotation = 90.;
    doc.objects[0].opacity = 0.5;
    let cutter = doc.add_rectangle([0., 2.], [1., 2.], [255; 4]);
    doc.set_clip(&[id, cutter], 0, None).unwrap();
    let after = crate::render(&doc, 0).unwrap();
    let red = after.pixel(0, 2).unwrap();
    assert!(red.red() >= 127 && red.red() == red.alpha());
    assert_eq!(after.pixel(0, 3).unwrap().alpha(), 0);
    assert_eq!(
        crate::sample_artwork(&doc, 0, [-0.5, 2.5], [1.; 2]).unwrap(),
        [0; 4]
    );
    doc.add_rectangle([0., 2.], [1., 1.], [0, 0, 255, 255]);
    assert_eq!(
        crate::sample_artwork(&doc, 0, [0.5, 2.5], [1.; 2]).unwrap(),
        [0, 0, 255, 255]
    );
}

#[test]
fn zoomed_artboard_tile_resolves_subpixel_art_and_patches_transparent_pixels() {
    let mut doc = Document {
        width: 1000,
        height: 1000,
        ..Default::default()
    };
    doc.add_rectangle([500., 500.], [0.25, 2.], [190, 50, 30, 128]);
    let mut scene = Scene::for_artboard(&doc, 0, [64_000; 2]).unwrap();
    let key = [125; 2]; // The tile starts at artboard (500, 500).
    let original = full(&scene, &doc, key);
    for x in 0..20 {
        let alpha = original.pixel(x, 5).unwrap().alpha();
        assert_eq!(alpha, if (1..17).contains(&x) { 128 } else { 0 });
    }
    let mut moved = doc.clone();
    moved.objects[0].position[0] += 0.5;
    let damage = scene.update(&doc, &moved, [key, key]);
    let mut patched = original;
    apply(
        &mut patched,
        scene.plan(key, &damage).unwrap().render(&moved).unwrap(),
    );
    assert_eq!(patched.data(), full(&scene, &moved, key).data());
    assert_eq!(patched.pixel(5, 5).unwrap().alpha(), 0);
    assert_eq!(patched.pixel(37, 5).unwrap().alpha(), 128);
}

#[test]
fn eyedropper_samples_the_visible_pasteboard_and_artboard_paints_in_the_same_coordinates() {
    let mut doc = sample_drawing();
    let gradient = doc.objects[0].gradient.clone();
    doc.add_rectangle([18., 21.], [35., 22.], [25, 100, 250, 172]);
    doc.add_rectangle([21., 24.], [29., 16.], [255; 4]);
    let top = doc.objects.last_mut().unwrap();
    top.gradient = gradient;
    top.opacity_keys.extend([(0, 0.5), (2, 0.8)]);
    let mut expanded = doc.clone();
    expanded.width = 384;
    expanded.height = 384;
    for object in &mut expanded.objects {
        object.position[0] += 128.;
        object.position[1] += 128.;
    }
    for frame in [0, 2] {
        let reference = crate::render_scaled(&expanded, frame, 2.).unwrap();
        let scene = Scene::new(&doc, frame, [256; 2]).unwrap();
        for [x, y] in [
            [-10., -32.],
            [-8., -32.],
            [150., 44.],
            [-60., 81.],
            [29., 29.],
            [41., 30.],
        ] {
            let point = [x, y];
            let pixel = reference
                .pixel(((x + 128.) * 2.) as u32, ((y + 128.) * 2.) as u32)
                .unwrap();
            let expected = [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()];
            assert_eq!(
                crate::sample_artwork(&doc, frame, point, [2.; 2]).unwrap(),
                expected
            );
            if !(0.0..128.).contains(&x) {
                assert_eq!(scene.sample(&doc, point).unwrap(), expected);
            }
        }
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn eyedropper_dense_pasteboard_timing() {
    let mut doc = sample_drawing();
    for i in 0..10_000 {
        doc.add_rectangle(
            [900. + (i % 100) as f32 * 8., -900. - (i / 100) as f32 * 8.],
            [3.; 2],
            [80, 90, 100, 255],
        );
    }
    let scene = Scene::new(&doc, 0, [256; 2]).unwrap();
    let mut timings = Vec::new();
    for _ in 0..100 {
        let start = std::time::Instant::now();
        let picked = scene.sample(&doc, [-10., -32.]).unwrap();
        timings.push(start.elapsed().as_secs_f64() * 1000.);
        assert_ne!(picked[3], 0);
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "10,000 off-page objects: eyedropper renderer median {:.3} ms, p95 {:.3} ms",
        timings[50], timings[95]
    );
}

#[test]
fn signed_tiles_match_an_expanded_page_with_scaled_gradient_and_crossing_stroke() {
    let doc = sample_drawing();
    let mut expanded = doc.clone();
    expanded.width = 384;
    expanded.height = 384;
    for o in &mut expanded.objects {
        o.position[0] += 128.;
        o.position[1] += 128.;
    }
    for scale in [1., 1.5, 2., 3.] {
        let px = |n: i32| (n as f32 * scale).round() as i32;
        let scene = Scene::new(&doc, 0, [px(128) as u32; 2]).unwrap();
        let reference = crate::render_scaled(&expanded, 0, scale).unwrap();
        let mut tiles = BTreeMap::new();
        let mut exterior_ink = 0;
        for y in px(-51)..px(91) {
            for x in px(-58)..px(192) {
                if (0..px(128)).contains(&x) && (0..px(128)).contains(&y) {
                    continue;
                }
                let tile = [
                    i64::from(x.div_euclid(TILE as i32)),
                    i64::from(y.div_euclid(TILE as i32)),
                ];
                let bitmap = tiles
                    .entry(tile)
                    .or_insert_with(|| full(&scene, &doc, tile));
                let p = bitmap
                    .pixel(
                        x.rem_euclid(TILE as i32) as u32 + 1,
                        y.rem_euclid(TILE as i32) as u32 + 1,
                    )
                    .unwrap();
                let r = reference
                    .pixel((x + px(128)) as u32, (y + px(128)) as u32)
                    .unwrap();
                let delta = [
                    p.red().abs_diff(r.red()),
                    p.green().abs_diff(r.green()),
                    p.blue().abs_diff(r.blue()),
                    p.alpha().abs_diff(r.alpha()),
                ];
                assert!(
                    delta.into_iter().max().unwrap() <= 1,
                    "pixel {x},{y} at {scale}x: {p:?}, {r:?}"
                );
                exterior_ink += usize::from(p.alpha() > 100);
            }
        }
        assert!(exterior_ink > 1000);
        let y = (px(-32)).rem_euclid(TILE as i32) as u32 + 1;
        let color = |x: i32| {
            tiles[&[i64::from(px(x).div_euclid(TILE as i32)), -1]]
                .pixel(px(x).rem_euclid(TILE as i32) as u32 + 1, y)
                .unwrap()
        };
        assert_eq!(color(-28).blue(), 255);
        assert_eq!(color(28).red(), 255);
        assert!((120..=136).contains(&color(0).blue()) && (120..=136).contains(&color(0).red()));
    }
}

#[test]
fn partial_repaints_follow_edits_cancellation_layer_order_and_visibility() {
    let original = sample_drawing();
    let mut doc = original.clone();
    let mut scene = Scene::new(&doc, 0, [256; 2]).unwrap();
    let keys = [[-1, -1], [-1, 0], [0, -1], [1, 0]];
    let mut images = keys.map(|key| full(&scene, &doc, key));
    for edit in 0..8 {
        let mut next = doc.clone();
        match edit {
            0 => next.objects[1].position = [139., 28.],
            1 => next.objects[0].position = [122., 44.],
            2 => next.layers.reverse(),
            3 => next.layers[0].visible = false,
            4 => {
                next.objects.remove(0);
            }
            5 => {
                next.add_rectangle([-15., -9.], [76., 17.], [143; 4]);
            }
            6 => next.objects.swap(0, 1),
            _ => next = original.clone(),
        }
        let damage = scene.update(&doc, &next, [[-1, -1], [1, 0]]);
        let fresh = Scene::new(&next, 0, [256; 2]).unwrap();
        for (&key, cached) in keys.iter().zip(images.iter_mut()) {
            if let Some(plan) = scene.plan(key, &damage) {
                apply(cached, plan.render(&next).unwrap());
            }
            let reference = full(&fresh, &next, key);
            // This set of tiles includes only one page-edge gutter on the
            // positive side. Artwork under the opaque artboard backdrop is intentionally skipped.
            let [ox, oy] = tile_origin(key);
            for y in 0..TEXTURE {
                for x in 0..TEXTURE {
                    if (1..255).contains(&(ox + i64::from(x)))
                        && (1..255).contains(&(oy + i64::from(y)))
                    {
                        continue;
                    }
                    assert_eq!(
                        cached.pixel(x, y),
                        reference.pixel(x, y),
                        "edit {edit}, tile {key:?}, {x},{y}"
                    );
                }
            }
        }
        doc = next;
    }
}

#[test]
fn dense_moves_and_membership_changes_match_a_fresh_index() {
    let mut doc = Document::default();
    doc.add_rectangle([115., 40.], [30., 18.], [220; 4]);
    for i in 0..10_000 {
        doc.add_rectangle([10_000. + i as f32, -3_000.], [12., 8.], [80; 4]);
    }
    let mut scene = Scene::new(&doc, 0, [128; 2]).unwrap();
    for phase in 0..3 {
        let mut next = doc.clone();
        for (i, o) in next.objects.iter_mut().enumerate().skip(1) {
            match phase {
                0 => o.position[0] += 75.,
                1 if i % 2 == 0 => o.position = [15., 20.],
                2 => o.position = [-5800. - i as f32, -3200.],
                _ => continue,
            }
        }
        let damage = scene.update(&doc, &next, [[-1, -1], [1, 1]]);
        assert!(!damage.affects([0, 0]));
        assert_eq!(
            scene.objects,
            Scene::new(&next, 0, [128; 2]).unwrap().objects
        );
        doc = next;
    }
}

#[test]
fn transformed_symbols_keep_overshooting_artwork_and_skip_distant_tiles() {
    use document_core::SymbolInstance;

    let mut master = Document {
        width: 32,
        height: 32,
        ..Document::default()
    };
    master.add_shape(
        [-16., -7.],
        [42., 27.],
        [62, 91, 188, 230],
        Shape::Ellipse,
        1,
    );
    let arc = &mut master.objects[0];
    arc.rotation = 23.;
    arc.animated.skew = -13.;
    arc.stroke_width = 5.;
    arc.stroke = [188, 190, 38, 255];
    arc.stencil = Some(Stencil {
        shape: Shape::Rectangle,
        origin: [-0.25, 0.],
        size: [1.5, 1.],
        fill_rule: Default::default(),
        alpha: Some([127, 230]),
    });
    let mut second = master.clone();
    second.objects[0].position = [47., 9.];
    let mut doc = Document::default();
    let a = Arc::new(master);
    let b = Arc::new(second);
    for (i, source) in [a.clone(), b, a].into_iter().enumerate() {
        doc.add_rectangle(
            [118. - i as f32 * 74., -26. + 18. * i as f32],
            [52., 39.],
            [0; 4],
        );
        let object = doc.objects.last_mut().unwrap();
        // Two independently hydrated definitions can have the same numeric ID.
        if i == 2 {
            object.embedded_artwork = Some(source.into());
        } else {
            object.symbol = Some(SymbolInstance::new(7, source));
        }
        object.rotation = 13. + 43. * i as f32;
        object.scale = [1.23, 0.75];
        object.animated.skew = -16.;
    }
    let damage = Damage::all();
    let mut scene = Scene::new(&doc, 0, [384; 2]).unwrap();
    let check = |scene: &Scene, doc: &Document| {
        let ordered = doc.paint_order();
        let mut painted = 0;
        for row in -2..=1 {
            for col in -2..=2 {
                let key = [col, row];
                let tile = scene.plan(key, &damage).unwrap();
                let actual = tile.render(doc).unwrap();
                let reference = tile
                    .paint
                    .render(
                        clip_groups::runs(ordered.iter().copied(), scene.frame()),
                        &doc.layers,
                    )
                    .unwrap();
                painted += usize::from(actual.pixels.is_some());
                assert_eq!(
                    actual.pixels.as_ref().map(Pixmap::data),
                    reference.pixels.as_ref().map(Pixmap::data),
                    "{key:?}"
                );
            }
        }
        assert!(painted > 2);
        assert!(scene.plan([16, 16], &damage).unwrap().is_empty());
    };
    check(&scene, &doc);
    let mut moved = doc.clone();
    moved.objects[2].position = [163., -43.];
    scene.update(&doc, &moved, [[-2, -2], [2, 1]]);
    check(&scene, &moved);
}

#[test]
fn detached_cold_tile_never_pins_a_document_snapshot() {
    let mut authored = Arc::new(sample_drawing());
    let scene = Scene::new(&authored, 0, [256; 2]).unwrap();
    let plan = scene.plan([1, 0], &Damage::all()).unwrap();
    let expected = plan.render(&authored).unwrap().pixels.unwrap();
    let job = plan.detach(&authored);
    Arc::get_mut(&mut authored).unwrap().objects.clear();
    let painted = job
        .render_until(&AtomicBool::new(false))
        .unwrap()
        .pixels
        .unwrap();
    assert_eq!(painted.data(), expected.data());
    assert!(job
        .render_until(&AtomicBool::new(true))
        .unwrap()
        .pixels
        .is_none());
    assert!(Scene::new(&authored, 0, [0, 3]).is_none());
    assert!(Scene::new(&authored, authored.frames, [128; 2]).is_none());
}
