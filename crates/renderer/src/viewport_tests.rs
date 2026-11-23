use super::*;
use document_core::{AnimationProject, Stencil};
use tiny_skia::{Mask, PathBuilder, PixmapPaint};

// Reference compositor for clip placements. Keep the viewport in source space
// and composite the entire intermediate surface, as in the original renderer.
fn draw_full_surface(target: &mut Pixmap, doc: &Document, transform: Transform) {
    let mut scratch = None;
    for object in doc.paint_order() {
        if let Some((child, _)) = object.instance_at(0) {
            let local = object.symbol_transform(0).unwrap().post_concat(transform);
            let mut layer = Pixmap::new(target.width(), target.height()).unwrap();
            draw_full_surface(&mut layer, child, local);
            let viewport = PathBuilder::from_rect(
                Rect::from_xywh(0., 0., child.width as f32, child.height as f32).unwrap(),
            );
            let mut mask = appearance::mask(target, object, 0, transform).unwrap();
            if let Some(mask) = &mut mask {
                mask.intersect_path(&viewport, FillRule::Winding, true, local);
            } else {
                let mut viewport_mask = Mask::new(target.width(), target.height()).unwrap();
                viewport_mask.fill_path(&viewport, FillRule::Winding, true, local);
                mask = Some(viewport_mask);
            }
            target.draw_pixmap(
                0,
                0,
                layer.as_ref(),
                &PixmapPaint {
                    opacity: object.opacity_at(0),
                    ..Default::default()
                },
                Transform::identity(),
                mask.as_ref(),
            );
        } else {
            draw_with_appearance(target, object, 0, transform, [0.; 2], &mut scratch, None)
                .unwrap();
        }
    }
}

#[test]
fn successive_isolated_paths_and_unbounded_symbols_match_fresh_full_layers() {
    use document_core::{
        Gradient, GradientKind, GradientStop, LineCap, LineJoin, PathEffect, PathNode,
        SymbolInstance,
    };
    let mut doc = Document::default();
    let nodes = vec![
        PathNode::corner([0., 0.7]),
        PathNode {
            handle_style: Default::default(),
            point: [0.5, 0.02],
            incoming: [0.65, -0.4],
            outgoing: [0.8, 0.1],
        },
        PathNode::corner([1.05, 0.9]),
        PathNode::corner([0.15, 1.]),
    ];
    // Alternate overlapping and disjoint rectangles that share scanlines, then
    // revisit an earlier part of the scratch surface and cross the artboard edge.
    for (i, position) in [[5., 13.], [24., 18.], [80., 12.], [7., 15.], [-15., 83.]]
        .into_iter()
        .enumerate()
    {
        doc.add_shape(
            position,
            [26., 19.],
            [25, 100, 145, 201],
            Shape::Path {
                nodes: nodes.clone(),
                closed: i % 2 == 0,
            },
            1,
        );
        let o = doc.objects.last_mut().unwrap();
        o.stroke = [245, 135, 24, 182];
        o.stroke_width = 4.7;
        o.cap = LineCap::Square;
        o.join = [LineJoin::Miter, LineJoin::Bevel, LineJoin::Round][i % 3];
        o.rotation = 23. - i as f32 * 11.;
        o.animated.skew = 14.;
        o.opacity = 0.46 + i as f32 * 0.06;
        o.effects.push(PathEffect::Mirror { horizontal: true });
        if i == 1 {
            o.effects.push(PathEffect::RoundCorners { distance: 2.1 });
            o.gradient = Some(Gradient {
                kind: GradientKind::Linear {
                    start: [0., 0.1],
                    end: [0.8, 1.],
                },
                stops: vec![
                    GradientStop {
                        offset: 0.,
                        color: [20, 160, 90, 250],
                    },
                    GradientStop {
                        offset: 1.,
                        color: [240, 15, 78, 140],
                    },
                ],
            });
            o.stencil = Some(Stencil {
                shape: Shape::Ellipse,
                origin: [-0.1, 0.05],
                size: [1.3, 0.9],
                fill_rule: Default::default(),
                alpha: Some([45, 235]),
            });
        }
    }
    let mut child = Document {
        width: 20,
        height: 20,
        ..Default::default()
    };
    child.add_rectangle([-11., -13.], [48., 51.], [99, 29, 180, 243]);
    doc.add_rectangle([18., 28.], [29., 24.], [0; 4]);
    let instance = doc.objects.last_mut().unwrap();
    instance.symbol = Some(SymbolInstance::new(1, std::sync::Arc::new(child)));
    instance.opacity = 0.62;
    // A full-width band exercises the contiguous-row clear after the symbol.
    doc.add_rectangle([-420., 43.], [1000., 9.], [125, 40, 60, 173]);
    let band = doc.objects.last_mut().unwrap();
    band.stroke_width = 2.5;
    band.opacity = 0.56;
    let key = doc.objects[0].position;
    doc.objects[0].keys.extend([(0, key), (2, [79., 17.])]);
    for frame in [0, 2] {
        for transform in [
            Transform::from_scale(1.31, 1.31),
            Transform::from_row(-1.15, 0.17, 0.23, 1.08, 147., 11.),
        ] {
            let mut expected = Pixmap::new(191, 173).unwrap();
            let mut actual = expected.clone();
            let mut scratch = None;
            for o in doc.paint_order() {
                draw_with_appearance(
                    &mut actual,
                    o,
                    frame,
                    transform,
                    [0.; 2],
                    &mut scratch,
                    None,
                )
                .unwrap();
                let mut fresh = Pixmap::new(expected.width(), expected.height()).unwrap();
                if o.is_instance() {
                    symbol::draw_symbol_content(
                        &mut fresh,
                        o,
                        frame,
                        transform,
                        [0.; 2],
                        RenderPurpose::Editor,
                    )
                    .unwrap();
                } else {
                    draw_object(&mut fresh, o, frame, transform, [0.; 2], 1.);
                }
                let mask = appearance::mask(&expected, o, frame, transform).unwrap();
                expected.draw_pixmap(
                    0,
                    0,
                    fresh.as_ref(),
                    &PixmapPaint {
                        opacity: o.opacity_at(frame),
                        ..Default::default()
                    },
                    Transform::identity(),
                    mask.as_ref(),
                );
                assert!(
                    actual.data() == expected.data(),
                    "object {}, frame {frame}, transform {transform:?}",
                    o.id
                );
            }
        }
    }
}

#[test]
fn translucent_stenciled_strokes_match_a_full_surface_composite() {
    let mut doc = Document::default();
    doc.add_rectangle([0.; 2], [128.; 2], [63, 138, 37, 255]);
    doc.add_shape(
        [-18., 3.],
        [69., 42.],
        [127, 85, 200, 202],
        Shape::Ellipse,
        1,
    );
    let obj = &mut doc.objects[1];
    obj.rotation = -31.;
    obj.stroke = [220, 210, 20, 159];
    obj.stroke_width = 6.7;
    obj.opacity = 0.61;
    obj.stencil = Some(Stencil {
        shape: Shape::Ellipse,
        origin: [0.05, -0.18],
        size: [1.15, 1.35],
        fill_rule: Default::default(),
        alpha: Some([60, 245]),
    });
    let scale = 1.75;
    let actual = render_scaled(&doc, 0, scale).unwrap();
    let mut expected = Pixmap::new(actual.width(), actual.height()).unwrap();
    let transform = Transform::from_scale(scale, scale);
    draw_object(&mut expected, &doc.objects[0], 0, transform, [0.; 2], 1.);
    let mut layer = Pixmap::new(actual.width(), actual.height()).unwrap();
    let obj = &doc.objects[1];
    draw_object(&mut layer, obj, 0, transform, [0.; 2], 1.);
    let mask = appearance::mask(&expected, obj, 0, transform).unwrap();
    expected.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint {
            opacity: obj.opacity,
            ..Default::default()
        },
        Transform::identity(),
        mask.as_ref(),
    );
    assert!(actual.data() == expected.data());
}

#[test]
fn bounded_viewport_matches_full_surface_with_transformed_nested_clips() {
    let mut doc = Document {
        width: 64,
        height: 64,
        ..Default::default()
    };
    let background = [41, 18, 37, 255];
    doc.add_rectangle([0.; 2], [64.; 2], background);
    let mut project = AnimationProject::from_document(doc);
    let middle = project.add_clip("Outer content").unwrap();
    let source = project.add_clip("Inner content").unwrap();
    let art = project.clips[source].drawing_at_mut(0);
    art.add_rectangle([-23., -12.], [109., 100.], [100, 85, 200, 255]);
    art.add_shape(
        [11.5, 0.],
        [48., 63.],
        [177, 89, 27, 157],
        Shape::Ellipse,
        1,
    );
    let inner = project
        .place_clip(project.clips[source].id, middle, 0, 1, [-4.2, 12.7])
        .unwrap();
    let o = project.clips[middle]
        .drawing_at_mut(0)
        .objects
        .iter_mut()
        .find(|o| o.id == inner)
        .unwrap();
    o.rotation = -17.3;
    o.opacity = 0.86;
    let outer = project
        .place_clip(project.clips[middle].id, 0, 0, 1, [0.; 2])
        .unwrap();
    for stencil in [
        None,
        Some(Stencil {
            shape: Shape::Ellipse,
            origin: [-0.1, 0.07],
            size: [1.2, 1.],
            fill_rule: Default::default(),
            alpha: Some([60, 255]),
        }),
    ] {
        for (position, rotation) in [
            ([0.6, -2.3], 0.),
            ([-26.7, -28.2], 37.),
            ([21.2, 24.6], -43.5),
            ([120., 40.], 8.),
        ] {
            let doc = project.clips[0].drawing_at_mut(0);
            let o = doc.objects.iter_mut().find(|o| o.id == outer).unwrap();
            o.position = position;
            o.rotation = rotation;
            o.scale = [0.62, 0.41];
            o.animated.skew = 13.;
            o.opacity = 0.73;
            o.stencil = stencil.clone();
            let actual = render_scaled(doc, 0, 1.75).unwrap();
            let mut reference = Pixmap::new(actual.width(), actual.height()).unwrap();
            draw_full_surface(&mut reference, doc, Transform::from_scale(1.75, 1.75));
            assert!(
                actual.data() == reference.data(),
                "different pixels at {position:?}, rotation {rotation}"
            );
            let touched = actual
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p != &background);
            assert_eq!(touched, position[0] < 100.);
        }
    }
}

mod shared_symbols;

#[test]
fn stacked_clip_copies_match_independent_full_surface_masks() {
    let mut page = Document::default();
    page.add_rectangle([0.; 2], [128.; 2], [99, 58, 39, 255]);
    let mut project = AnimationProject::from_document(page);
    let source = project.add_clip("Paint outside viewport").unwrap();
    let art = project.clips[source].drawing_at_mut(0);
    // Child ink outside its viewport leaves pixels elsewhere in a reused scratch layer.
    art.add_rectangle([-120.; 2], [370.; 2], [27, 198, 146, 119]);
    art.add_shape(
        [40., 15.],
        [112., 82.],
        [204, 83, 52, 210],
        Shape::Ellipse,
        1,
    );
    let clip = project.clips[source].id;
    for (i, (position, rotation, scale)) in [
        ([8., 12.], 0., [1., 1.]),
        ([22., 17.], 0., [-1., 1.]),
        ([83., 80.], 0., [1., -1.]),
        ([-7., 25.], 0., [1., 1.]),
        ([14.3, 15.6], 0., [1.17, 0.69]),
        ([59., 72.], 33.4, [-1., 1.]),
        ([178., 81.], -12., [1., 1.]),
        ([8., 12.], 0., [1., 1.]),
    ]
    .into_iter()
    .enumerate()
    {
        let id = project.place_clip(clip, 0, 0, 1, position).unwrap();
        let o = project.clips[0]
            .drawing_at_mut(0)
            .objects
            .last_mut()
            .unwrap();
        assert_eq!(o.id, id);
        o.size = [32., 24.];
        o.rotation = rotation;
        o.scale = scale;
        o.opacity = 0.72;
        if i % 2 == 1 {
            o.stencil = Some(Stencil {
                shape: Shape::Ellipse,
                origin: [-0.2, 0.07],
                size: [1.4, 1.2],
                fill_rule: Default::default(),
                alpha: Some([51, 242]),
            });
        }
    }
    let scene = project.clips[0].drawing_at(0);
    for density in [1., 1.375, 2.] {
        let actual = render_scaled(scene, 0, density).unwrap();
        let mut reference = Pixmap::new(actual.width(), actual.height()).unwrap();
        draw_full_surface(
            &mut reference,
            scene,
            Transform::from_scale(density, density),
        );
        assert!(
            actual.data() == reference.data(),
            "coverage differed at {density}×"
        );
    }
}
