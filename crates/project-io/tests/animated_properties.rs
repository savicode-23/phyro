use document_core::{
    AnimationProject, Document, Easing, KeyValue, PathEffect, PathNode, Property, Shape,
};
use project_io::{export_svg, import_svg, load_animation, save_animation};

fn square(x: f32) -> Shape {
    Shape::Path {
        closed: true,
        nodes: [[x, 0.], [x + 0.5, 0.], [x + 0.5, 1.], [x, 1.]]
            .map(PathNode::corner)
            .to_vec(),
    }
}

#[test]
fn a_custom_morph_and_paint_tracks_survive_native_and_svg_and_match_picking() {
    let mut doc = Document::default();
    let id = doc.add_shape([10., 10.], [20., 16.], [255, 0, 0, 255], square(0.), 1);
    let obj = &mut doc.objects[0];
    obj.effects.push(PathEffect::Mirror { horizontal: true });
    assert!(Property::Geometry.add(obj, 0));
    assert!(Property::Geometry.set(obj, 4, KeyValue::Shape(square(0.5))));
    assert!(Property::Fill.add(obj, 0));
    assert!(Property::Fill.set(obj, 4, KeyValue::Color([0, 0, 255, 255])));
    assert!(Property::Geometry.set_easing(obj, 0, Easing::EaseIn));
    let core_shape = obj.shape.clone();
    let u = Easing::EaseIn.sample(0.5) as f32;
    assert!((obj.shape_at(2).contours()[0][0].point[0] - u * 0.5).abs() < 1e-5);
    assert!((obj.evaluated_shape_at(2).contours()[0][0].point[0] - (1. - u * 0.5)).abs() < 1e-5);
    assert_eq!(obj.fill_at(2), [128, 0, 128, 255]);
    assert_eq!(core_shape, obj.shape);
    assert!(!Property::Geometry.set(obj, 2, KeyValue::Shape(Shape::Ellipse)));
    assert!(Property::Visibility.add(obj, 0));
    assert!(Property::Visibility.set(obj, 4, KeyValue::Visible(false)));
    for tick in 0..5 {
        let image = renderer::render(&doc, tick).unwrap();
        for y in 0..42 {
            for x in 0..42 {
                let alpha = image.pixel(x, y).unwrap().alpha();
                if alpha == 255 || alpha == 0 {
                    assert_eq!(
                        doc.hit_test([x as f32 + 0.5, y as f32 + 0.5], tick) == Some(id),
                        alpha == 255
                    );
                }
            }
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let project = AnimationProject::from_document(doc);
    let native = tmp.path().join("morph.vanim");
    save_animation(&native, &project, false).unwrap();
    assert_eq!(project, load_animation(&native).unwrap());
    let doc = project.clips[0].drawing_at(2);
    let svg = tmp.path().join("pose.svg");
    export_svg(&svg, doc, 2, false).unwrap();
    let reimport = import_svg(&svg).unwrap();
    assert_eq!(
        renderer::render(doc, 2).unwrap().data(),
        renderer::render(&reimport, 0).unwrap().data()
    );
}
#[test]
fn range_edits_preserve_nonlinear_poses_on_both_sides_and_mute_is_nondestructive() {
    let mut doc = Document {
        frames: 17,
        ..Default::default()
    };
    doc.add_rectangle([0.; 2], [4.; 2], [255; 4]);
    let obj = &mut doc.objects[0];
    Property::Position.add(obj, 0);
    Property::Position.set(obj, 16, KeyValue::Pair([88., 35.]));
    Property::Position.set_easing(
        obj,
        0,
        Easing::Cubic {
            out: [0.75, 0.2],
            incoming: [1., 0.4],
        },
    );
    Property::Skew.set(obj, 0, KeyValue::Scalar(17.));
    Property::Skew.set(obj, 16, KeyValue::Scalar(52.));
    Property::Skew.set_muted(obj, true);
    assert_eq!(obj.skew_at(8), 0.);
    let original: Vec<_> = (0..17).map(|f| obj.position_at(f)).collect();
    let project = AnimationProject::from_document(doc);
    for (a, b) in [(0, 4), (3, 7), (5, 15), (9, 9), (11, 16)] {
        let mut clip = project.clips[0].clone();
        clip.duplicate_range(a, b).unwrap();
        let expected = original[..=b as usize]
            .iter()
            .chain(&original[a as usize..=b as usize])
            .chain(&original[b as usize + 1..]);
        for (f, v) in expected.enumerate() {
            let out = clip.drawing_at(f as u32).objects[0].position_at(f as u32);
            assert!(
                (out[0] - v[0]).abs() < 0.0001,
                "Duplicate {a}..{b} at {f}: {out:?} vs {v:?}"
            );
        }
        clip.delete_range(b + 1, b + 1 + b - a).unwrap();
        for f in 0..17 {
            let out = clip.drawing_at(f).objects[0].position_at(f);
            assert!((out[0] - original[f as usize][0]).abs() < 0.0001);
        }
        assert!(Property::Skew.is_muted(&clip.drawing_at(0).objects[0]));
    }
}
