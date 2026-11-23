use super::*;
use crate::commands::Alignment::*;
use document_core::Shape;

fn rect(doc: &mut Document, pos: [f32; 2], size: [f32; 2]) -> u64 {
    doc.add_shape(pos, size, [180; 4], Shape::Rectangle, 1)
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} differs from {b}");
}

#[test]
fn tall_wide_and_large_area_references_keep_the_expected_object_fixed() {
    let mut doc = Document::default();
    let tall = rect(&mut doc, [8., 5.], [6., 85.]);
    let wide = rect(&mut doc, [40., 97.], [78., 4.]);
    let square = rect(&mut doc, [62., 15.], [42., 42.]);
    for (direction, fixed) in [
        (Left, 0),
        (CenterX, 0),
        (Right, 0),
        (Top, 1),
        (CenterY, 1),
        (Bottom, 1),
    ] {
        for (reference, index) in [
            (Reference::BiggestByDirection, fixed),
            (Reference::LargestArea, 2),
        ] {
            let mut arranged = doc.clone();
            let action = Action::Align(direction, reference);
            assert!(apply(
                &mut arranged,
                &[square, wide, tall],
                2,
                KeyMode::Auto,
                action
            )
            .unwrap());
            assert_eq!(arranged.objects[index], doc.objects[index]);
            let expected = direction.coordinate(doc.objects[index].bounds(2));
            for object in &arranged.objects {
                close(direction.coordinate(object.bounds(2)), expected);
            }
        }
    }
}

#[test]
fn repeated_alignment_on_far_pasteboard_curves_leaves_one_edit() {
    for origin in [15_000., -120_000.] {
        let mut original = Document::default();
        let pin = rect(&mut original, [origin, origin + 8.], [91., 73.]);
        let other = original.add_shape(
            [origin + 143., origin - 35.],
            [15., 24.],
            [190; 4],
            Shape::Ellipse,
            1,
        );
        original.objects[0].rotation = 37.;
        original.objects[1].rotation = -25.;
        original.objects[1].stroke_width = 3.;
        for direction in [Left, CenterX, Right, Top, CenterY, Bottom] {
            let mut doc = original.clone();
            let action = Action::Align(direction, Reference::LargestArea);
            assert!(apply(&mut doc, &[other, pin], 3, KeyMode::Auto, action).unwrap());
            let aligned = doc.clone();
            let target = direction.coordinate(doc.objects[0].bounds(3));
            let actual = direction.coordinate(doc.objects[1].bounds(3));
            assert!((target - actual).abs() <= f32::EPSILON * origin.abs() * 2.);
            assert!(
                !apply(&mut doc, &[other, pin], 3, KeyMode::Auto, action).unwrap(),
                "second alignment at {origin}: target {target}, actual {actual}"
            );
            assert_eq!(doc, aligned);
        }
    }
}

#[test]
fn all_six_directions_align_the_stroked_transformed_bounds_to_the_biggest() {
    let mut original = Document::default();
    let large = rect(&mut original, [47., 38.], [44., 39.]);
    let small = rect(&mut original, [10., 7.], [8., 13.]);
    let other = rect(&mut original, [122., 105.], [9., 6.]);
    original.objects[0].rotation = 17.;
    original.objects[1].rotation = -24.;
    original.objects[1].animated.skew = 16.;
    original.objects[1].stroke_width = 4.;
    for direction in [Left, CenterX, Right, Top, CenterY, Bottom] {
        let mut doc = original.clone();
        let action = Action::Align(direction, Reference::LargestArea);
        assert!(apply(
            &mut doc,
            &[other, small, large],
            0,
            KeyMode::Tracked,
            action
        )
        .unwrap());
        assert_eq!(doc.objects[0], original.objects[0]);
        let target = direction.coordinate(doc.objects[0].bounds(0));
        for o in &doc.objects[1..] {
            close(direction.coordinate(o.bounds(0)), target);
        }
        let aligned = doc.clone();
        assert!(!apply(&mut doc, &[large, small, other], 0, KeyMode::Auto, action).unwrap());
        assert_eq!(doc, aligned);
    }
}

#[test]
fn group_moves_as_one_unit_and_equal_areas_use_stable_document_order() {
    let mut doc = Document::default();
    let pin = rect(&mut doc, [58., 30.], [25., 10.]);
    let first = rect(&mut doc, [0., 5.], [5., 10.]);
    let last = rect(&mut doc, [20., 5.], [5., 10.]);
    let other_layer = commands::add_layer(&mut doc);
    let other = doc.add_shape([95., 7.], [9., 5.], [145; 4], Shape::Rectangle, other_layer);
    for o in &mut doc.objects[1..] {
        o.group = Some(12);
    }
    let untouched = doc.objects[3].clone();
    // Group and reference both measure 25×10. Selection order is reversed.
    assert_eq!(available_units(&doc, &[last, first, pin, 999]), 2);
    assert_eq!(available_units(&doc, &[last, pin, other]), 3);
    assert!(apply(
        &mut doc,
        &[last, pin],
        2,
        KeyMode::Auto,
        Action::Align(Right, Reference::LargestArea)
    )
    .unwrap());
    assert_eq!(doc.objects[0].position, [58., 30.]);
    assert!(doc.objects[0].keys.is_empty());
    assert_eq!(doc.objects[1].position_at(2), [58., 5.]);
    assert_eq!(doc.objects[2].position_at(2), [78., 5.]);
    assert_eq!(doc.objects[1].position_at(0), [0., 5.]);
    assert_eq!(doc.objects[3], untouched);
    doc.layers[1].locked = true;
    assert_eq!(available_units(&doc, &[other, last, pin]), 2);
    doc.layers[0].visible = false;
    assert_eq!(available_units(&doc, &[other, last, pin]), 0);
}

#[test]
fn negative_gaps_and_different_sizes_distribute_with_both_endpoints_fixed() {
    for (axis, action) in [(0, Action::DistributeX), (1, Action::DistributeY)] {
        for (far, gap) in [(90., 16.), (30., -4.)] {
            let mut doc = Document::default();
            let mut ids = Vec::new();
            for (position, width) in [(0., 10.), (12., 18.), (23., 14.), (far, 9.)] {
                let mut pos = [3.; 2];
                pos[axis] = position;
                let mut size = [7.; 2];
                size[axis] = width;
                ids.push(rect(&mut doc, pos, size));
            }
            let ends = [doc.objects[0].clone(), doc.objects[3].clone()];
            ids.reverse();
            assert!(apply(&mut doc, &ids, 3, KeyMode::Auto, action).unwrap());
            assert_eq!(doc.objects[0], ends[0]);
            assert_eq!(doc.objects[3], ends[1]);
            for objects in doc.objects.windows(2) {
                close(
                    objects[1].bounds(3)[axis] - objects[0].bounds(3)[axis + 2],
                    gap,
                );
            }
            let distributed = doc.clone();
            assert!(!apply(&mut doc, &ids, 3, KeyMode::Auto, action).unwrap());
            assert_eq!(doc, distributed);
        }
    }
}

#[test]
fn key_policy_checks_only_movers_and_refuses_the_whole_edit_on_failure() {
    let mut doc = Document::default();
    let pin = rect(&mut doc, [30.; 2], [20.; 2]);
    let a = rect(&mut doc, [4., 6.], [7.; 2]);
    let b = rect(&mut doc, [70., 3.], [3.; 2]);
    doc.objects[0].keys.insert(0, [30.; 2]);
    doc.objects[2].keys.insert(0, [70., 3.]);
    let before = doc.clone();
    let ids = [pin, a, b];
    assert!(apply(
        &mut doc,
        &ids,
        2,
        KeyMode::Manual,
        Action::Align(Left, Reference::LargestArea)
    )
    .is_err());
    assert_eq!(doc, before);
    doc.objects[2].keys.insert(2, [70., 3.]);
    Property::Position.set_muted(&mut doc.objects[0], true);
    let fixed = doc.objects[0].clone();
    assert!(apply(
        &mut doc,
        &ids,
        2,
        KeyMode::Manual,
        Action::Align(Left, Reference::LargestArea)
    )
    .unwrap());
    assert_eq!(doc.objects[0], fixed);
    assert_eq!(doc.objects[1].position, [30., 6.]);
    assert_eq!(doc.objects[2].keys[&2], [30., 3.]);
    assert_eq!(doc.objects[2].keys[&0], [70., 3.]);
}
