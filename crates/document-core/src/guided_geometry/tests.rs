use super::*;
use crate::{AnimationProject, Document, PathNode};

fn pinned() -> (Document, MorphGuide) {
    let mut doc = Document {
        frames: 80,
        ..Default::default()
    };
    let path = |points: &[[f32; 2]]| Shape::Path {
        closed: false,
        nodes: points.iter().copied().map(PathNode::corner).collect(),
    };
    let left = path(&[[0., 0.], [0.3, 0.8], [1., 1.]]);
    let right = path(&[[0., 1.], [0.45, 0.85], [0.4, 0.2], [1., 0.]]);
    let guide = MorphGuide {
        from: MorphPoint {
            segment: 1,
            t: 0.35,
        },
        to: MorphPoint { segment: 2, t: 0.2 },
    };
    doc.add_shape([0.; 2], [10.; 2], [255; 4], left.clone(), 1);
    doc.objects[0]
        .set_guided_morph(0, 60, left, right, vec![guide])
        .unwrap();
    (doc, guide)
}

fn same_geometry(a: &Shape, b: &Shape) {
    let (a, b) = (&a.contours()[0], &b.contours()[0]);
    assert_eq!(a.len(), b.len(), "unexpected subdivision");
    for (a, b) in a.iter().zip(b) {
        for (p, q) in [
            (a.point, b.point),
            (a.incoming, b.incoming),
            (a.outgoing, b.outgoing),
        ] {
            for (x, y) in p.into_iter().zip(q) {
                assert!((x - y).abs() < 2e-5, "{x} vs {y}");
            }
        }
    }
}

#[test]
fn inserting_retiming_and_deleting_a_breakdown_keeps_matching_points() {
    let (mut doc, guide) = pinned();
    let o = &mut doc.objects[0];
    let original = o.clone();
    assert!(Property::Geometry.add(o, 30));
    let left = o.morph_guides(0, 30).unwrap()[0];
    let right = o.morph_guides(30, 60).unwrap()[0];
    assert_eq!(
        (left.from, left.to, right.to),
        (guide.from, right.from, guide.to)
    );
    for t in [4, 12, 30, 48, 55] {
        same_geometry(&o.shape_at(t), &original.shape_at(t));
    }
    o.move_guided_keys(&BTreeMap::from([(0, 8), (30, 38), (60, 68)]))
        .unwrap();
    assert_eq!(o.morph_guides(8, 38), Some([left].as_slice()));
    assert!(Property::Geometry.remove(o, 38));
    assert_eq!(o.morph_guides(8, 68), Some([guide].as_slice()));
    same_geometry(&o.shape_at(50), &original.shape_at(42));
    assert!(Property::Geometry.remove(o, 8));
    doc.validate().unwrap();
    assert!(Property::Geometry.remove(&mut doc.objects[0], 68));
    assert!(doc.objects[0].animated.guided_geometry.is_none());
}

#[test]
fn range_edits_preserve_eased_gesture_and_sparse_keys() {
    let (mut doc, _) = pinned();
    let original = &mut doc.objects[0];
    Property::Geometry.set_easing(original, 0, crate::Easing::EaseOut);
    let first = original.clone();
    Property::Geometry.set_muted(original, true);
    let mut project = AnimationProject::from_document(doc);
    let clip = &mut project.clips[0];
    clip.duplicate_range(12, 24).unwrap();
    for drawing in clip.cels.values_mut() {
        let object = &mut Arc::make_mut(drawing).objects[0];
        assert!(Property::Geometry.is_muted(object));
        Property::Geometry.set_muted(object, false);
    }
    for (t, original) in (0..=24).chain(12..=24).chain(25..80).enumerate() {
        let t = t as u32;
        same_geometry(
            &clip.drawing_at(t).objects[0].shape_at(t),
            &first.shape_at(original),
        );
    }
    assert!(clip.drawing_at(12).objects[0].animated.geometry_keys.len() < 10);
    clip.delete_range(25, 37).unwrap();
    for t in 0..80 {
        same_geometry(
            &clip.drawing_at(t).objects[0].shape_at(t),
            &first.shape_at(t),
        );
    }
    project.validate().unwrap();
}

#[test]
fn adding_a_breakdown_keeps_nonlinear_timing_and_rejects_invalid_edits_atomically() {
    let (mut doc, guide) = pinned();
    let o = &mut doc.objects[0];
    Property::Geometry.set_easing(o, 0, crate::Easing::EaseInOut);
    let before = o.clone();
    assert!(Property::Geometry.add(o, 17));
    for t in 0..60 {
        same_geometry(&before.shape_at(t), &o.shape_at(t));
    }
    let with_breakdown = o.clone();
    let invalid = MorphGuide {
        from: MorphPoint {
            segment: usize::MAX,
            t: 0.,
        },
        ..guide
    };
    assert!(o.set_morph_guides(0, 17, vec![invalid]).is_err());
    assert_eq!(o, &with_breakdown);
    let mut changed = o.animated.geometry_keys[&17].clone();
    changed.contours_mut()[0].pop();
    assert!(o.set_shape_at(17, changed).is_err());
    assert_eq!(o, &with_breakdown);
    o.stroke_profile = vec![[0., 1.], [1., 0.5]];
    assert!(doc.validate().is_err());
}

#[test]
fn two_closed_breakdowns_keep_the_authored_seam_and_landmark() {
    let (mut doc, guide) = pinned();
    let o = &mut doc.objects[0];
    let mut first = o.animated.geometry_keys[&0].clone();
    let mut end = o.animated.geometry_keys[&60].clone();
    for shape in [&mut first, &mut end] {
        let Shape::Path { closed, .. } = shape else {
            unreachable!()
        };
        *closed = true;
    }
    end.contours_mut()[0].reverse();
    o.animated = Default::default();
    o.shape = first.clone();
    o.set_guided_morph(0, 60, first, end, vec![guide]).unwrap();
    let original = o.clone();
    assert!(Property::Geometry.add(o, 35));
    assert!(Property::Geometry.add(o, 14));
    let pairs = o.morph_guides(14, 35).unwrap().to_vec();
    o.set_morph_guides(14, 35, pairs).unwrap();
    for t in 0..60 {
        same_geometry(&o.shape_at(t), &original.shape_at(t));
    }
}

#[test]
fn cache_is_shared_by_undo_snapshots_and_rechecks_directly_edited_endpoints() {
    let (doc, _) = pinned();
    let mut copy = doc.objects[0].clone();
    let cached = |o: &VectorObject| {
        o.guided_plan(
            0,
            60,
            &o.animated.geometry_keys[&0],
            &o.animated.geometry_keys[&60],
        )
        .unwrap()
    };
    let initial = cached(&doc.objects[0]);
    assert!(Arc::ptr_eq(&initial, &cached(&copy)));
    copy.animated
        .geometry_keys
        .get_mut(&0)
        .unwrap()
        .contours_mut()[0][0]
        .point[0] = -0.1;
    assert!(!Arc::ptr_eq(&initial, &cached(&copy)));
    same_geometry(&doc.objects[0].shape_at(30), &initial.sample(0.5));
}
