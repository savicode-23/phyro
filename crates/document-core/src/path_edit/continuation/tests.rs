use super::*;
use crate::{Document, Gradient, PathEffect};

fn path() -> VectorObject {
    let mut doc = Document::default();
    let mut nodes = [[0., 0.], [0.45, 0.4], [1., 0.2]].map(PathNode::corner);
    nodes[0].outgoing = [0.1, 0.3];
    nodes[1].incoming = [0.3, 0.8];
    nodes[2].incoming = [0.75, -0.2];
    doc.add_shape(
        [103., 17.],
        [92., 60.],
        [85, 20, 72, 255],
        Shape::Path {
            nodes: nodes.to_vec(),
            closed: false,
        },
        1,
    );
    doc.objects.pop().unwrap()
}

fn same_point(a: [f32; 2], b: [f32; 2]) {
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 0.001, "{a:?} vs {b:?}");
}

#[test]
fn either_tip_extends_in_world_space_preserving_existing_pose_and_direction() {
    let mut base = path();
    base.rotation = 28.;
    base.animated.skew = -23.;
    base.scale = [-1.7, 0.8];
    base.pivot = Some([0.22, 0.67]);
    base.keys.insert(4, [-11., 24.]);
    base.stroke_width = 7.;
    base.name = "Arm outline".into();
    base.gradient = Some(Gradient::from_color(
        crate::GradientKind::Linear {
            start: [0., 0.],
            end: [1., 1.],
        },
        [12; 4],
    ));
    let frame = 4;
    let ends = base.pen_endpoints_at(frame).unwrap();
    let mut a = PathNode::corner([46., 156.]);
    a.incoming = [50., 140.];
    a.outgoing = [44., 165.];
    let b = PathNode::corner([12., 180.]);
    for (from, join, before) in [
        (PathEnd::Start, ends[0].incoming, true),
        (PathEnd::End, ends[1].outgoing, false),
    ] {
        let mut result = base.clone();
        assert!(result.extend_open_path(frame, from, join, &[a, b], None));
        let nodes = &result.shape.contours()[0];
        let original = &base.shape.contours()[0];
        let offset = if before { 2 } else { 0 };
        assert_eq!(&nodes[offset..offset + original.len()], original);
        let n = nodes[if before { 1 } else { original.len() }];
        same_point(result.local_to_world(n.point, frame), a.point);
        same_point(
            result.local_to_world(n.incoming, frame),
            if before { a.outgoing } else { a.incoming },
        );
        same_point(
            result.local_to_world(n.outgoing, frame),
            if before { a.incoming } else { a.outgoing },
        );
        let n = nodes[if before { 0 } else { original.len() + 1 }];
        same_point(result.local_to_world(n.point, frame), b.point);
        let mut expected = base.clone();
        expected.shape = result.shape.clone();
        assert_eq!(result, expected);
    }
}

#[test]
fn direct_closure_changes_only_the_two_previously_unused_controls() {
    for from in [PathEnd::Start, PathEnd::End] {
        let mut o = path();
        let original = o.shape.clone();
        let a = [55., 60.];
        let b = [140., 115.];
        assert!(o.extend_open_path(0, from, a, &[], Some(b)));
        let Shape::Path {
            nodes,
            closed: true,
        } = &o.shape
        else {
            panic!()
        };
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[1], original.contours()[0][1]);
        assert_eq!(nodes[0].outgoing, original.contours()[0][0].outgoing);
        assert_eq!(nodes[2].incoming, original.contours()[0][2].incoming);
        let (first, last) = if from == PathEnd::Start {
            (a, b)
        } else {
            (b, a)
        };
        same_point(o.local_to_world(nodes[0].incoming, 0), first);
        same_point(o.local_to_world(nodes[2].outgoing, 0), last);
        assert!(o.pen_endpoints_at(0).is_none());
    }
}

#[test]
fn pressure_stops_stay_on_the_same_old_arc_when_extending_and_closing() {
    let mut o = path();
    o.position = [0.; 2];
    o.size = [100.; 2];
    o.shape = Shape::Path {
        nodes: [[0., 0.], [1., 0.]].map(PathNode::corner).to_vec(),
        closed: false,
    };
    o.stroke_profile = vec![[0., 0.2], [0.25, 0.9], [1., 0.4]];
    let mut append = o.clone();
    assert!(append.extend_open_path(
        0,
        PathEnd::End,
        [100., 0.],
        &[PathNode::corner([200., 0.])],
        None
    ));
    assert_eq!(
        append.stroke_profile,
        [[0., 0.2], [0.125, 0.9], [0.5, 0.4], [1., 0.4]]
    );
    let mut prepend = o.clone();
    assert!(prepend.extend_open_path(
        0,
        PathEnd::Start,
        [0., 0.],
        &[PathNode::corner([-100., 0.])],
        None
    ));
    assert_eq!(
        prepend.stroke_profile,
        [[0., 0.2], [0.5, 0.2], [0.625, 0.9], [1., 0.4]]
    );
    assert!(prepend.extend_open_path(0, PathEnd::End, [100., 0.], &[], Some([-100., 0.])));
    assert_eq!(
        prepend.stroke_profile,
        [[0., 0.2], [0.25, 0.2], [0.3125, 0.9], [0.5, 0.4], [1., 0.2]]
    );
    let mut full = o;
    full.stroke_profile = (0..crate::MAX_STROKE_SAMPLES)
        .map(|i| {
            [
                i as f32 / (crate::MAX_STROKE_SAMPLES - 1) as f32,
                (i % 2) as f32,
            ]
        })
        .collect();
    assert!(full.extend_open_path(
        0,
        PathEnd::End,
        [100., 0.],
        &[PathNode::corner([200., 0.])],
        None
    ));
    assert_eq!(full.stroke_profile.len(), crate::MAX_STROKE_SAMPLES);
    assert!(crate::pressure::valid_profile(&full.stroke_profile));
}

#[test]
fn rejected_continuations_leave_artwork_and_pressure_untouched() {
    let mut o = path();
    let before = o.clone();
    let extra = PathNode::corner([28., 100.]);
    assert!(!o.extend_open_path(0, PathEnd::End, [22.; 2], &[], None));
    assert!(!o.extend_open_path(0, PathEnd::End, [f32::NAN; 2], &[extra], None));
    assert!(!o.extend_open_path(
        0,
        PathEnd::Start,
        [2.; 2],
        &[PathNode::corner([f32::INFINITY; 2])],
        None
    ));
    assert!(!o.extend_open_path(0, PathEnd::End, [2.; 2], &[extra], Some([f32::NAN; 2])));
    assert_eq!(o, before);
    o.animated.geometry_keys.insert(2, o.shape.clone());
    assert!(o.pen_endpoints_at(2).is_none());
    let keyed = o.clone();
    assert!(!o.extend_open_path(2, PathEnd::Start, [2.; 2], &[extra], None));
    assert_eq!(o, keyed);
    o.animated.geometry_keys.clear();
    o.effects.push(PathEffect::RoundCorners { distance: 4. });
    assert!(o.pen_endpoints_at(0).is_none());
}
