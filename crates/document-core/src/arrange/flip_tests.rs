use super::*;
use crate::{PathNode, Shape};

fn apply(doc: &mut Document, ids: &[u64], horizontal: bool) {
    for e in plan_flip(doc, ids, 0, horizontal).unwrap() {
        let o = &mut doc.objects[e.index];
        o.set_position_at(0, e.position);
        o.set_scale_at(0, e.scale);
        o.set_rotation_at(0, e.rotation);
        o.animated.skew = e.skew;
    }
}

fn close(p: [f32; 2], q: [f32; 2]) {
    assert!((p[0] - q[0]).hypot(p[1] - q[1]) < 0.001, "{p:?} != {q:?}");
}

#[test]
fn reflecting_a_group_maps_every_control_about_its_tight_stroked_bounds() {
    let mut src = Document::default();
    let path = Shape::Path {
        nodes: [[-0.4, 0.1], [0.85, -0.3], [0.1, 0.7]]
            .map(PathNode::corner)
            .into(),
        closed: true,
    };
    let id = src.add_shape([5., 8.], [16., 32.], [255; 4], path, 1);
    src.add_rectangle([38., 12.], [17., 8.], [255; 4]);
    for (o, angle) in src.objects.iter_mut().zip([27., 1110.]) {
        o.group = Some(23);
        o.rotation = angle;
        o.animated.skew = 23.;
        o.stroke_width = 2.;
        o.stroke = [255; 4];
        o.scale = [-0.8, 1.25];
    }
    let bounds = src
        .objects
        .iter()
        .map(|o| o.bounds(0))
        .reduce(union)
        .unwrap();
    for (axis, horizontal) in [(0, true), (1, false)] {
        let mut doc = src.clone();
        apply(&mut doc, &[id], horizontal);
        for (old, new) in src.objects.iter().zip(&doc.objects) {
            assert!((new.rotation - old.rotation).abs() <= 90.);
            for local in [[0., 0.], [1., 0.], [0., 1.], [0.2, 1.4]] {
                let mut expected = old.local_to_world(local, 0);
                expected[axis] = bounds[axis] + bounds[2 + axis] - expected[axis];
                close(new.local_to_world(local, 0), expected);
                close(new.world_to_local(expected, 0), local);
                let (_, size) = new.scaled_box_at(0);
                let mut actual =
                    tiny_skia_path::Point::from_xy(local[0] * size[0], local[1] * size[1]);
                new.box_transform(0).map_point(&mut actual);
                close([actual.x, actual.y], expected);
            }
        }
        apply(&mut doc, &[id], horizontal);
        for (old, new) in src.objects.iter().zip(&doc.objects) {
            close(new.position, old.position);
            assert_eq!(
                (new.scale, new.rotation, new.animated.skew),
                (old.scale, old.rotation, old.animated.skew)
            );
        }
        doc.validate().unwrap();
    }
}

#[test]
fn clipped_flip_uses_the_visible_shape_and_the_released_cutter_keeps_its_pose() {
    let mut doc = Document::default();
    let target = doc.add_rectangle([0.; 2], [100.; 2], [255; 4]);
    let cutter = doc.add_shape(
        [10., 20.],
        [30., 15.],
        [255; 4],
        Shape::Path {
            nodes: [[0., 0.], [1., 0.], [0., 1.]].map(PathNode::corner).into(),
            closed: true,
        },
        1,
    );
    doc.set_clip(&[target, cutter], 0, None).unwrap();
    apply(&mut doc, &[target], true);
    assert_eq!(doc.hit_test([37., 22.], 0), Some(target));
    assert_eq!(doc.hit_test([13., 32.], 0), None);
    let released = doc.release_clip(&[target], 0).unwrap();
    let c = doc.objects.iter().find(|o| o.id == cutter).unwrap();
    assert!(released.contains(&c.id));
    for (uv, xy) in [
        ([0., 0.], [40., 20.]),
        ([1., 0.], [10., 20.]),
        ([0., 1.], [40., 35.]),
    ] {
        close(c.local_to_world(uv, 0), xy);
    }
    doc.validate().unwrap();
}

#[test]
fn scale_keys_interpolate_magnitudes_and_switch_orientation_at_the_authored_frame() {
    let mut doc = Document {
        frames: 7,
        ..Default::default()
    };
    doc.add_rectangle([0.; 2], [10.; 2], [255; 4]);
    let o = &mut doc.objects[0];
    o.scale_keys
        .extend([(0, [1., -2.]), (4, [-3., 4.]), (6, [1., 2.])]);
    assert_eq!(o.scale_at(2), [2., -3.]);
    assert_eq!(o.scale_at(4), [-3., 4.]);
    assert_eq!(o.scale_at(5), [-2., 3.]);
    assert_eq!(o.scale_at(6), [1., 2.]);
    doc.validate().unwrap();
}

#[test]
fn flipping_a_rotated_symbol_uses_its_childs_ink_as_the_pivot() {
    let mut source = Document::default();
    let id = source.add_shape(
        [4., 8.],
        [60., 35.],
        [255; 4],
        Shape::Path {
            nodes: [[0., 0.], [1., 0.], [0.12, 1.]]
                .map(PathNode::corner)
                .into(),
            closed: true,
        },
        1,
    );
    let mut project = crate::AnimationProject::from_document(source);
    let (_, placed) = project.convert_to_symbol(0, 0, &[id], "Triangle").unwrap();
    let doc = project.clips[0].drawing_at_mut(0);
    let object = &mut doc.objects[0];
    object.rotation = 39.;
    let ink = |o: &VectorObject| {
        let (art, t) = o.instance_at(0).unwrap();
        art.objects[0]
            .outline(t)
            .unwrap()
            .transform(o.symbol_transform(0).unwrap())
            .unwrap()
    };
    let old = ink(object);
    let actual = old.compute_tight_bounds().unwrap();
    let padded = object.bounds(0);
    assert!((actual.left() + actual.right() - padded[0] - padded[2]).abs() > 1.);
    apply(doc, &[placed], true);
    let after = ink(&doc.objects[0]);
    for (p, q) in old.points().iter().zip(after.points()) {
        close([q.x, q.y], [actual.left() + actual.right() - p.x, p.y]);
    }
}
