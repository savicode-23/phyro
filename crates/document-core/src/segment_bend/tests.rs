use super::*;
use crate::Document;

fn object(shape: Shape) -> VectorObject {
    let mut doc = Document::default();
    doc.add_shape([33., -12.], [120., 45.], [255; 4], shape, 1);
    doc.objects.pop().unwrap()
}

fn line() -> Shape {
    Shape::Path {
        nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([1., 0.])],
        closed: false,
    }
}

fn point(
    o: &VectorObject,
    source: &Shape,
    frame: u32,
    contour: usize,
    segment: usize,
    t: f32,
) -> [f32; 2] {
    let shape = o.skin_shape_at(Cow::Borrowed(source), frame);
    let nodes = &shape.contours()[contour];
    let a = nodes[segment];
    let b = nodes[(segment + 1) % nodes.len()];
    let u = 1. - t;
    let p = std::array::from_fn(|i| {
        u.powi(3) * a.point[i]
            + 3. * u * u * t * a.outgoing[i]
            + 3. * u * t * t * b.incoming[i]
            + t.powi(3) * b.point[i]
    });
    o.local_to_world(p, frame)
}

fn assert_drag(old: [f32; 2], new: [f32; 2], delta: [f32; 2]) {
    for axis in 0..2 {
        assert!(
            (new[axis] - old[axis] - delta[axis]).abs() < 0.001,
            "{old:?} -> {new:?}, delta {delta:?}"
        );
    }
}

#[test]
fn straight_edges_gain_tangent_handles_and_keep_the_point_beneath_the_pointer() {
    let mut o = object(line());
    o.rotation = 37.;
    o.scale = [-1.4, 0.72];
    o.animated.skew = 24.;
    let delta = [5.3, -9.1];
    for t in [0.065, 0.25, 0.5, 0.75, 0.935] {
        let shape = o.bent_segment(0, 0, 0, t, delta).unwrap();
        let s = t * t * (3. - 2. * t);
        assert_drag(
            point(&o, &o.shape, 0, 0, 0, t),
            point(&o, &shape, 0, 0, 0, s),
            delta,
        );
        let nodes = &shape.contours()[0];
        assert_eq!(nodes[0].point, [0., 0.]);
        assert_eq!(nodes[1].point, [1., 0.]);
        assert_eq!(nodes[0].incoming, [0., 0.]);
        assert_eq!(nodes[1].outgoing, [1., 0.]);
        if s < 1. / 6. {
            assert_eq!(nodes[1].incoming, [2. / 3., 0.]);
        }
        if s > 5. / 6. {
            assert_eq!(nodes[0].outgoing, [1. / 3., 0.]);
        }
    }
}

#[test]
fn bending_a_closed_seam_preserves_other_contours_and_adjacent_segments() {
    let mut nodes = [[0., 0.], [0.5, 1.], [1., 0.]].map(PathNode::corner);
    nodes[0].incoming = [0.2, -0.2];
    nodes[0].outgoing = [-0.2, 0.2];
    nodes[0].handle_style = HandleStyle::Symmetric;
    nodes[2].incoming = [1.1, 0.1];
    nodes[2].outgoing = [0.8, -0.2];
    nodes[2].handle_style = HandleStyle::Smooth;
    let o = object(Shape::Compound {
        contours: vec![nodes.to_vec(), nodes.to_vec()],
    });
    for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
        let delta = [2., 5.];
        let bent = o.bent_segment(0, 1, 2, t, delta).unwrap();
        let after = &bent.contours()[1];
        assert_eq!(bent.contours()[0], nodes);
        assert_eq!(after[1], nodes[1]);
        assert_eq!(after[0].outgoing, nodes[0].outgoing);
        assert_eq!(after[2].incoming, nodes[2].incoming);
        for i in [0, 2] {
            assert_eq!(after[i].point, nodes[i].point);
        }
        if t < 1. / 6. {
            assert_eq!(after[0], nodes[0]);
            assert_eq!(after[2].handle_style, HandleStyle::Corner);
        }
        if t > 5. / 6. {
            assert_eq!(after[2], nodes[2]);
            assert_eq!(after[0].handle_style, HandleStyle::Corner);
        }
        assert_drag(
            point(&o, &o.shape, 0, 1, 2, t),
            point(&o, &bent, 0, 1, 2, t),
            delta,
        );
    }
}

#[test]
fn bends_the_evaluated_geometry_and_inverts_each_ends_skin_pose() {
    let mut doc = Document {
        frames: 6,
        ..Default::default()
    };
    let arm = doc.add_bone("Arm", None, [0., 0.], [50., 0.]).unwrap();
    let wrist = doc
        .add_bone("Wrist", Some(arm), [50., 0.], [100., 0.])
        .unwrap();
    let shape = line();
    let id = doc.add_shape([0., 0.], [100., 50.], [255; 4], shape.clone(), 1);
    let mut end = shape.clone();
    end.contours_mut()[0][0].outgoing = [0.1, 0.9];
    let keys = &mut doc.objects[0].animated.geometry_keys;
    keys.insert(0, shape.clone());
    keys.insert(4, end);
    doc.bind_skin_at(id, wrist, 0).unwrap();
    doc.set_skin_weight(id, 0, 0, 0.).unwrap();
    doc.set_skin_weight(id, 0, 1, 1.).unwrap();
    doc.set_bone_rotation_key(wrist, 0, 0.).unwrap();
    doc.set_bone_rotation_key(wrist, 4, 90.).unwrap();
    let o = &doc.objects[0];
    let delta = [-4., 7.];
    let t = 0.34;
    let bent = o.bent_segment(2, 0, 0, t, delta).unwrap();
    assert_drag(
        point(o, &o.shape_at(2), 2, 0, 0, t),
        point(o, &bent, 2, 0, 0, t),
        delta,
    );
    assert_eq!(o.shape, shape);
    assert_eq!(o.animated.geometry_keys.len(), 2);
    doc.set_skin_weight(id, 0, 1, 0.5).unwrap();
    doc.set_bone_rotation_key(wrist, 4, 180.).unwrap();
    assert!(doc.objects[0].bent_segment(4, 0, 0, t, delta).is_none());
}

#[test]
fn rejects_endpoints_zero_motion_missing_segments_and_nonfinite_geometry() {
    let o = object(line());
    for t in [0., 1., -0.1, f32::NAN] {
        assert!(o.bent_segment(0, 0, 0, t, [2.; 2]).is_none());
    }
    for delta in [[0.; 2], [f32::NAN, 0.], [f32::INFINITY, 0.]] {
        assert!(o.bent_segment(0, 0, 0, 0.5, delta).is_none());
    }
    assert!(o.bent_segment(0, 0, 1, 0.5, [2.; 2]).is_none());
    assert!(o.bent_segment(0, 1, 0, 0.5, [2.; 2]).is_none());
    let mut collapsed = o.clone();
    collapsed.shape.contours_mut()[0][1] = PathNode::corner([0.; 2]);
    assert!(collapsed.bent_segment(0, 0, 0, 0.5, [2.; 2]).is_none());
}
