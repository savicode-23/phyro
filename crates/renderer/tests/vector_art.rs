use document_core::{Document, Layer, LineCap, LineJoin, PathNode, Shape};
use renderer::{render, render_scaled};

const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

#[test]
fn rectangular_fill_tracks_the_object_and_keeps_box_contours_and_hard_alpha_stops() {
    use document_core::{Gradient, GradientKind, GradientStop};
    let mut doc = Document::default();
    let id = doc.add_rectangle([10.5, 12.5], [48., 32.], [0; 4]);
    doc.objects[0].gradient = Some(Gradient {
        kind: GradientKind::Rectangular {
            center: [0.5; 2],
            radius: 0.5,
        },
        stops: [
            (0., RED),
            (0.5, [0, 0, 255, 128]),
            (0.75, [0, 0, 255, 128]),
            (0.75, [0, 0, 255, 0]),
            (1., [0, 0, 255, 0]),
        ]
        .map(|(offset, color)| GradientStop { offset, color })
        .into(),
    });
    doc.objects[0].rotation_keys.extend([(0, 0.), (4, 180.)]);
    for frame in [0, 2] {
        let image = render(&doc, frame).unwrap();
        let zoom = render_scaled(&doc, frame, 3.).unwrap();
        let point = |local| doc.objects[0].local_to_world(local, frame);
        let inspect = |local| {
            let p = point(local);
            let (x, y) = (p[0].floor() as u32, p[1].floor() as u32);
            let pixel = image.pixel(x, y).unwrap();
            assert_eq!(pixel, zoom.pixel(3 * x + 1, 3 * y + 1).unwrap());
            let sample = renderer::sample_artwork(&doc, frame, p, [1.; 2]).unwrap();
            assert_eq!(
                sample,
                [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
            );
            assert_eq!(doc.hit_test(p, frame) == Some(id), pixel.alpha() > 0);
            sample
        };
        assert_eq!(inspect([0.5, 0.5]), RED);
        for local in [[0.75, 0.5], [0.5, 0.75], [0.75, 0.75], [0.25, 0.25]] {
            assert_eq!(inspect(local), [0, 0, 128, 128]);
        }
        assert_eq!(inspect([0.9375, 0.5]), [0; 4]);
    }
}

#[test]
fn rotated_rectangular_gradient_matches_eyedropper_across_narrow_stop_intervals() {
    use document_core::{Gradient, GradientKind, GradientStop};
    let mut doc = Document::default();
    doc.add_rectangle([22., 34.], [72., 46.], [0; 4]);
    let object = &mut doc.objects[0];
    object.rotation = 27.;
    object.scale = [-1.1, 0.8];
    object.gradient = Some(Gradient {
        kind: GradientKind::Rectangular {
            center: [0.36, 0.57],
            radius: 0.48,
        },
        stops: [
            (0., RED),
            (0.71, [43, 17, 88, 190]),
            (0.7101, [55, 208, 38, 30]),
            (0.7101, [77, 44, 130, 90]),
            (0.7102, [50, 160, 200, 160]),
            (1., BLUE),
        ]
        .map(|(offset, color)| GradientStop { offset, color })
        .into(),
    });
    let image = render(&doc, 0).unwrap();
    for y in (25..100).step_by(3) {
        for x in (16..100).step_by(3) {
            let pos = [x as f32 + 0.5, y as f32 + 0.5];
            if !doc.objects[0]
                .world_to_local(pos, 0)
                .into_iter()
                .all(|v| (0.02..0.98).contains(&v))
            {
                continue;
            }
            let pixel = image.pixel(x, y).unwrap();
            let sample = renderer::sample_artwork(&doc, 0, pos, [1.; 2]).unwrap();
            assert_eq!(
                sample,
                [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()],
                "at {pos:?}"
            );
        }
    }
}

#[test]
fn masked_gradient_fills_follow_the_evaluated_object_at_every_zoom() {
    use document_core::{Gradient, GradientKind, GradientStop, Stencil};
    let mut doc = Document::default();
    let id = doc.add_rectangle([10., 10.], [40., 20.], [0; 4]);
    let o = &mut doc.objects[0];
    o.gradient = Some(Gradient {
        kind: GradientKind::Linear {
            start: [0., 0.],
            end: [1., 0.],
        },
        stops: vec![
            GradientStop {
                offset: 0.,
                color: RED,
            },
            GradientStop {
                offset: 1.,
                color: BLUE,
            },
        ],
    });
    o.stroke = RED;
    o.stroke_width = 4.;
    o.stencil = Some(Stencil {
        fill_rule: Default::default(),
        shape: Shape::Ellipse,
        origin: [0.; 2],
        size: [1.; 2],
        alpha: Some([0, 255]),
    });
    let native = render(&doc, 0).unwrap();
    let zoom = render_scaled(&doc, 0, 3.).unwrap();
    for (x, y, hit) in [
        (10, 10, false),
        (13, 20, true),
        (29, 20, true),
        (46, 20, true),
    ] {
        assert_eq!(
            doc.hit_test([x as f32 + 0.5, y as f32 + 0.5], 0) == Some(id),
            hit
        );
        assert_eq!(
            native.pixel(x, y).unwrap(),
            zoom.pixel(3 * x + 1, 3 * y + 1).unwrap()
        );
    }
    let start = native.pixel(13, 20).unwrap();
    let end = native.pixel(46, 20).unwrap();
    assert!(
        start.alpha() < 32
            && end.alpha() > 224
            && start.red() > start.blue()
            && end.blue() > end.red()
    );
    assert_eq!(native.pixel(29, 20).unwrap().alpha(), 124);
    let o = &mut doc.objects[0];
    o.stencil.as_mut().unwrap().alpha = None;
    o.rotation_keys.extend([(0, 0.), (4, 180.)]);
    o.gradient.as_mut().unwrap().kind = GradientKind::Radial {
        center: [0.5, 0.5],
        radius: 0.5,
    };
    let rotated = render(&doc, 2).unwrap();
    assert!(rotated.pixel(30, 20).unwrap().red() > 225);
    assert!(rotated.pixel(30, 3).unwrap().blue() > 195);
    assert_eq!(rotated.pixel(20, 3).unwrap().alpha(), 0);
    assert_eq!(doc.hit_test([20.5, 3.5], 2), None);
}

#[test]
fn evaluated_geometry_matches_hits_and_object_opacity_composites_overlapping_paints() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([16., 16.], [8., 8.], BLUE);
    let object = &mut doc.objects[0];
    object.stroke = RED;
    object.stroke_width = 2.;
    object.scale_keys.extend([(0, [1., 1.]), (4, [3., 1.])]);
    object.rotation_keys.extend([(0, 0.), (4, 180.)]);
    object.opacity_keys.extend([(0, 1.), (4, 0.)]);
    let at_mid = render(&doc, 2).unwrap();
    for (x, y, in_shape) in [
        (20, 20, true),
        (20, 12, true),
        (20, 11, true),
        (20, 10, false),
        (14, 20, false),
    ] {
        let pixel = at_mid.pixel(x, y).unwrap();
        assert_eq!(
            doc.hit_test([x as f32 + 0.5, y as f32 + 0.5], 2) == Some(id),
            in_shape
        );
        if in_shape {
            assert!(pixel.alpha().abs_diff(128) <= 1);
        } else {
            assert_eq!(pixel.alpha(), 0);
        }
    }
    assert!(at_mid.pixel(20, 12).unwrap().red() > 120);
    assert!(at_mid.pixel(20, 20).unwrap().blue() > 120);
    assert_eq!(render(&doc, 4).unwrap().pixel(20, 20).unwrap().alpha(), 0);
    assert_eq!(doc.hit_test([20., 20.], 4), None);
    assert_eq!(doc.objects[0].scale, [1.; 2]);
}

#[test]
fn layers_control_paint_order_and_visibility() {
    let mut doc = Document::default();
    doc.layers.push(Layer {
        id: 2,
        name: "Foreground".into(),
        visible: true,
        locked: true,
    });
    doc.add_shape([4.0, 4.0], [16.0, 16.0], RED, Shape::Ellipse, 2);
    doc.add_rectangle([4.0, 4.0], [16.0, 16.0], BLUE);
    let painted = render(&doc, 0).unwrap();
    assert_eq!(painted.pixel(12, 12).unwrap().red(), 255);
    assert_eq!(painted.pixel(4, 4).unwrap().blue(), 255);
    doc.layers[1].visible = false;
    assert_eq!(render(&doc, 0).unwrap().pixel(12, 12).unwrap().blue(), 255);
}

#[test]
fn both_shapes_rotate_around_their_centers() {
    for shape in [Shape::Rectangle, Shape::Ellipse] {
        let mut doc = Document::default();
        doc.add_shape([8.0, 12.0], [24.0, 8.0], RED, shape, 1);
        doc.objects[0].rotation = 90.0;
        let image = render(&doc, 0).unwrap();
        assert_eq!(image.pixel(20, 8).unwrap().alpha(), 255);
        assert_eq!(image.pixel(11, 16).unwrap().alpha(), 0);
    }
}

#[test]
fn pen_curve_fills_an_implicitly_closed_contour() {
    let mut doc = Document::default();
    // A bowed segment with both endpoints on the baseline. Its fill closes
    // straight across them; a straight implementation would have zero area.
    let mut a = PathNode::corner([0.0, 1.0]);
    let mut b = PathNode::corner([1.0, 1.0]);
    a.outgoing = [0.0, 0.0];
    b.incoming = [1.0, 0.0];
    doc.add_shape(
        [4.0, 4.0],
        [32.0, 32.0],
        BLUE,
        Shape::Path {
            nodes: vec![a, b],
            closed: false,
        },
        1,
    );
    let image = render(&doc, 0).unwrap();
    assert_eq!(image.pixel(20, 16).unwrap().alpha(), 255);
    assert_eq!(image.pixel(20, 9).unwrap().alpha(), 0);
    assert_eq!(image.pixel(20, 37).unwrap().alpha(), 0);
}

#[test]
fn stroke_width_is_in_artboard_pixels_and_open_caps_are_round() {
    let mut doc = Document::default();
    let nodes = vec![PathNode::corner([0.0, 0.5]), PathNode::corner([1.0, 0.5])];
    doc.add_shape(
        [12.0, 6.0],
        [64.0, 20.0],
        [0; 4],
        Shape::Path {
            nodes,
            closed: false,
        },
        1,
    );
    doc.objects[0].stroke = RED;
    doc.objects[0].stroke_width = 8.0;
    let image = render(&doc, 0).unwrap();
    assert_eq!(image.pixel(32, 12).unwrap().alpha(), 255);
    assert_eq!(image.pixel(32, 11).unwrap().alpha(), 0);
    assert_eq!(image.pixel(9, 16).unwrap().alpha(), 255);
    assert_eq!(image.pixel(8, 12).unwrap().alpha(), 0);
    let zoom = render_scaled(&doc, 0, 2.0).unwrap();
    assert_eq!(zoom.pixel(64, 24).unwrap().alpha(), 255);
    assert_eq!(zoom.pixel(64, 23).unwrap().alpha(), 0);

    doc.objects[0].cap = LineCap::Butt;
    assert_eq!(render(&doc, 0).unwrap().pixel(9, 16).unwrap().alpha(), 0);
    doc.objects[0].cap = LineCap::Square;
    assert_eq!(render(&doc, 0).unwrap().pixel(8, 12).unwrap().alpha(), 255);
}

#[test]
fn expanding_a_rotated_linear_stroke_preserves_its_gradient_and_selects_the_outline() {
    use document_core::{Gradient, GradientKind, GradientStop};
    let mut doc = Document::default();
    let id = doc.add_rectangle([38., 35.], [44., 28.], [0; 4]);
    let object = &mut doc.objects[0];
    object.rotation = 23.;
    object.stroke_width = 9.;
    object.stroke_gradient = Some(Gradient {
        kind: GradientKind::Linear {
            start: [0.05, 0.1],
            end: [0.9, 0.7],
        },
        stops: vec![
            GradientStop {
                offset: 0.,
                color: BLUE,
            },
            GradientStop {
                offset: 1.,
                color: RED,
            },
        ],
    });
    let before = render(&doc, 0).unwrap();
    assert_eq!(
        document_core::path_edit::stroke_to_paths(&mut doc, &[id], 0).unwrap(),
        [id]
    );
    let after = render(&doc, 0).unwrap();
    assert!(doc.objects[0].no_stroke);
    assert_eq!(doc.objects[0].shape.contours().len(), 2);
    let different = before
        .data()
        .chunks_exact(4)
        .zip(after.data().chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
        .count();
    assert!(different < 12, "{different} pixels differed");
}

#[test]
fn pressure_stroke_conversion_keeps_variable_width_as_filled_geometry() {
    let mut doc = Document::default();
    let id = doc
        .add_artboard_path(
            [[16., 40.], [108., 40.]].map(PathNode::corner).to_vec(),
            false,
            1,
            [0; 4],
        )
        .unwrap();
    doc.objects[0].stroke = RED;
    doc.objects[0].stroke_width = 18.;
    doc.objects[0].stroke_profile = vec![[0., 0.25], [0.5, 1.], [1., 0.25]];
    let before = render(&doc, 0).unwrap();
    assert_eq!(
        document_core::path_edit::stroke_to_paths(&mut doc, &[id], 0).unwrap(),
        [id]
    );
    assert!(doc.objects[0].stroke_profile.is_empty());
    assert_eq!(render(&doc, 0).unwrap(), before);
}

#[test]
fn stroke_join_changes_the_exterior_corner() {
    let mut doc = Document::default();
    doc.add_rectangle([12.0, 12.0], [20.0, 20.0], [0; 4]);
    doc.objects[0].stroke = RED;
    doc.objects[0].stroke_width = 12.0;
    doc.objects[0].join = LineJoin::Miter;
    assert_eq!(render(&doc, 0).unwrap().pixel(7, 7).unwrap().alpha(), 255);
    doc.objects[0].join = LineJoin::Bevel;
    assert_eq!(render(&doc, 0).unwrap().pixel(7, 7).unwrap().alpha(), 0);
}

#[test]
fn acute_corners_bevel_past_the_miter_limit_and_pressure_uses_round_joins() {
    for (ends, profile) in [
        ([40., 56.], vec![]),
        ([14., 82.], vec![[0., 0.5], [0.5, 1.], [1., 0.7]]),
    ] {
        let mut doc = Document::default();
        let nodes = [[ends[0], 64.], [48., 24.], [ends[1], 64.]].map(PathNode::corner);
        doc.add_artboard_path(nodes.to_vec(), false, 1, [0; 4])
            .unwrap();
        let object = &mut doc.objects[0];
        object.stroke = RED;
        object.stroke_width = 12.;
        object.stroke_profile = profile;
        let miter = render(&doc, 0).unwrap();
        doc.objects[0].join = LineJoin::Bevel;
        assert_eq!(miter, render(&doc, 0).unwrap());
        doc.objects[0].join = LineJoin::Round;
        let round = render(&doc, 0).unwrap();
        assert_eq!(miter == round, !doc.objects[0].stroke_profile.is_empty());
    }
}
