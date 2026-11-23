use document_core::{Document, Gradient, GradientKind, GradientStop, LineCap, PathNode, Shape};
use renderer::{render, render_scaled, sample_artwork};

#[test]
fn transparent_stroke_extends_editing_bounds_but_not_export_crops_or_picking() {
    use document_core::arrange::{plan_flip, position_updates, Action, Alignment, Reference};
    let mut doc = Document::default();
    let a = doc.add_rectangle([16., 16.], [20., 20.], [235; 4]);
    let b = doc.add_rectangle([64., 16.], [20., 20.], [235; 4]);
    doc.objects[0].stroke = [128, 40, 72, 0];
    doc.objects[0].stroke_width = 8.;
    for (no_stroke, left) in [(false, 12.), (true, 16.)] {
        doc.objects[0].no_stroke = no_stroke;
        let object = &doc.objects[0];
        assert_eq!(object.bounds(0), [left, left, 52. - left, 52. - left]);
        assert_eq!(object.bounds_within([16., 16., 36., 36.], 0), no_stroke);
        assert_eq!(doc.hit_test([14., 24.], 0), None);
        assert_eq!(
            renderer::artwork_bounds(&[object], 0, 1).unwrap(),
            Some([16, 16, 36, 36])
        );
        let moves = position_updates(
            &doc,
            &[a, b],
            0,
            Action::Align(Alignment::Left, Reference::Bounds),
        )
        .unwrap();
        assert_eq!(moves, vec![(1, [left, 16.])]);
        let flips = plan_flip(&doc, &[a, b], 0, true).unwrap();
        assert_eq!(
            flips.iter().find(|f| f.index == 1).unwrap().position,
            [left, 16.]
        );
    }
    doc.objects[0].no_stroke = false;
    doc.objects[0].stroke = [255; 4];
    doc.objects[0].stroke_width = 2.;
    doc.objects[0].fill[3] = 0;
    for no_fill in [false, true] {
        doc.objects[0].no_fill = no_fill;
        assert_eq!(doc.objects[0].bounds(0), [15., 15., 37., 37.]);
    }
}

#[test]
fn disabled_paints_skip_color_gradients_and_hit_testing_without_erasing_them() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([16., 16.], [24., 24.], [180, 0, 0, 255]);
    let gradient = Gradient::from_color(
        GradientKind::Linear {
            start: [0.; 2],
            end: [1., 0.],
        },
        [10, 200, 40, 255],
    );
    let o = &mut doc.objects[0];
    o.stroke_width = 6.;
    o.gradient = Some(gradient.clone());
    o.stroke_gradient = Some(gradient);
    for (no_fill, no_stroke) in [(true, false), (false, true), (true, true), (false, false)] {
        doc.objects[0].no_fill = no_fill;
        doc.objects[0].no_stroke = no_stroke;
        let image = render(&doc, 0).unwrap();
        for (x, visible) in [(22, !no_fill), (15, !no_stroke)] {
            assert_eq!(image.pixel(x, 24).unwrap().alpha() > 0, visible);
            assert_eq!(
                doc.hit_test([x as f32 + 0.5, 24.5], 0),
                visible.then_some(id)
            );
        }
    }
}

#[test]
fn stroke_gradients_span_line_thickness_and_follow_pressure_rotation_and_zoom() {
    let center = [0.5; 2];
    for kind in [
        GradientKind::Linear {
            start: center,
            end: [0.5, 1.],
        },
        GradientKind::Radial {
            center,
            radius: 0.5,
        },
        GradientKind::Rectangular {
            center,
            radius: 0.5,
        },
    ] {
        let mut doc = Document::default();
        let id = doc.add_shape(
            [12.5, 16.5],
            [48., 16.],
            [0; 4],
            Shape::Path {
                nodes: [[0., 0.5], [1., 0.5]].map(PathNode::corner).into(),
                closed: false,
            },
            1,
        );
        let object = &mut doc.objects[0];
        object.stroke = [0; 4];
        object.stroke_width = 8.;
        object.animated.width_keys.extend([(0, 8.), (1, 16.)]);
        object.rotation_keys.extend([(0, 0.), (2, 180.)]);
        object.stroke_gradient = Some(Gradient {
            kind,
            stops: [
                (0., [250, 0, 0, 255]),
                (0.5, [0, 0, 220, 128]),
                (0.65, [0, 0, 220, 0]),
                (1., [0; 4]),
            ]
            .map(|(offset, color)| GradientStop { offset, color })
            .into(),
        });
        for profile in [vec![], vec![[0., 0.2], [0.5, 1.], [1., 0.2]]] {
            doc.objects[0].stroke_profile = profile;
            for frame in [0, 1] {
                let image = render(&doc, frame).unwrap();
                let zoom = render_scaled(&doc, frame, 3.).unwrap();
                for (dy, [red, blue, alpha]) in
                    [(0, [250, 0, 255]), (2, [0, 110, 128]), (3, [0, 0, 0])]
                {
                    let (x, y) = if frame == 0 {
                        (36, 24 + dy)
                    } else {
                        (36 - 2 * dy, 24)
                    };
                    let point = [x as f32 + 0.5, y as f32 + 0.5];
                    let pixel = image.pixel(x, y).unwrap();
                    assert_eq!(doc.hit_test(point, frame) == Some(id), alpha > 0);
                    for (v, expected) in [
                        (pixel.red(), red),
                        (pixel.blue(), blue),
                        (pixel.alpha(), alpha),
                    ] {
                        assert!(
                            v.abs_diff(expected) <= 1,
                            "at {point:?}, got {pixel:?}, expected [{red},0,{blue},{alpha}]"
                        );
                    }
                    let dense = zoom.pixel(3 * x + 1, 3 * y + 1).unwrap();
                    assert!(dense.alpha().abs_diff(pixel.alpha()) <= 1);
                    let sample = sample_artwork(&doc, frame, point, [1.; 2]).unwrap();
                    assert!(sample[3].abs_diff(pixel.alpha()) <= 1);
                }
            }
        }
    }
}

#[test]
fn gradient_stroke_supports_off_page_caps_and_object_opacity_over_fill() {
    let mut doc = Document::default();
    let id = doc.add_shape(
        [-16., 4.],
        [60., 20.],
        [0; 4],
        Shape::Path {
            nodes: [[0., 0.5], [1., 0.5]].map(PathNode::corner).into(),
            closed: false,
        },
        1,
    );
    let object = &mut doc.objects[0];
    object.cap = LineCap::Square;
    object.stroke_width = 8.;
    object.stroke = [0; 4];
    object.stroke_gradient = Some(Gradient {
        kind: GradientKind::Rectangular {
            center: [0.5; 2],
            radius: 0.5,
        },
        stops: vec![
            GradientStop {
                offset: 0.,
                color: [255, 255, 255, 255],
            },
            GradientStop {
                offset: 1.,
                color: [0, 0, 255, 255],
            },
        ],
    });
    let cap = [-18.5, 13.5];
    assert_eq!(doc.hit_test(cap, 0), Some(id));
    assert_eq!(
        sample_artwork(&doc, 0, cap, [1.; 2]).unwrap(),
        [0, 0, 255, 255]
    );
    assert!(doc.objects[0].bounds(0)[0] <= -20.);
    doc.objects[0].shape = Shape::Rectangle;
    doc.objects[0].fill = [220, 0, 0, 255];
    doc.objects[0].opacity = 0.5;
    let image = render(&doc, 0).unwrap();
    for (x, y) in [(1, 12), (42, 12), (45, 12)] {
        assert_eq!(image.pixel(x, y).unwrap().alpha(), 128);
    }
}

#[test]
fn editable_text_can_be_rendered_and_picked_by_its_gradient_outline_alone() {
    let mut doc = Document::default();
    let id = document_core::typography::TextStyle {
        content: "Hi".into(),
        size: 38.,
        ..Default::default()
    }
    .insert(&mut doc, 1, [18., 16.], [0; 4])
    .unwrap();
    let object = &mut doc.objects[0];
    object.stroke = [0; 4];
    object.stroke_width = 4.;
    object.stroke_gradient = Some(Gradient::from_color(
        GradientKind::Linear {
            start: [0., 0.5],
            end: [1., 0.5],
        },
        [0, 150, 220, 220],
    ));
    let image = render(&doc, 0).unwrap();
    let (i, _) = image
        .pixels()
        .iter()
        .enumerate()
        .find(|(_, c)| c.alpha() > 180)
        .unwrap();
    let x = i as u32 % image.width();
    let y = i as u32 / image.width();
    let point = [x as f32 + 0.5, y as f32 + 0.5];
    assert_eq!(doc.hit_test(point, 0), Some(id));
    assert_eq!(doc.text_at(point, 0).unwrap().0.id, id);
    doc.objects[0].stroke_width = 0.;
    assert!(render(&doc, 0)
        .unwrap()
        .pixels()
        .iter()
        .all(|c| c.alpha() == 0));
}
