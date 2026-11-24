use super::*;
use document_core::{PathNode, Shape, SymbolInstance};
use std::sync::Arc;

fn master(overhang: f32) -> Arc<Document> {
    let mut art = Document {
        width: 32,
        height: 32,
        ..Default::default()
    };
    art.add_shape(
        [overhang, 3.7],
        [23.4, 18.9],
        [23, 161, 190, 214],
        Shape::Ellipse,
        1,
    );
    let nodes = [[0., 1.], [0.45, -0.52], [1., 1.]].map(PathNode::corner);
    art.add_shape(
        [overhang, 2.7],
        [23.4, 19.5],
        [0; 4],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        1,
    );
    let outline = art.objects.last_mut().unwrap();
    outline.stroke = [219, 84, 37, 233];
    outline.stroke_width = 4.1;
    outline.cap = document_core::LineCap::Square;
    Arc::new(art)
}

fn place(doc: &mut Document, source: &Arc<Document>, position: [f32; 2]) {
    doc.add_rectangle(position, [32.; 2], [0; 4]);
    doc.objects.last_mut().unwrap().symbol = Some(SymbolInstance::new(7, Arc::clone(source)));
}

#[test]
fn independent_artwork_paints_and_picks_beyond_its_placement_box() {
    let mut source = Document {
        width: 32,
        height: 32,
        ..Default::default()
    };
    source.add_rectangle([-20., 8.], [28., 12.], [80, 193, 145, 255]);
    let source = Arc::new(source);
    let mut linked = Document::default();
    place(&mut linked, &source, [136., 22.]);
    let mut independent = linked.clone();
    let object = &mut independent.objects[0];
    let id = object.id;
    object.symbol = None;
    object.embedded_artwork = Some(source.into());

    assert_eq!(object.bounds(0), [116., 30., 144., 42.]);
    assert_eq!(independent.hit_test([118., 32.], 0), Some(id));
    assert_eq!(independent.hit_test([142., 32.], 0), Some(id));
    assert_eq!(independent.hit_test([150., 32.], 0), None);
    let image = render(&independent, 0).unwrap();
    assert_eq!(image.pixel(118, 32).unwrap().alpha(), 255);
    assert!(image.data() == render(&linked, 0).unwrap().data());
    let (crop, origin) =
        render_artwork(&independent.paint_order(), &independent.layers, 0, 1).unwrap();
    assert_eq!((origin, crop.width(), crop.height()), ([116, 30], 28, 12));
    let (linked_crop, _) = render_artwork(&linked.paint_order(), &linked.layers, 0, 1).unwrap();
    assert!(crop.data() == linked_crop.data());
}

// The uncached painter descends into every static symbol, even when its placement is off-page.
fn reference(doc: &Document, frame: u32, scale: f32) -> Pixmap {
    let page = artboard(doc, frame, scale).unwrap();
    let mut pixels = page.allocate().unwrap();
    let mut layer = None;
    for o in doc.paint_order() {
        draw_with_appearance(
            &mut pixels,
            o,
            frame,
            page.transform,
            [0.; 2],
            &mut layer,
            None,
        )
        .unwrap();
    }
    pixels
}

#[test]
fn off_page_symbols_keep_stroke_overhangs_and_follow_the_current_placement_frame() {
    let mut doc = Document::default();
    let left = master(-45.);
    let right = master(43.);
    let unavailable = master(0.);
    place(&mut doc, &left, [612., 510.]);
    place(&mut doc, &right, [-53., 35.]);
    place(&mut doc, &left, [146., 63.]);
    let moving = doc.objects.last_mut().unwrap();
    moving.animated.skew = -12.;
    moving.rotation = 9.;
    moving.keys.extend([(0, [146., 63.]), (4, [76., 81.])]);
    place(&mut doc, &unavailable, [340., 289.]);
    doc.objects.last_mut().unwrap().animated.visible = false;
    place(&mut doc, &unavailable, [231., 308.]);
    doc.objects.last_mut().unwrap().opacity = 0.;

    for (frame, scale) in [(0, 1.), (2, 1.5), (4, 2.31)] {
        let exact = reference(&doc, frame, scale);
        let actual = render_scaled(&doc, frame, scale).unwrap();
        assert!(exact.pixels().iter().any(|p| p.alpha() > 0));
        assert!(actual.data() == exact.data());
    }

    let mut cache = bounds::SymbolBounds::new();
    let page = artboard(&doc, 0, 1.).unwrap();
    let mut pixels = page.allocate().unwrap();
    for o in &doc.objects {
        draw_with_appearance(
            &mut pixels,
            o,
            0,
            page.transform,
            [0.; 2],
            &mut None,
            Some(&mut cache),
        )
        .unwrap();
    }
    // Definition IDs can be recycled across hydrated snapshots. Only the two
    // distinct, visible sources should have needed a bounds calculation.
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key(&std::ptr::from_ref(unavailable.as_ref())));
}

#[test]
#[ignore = "release timing of repeated off-page symbol placements"]
fn repeated_symbol_page_render_time() {
    use std::{hint::black_box, time::Instant};
    let source = master(-11.);
    for count in [0, 32, 1000, 10000] {
        let mut doc = Document::default();
        place(&mut doc, &source, [59., 51.]);
        let original = doc.objects[0].clone();
        for i in 0..count {
            let mut placed = original.clone();
            placed.id = doc.next_id;
            doc.next_id += 1;
            placed.position = [230. + (i % 100) as f32 * 43., 240. + (i / 100) as f32 * 41.];
            doc.objects.push(placed);
        }
        let expected = reference(&doc, 0, 2.);
        assert!(expected.data() == render_scaled(&doc, 0, 2.).unwrap().data());
        let mut times = [Vec::new(), Vec::new()];
        for pass in 0..15 {
            for route in [pass % 2, 1 - pass % 2] {
                let start = Instant::now();
                let image = if route == 0 {
                    reference(&doc, 0, 2.)
                } else {
                    render_scaled(&doc, 0, 2.).unwrap()
                };
                if pass >= 2 {
                    times[route].push(start.elapsed());
                }
                black_box(image);
            }
        }
        for route in &mut times {
            route.sort_unstable();
        }
        println!(
            "{count} remote placements: reference {:?}, cached {:?}",
            times[0][6], times[1][6]
        );
    }
}
