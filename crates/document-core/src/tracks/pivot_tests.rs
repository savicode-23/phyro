use super::*;
use crate::{Document, MotionPath, PathNode, SymbolDefinition};

fn close(a: [f32; 2], b: [f32; 2]) {
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 0.001, "{a:?} != {b:?}");
}

fn corners(o: &VectorObject, t: u32) -> [[f32; 2]; 4] {
    [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(o.local_to_world_at(t))
}

#[test]
fn repositioning_an_animated_hinge_keeps_this_pose_and_drives_the_whole_tween() {
    let mut doc = Document {
        frames: 9,
        ..Document::default()
    };
    let id = doc.add_rectangle([14., 23.], [40., 19.], [255; 4]);
    let o = &mut doc.objects[0];
    o.keys.extend([(0, [12., 24.]), (8, [44., 40.])]);
    o.rotation_keys.extend([(0, -16.), (8, 152.)]);
    o.scale_keys.extend([(0, [-0.75, 1.]), (8, [2., 0.4])]);
    o.animated.skew_keys.extend([(0, 30.), (8, -15.)]);
    let playhead = 3;
    let before = corners(o, playhead);
    let local = [-0.35, 0.73];
    let world = o.local_to_world(local, playhead);
    assert!(o.set_pivot_at(playhead, world));
    close(o.pivot.unwrap(), local);
    assert_eq!(o.keys.len(), 2);
    for (a, b) in before.into_iter().zip(corners(o, playhead)) {
        close(a, b);
    }
    for t in 0..=8 {
        let delta = t as f32 - playhead as f32;
        let expected = [world[0] + 4. * delta, world[1] + 2. * delta];
        close(o.pivot_at(t), expected);
        close(o.local_to_world(local, t), expected);
    }
    let hinges: Vec<_> = (0..=8).map(|t| o.pivot_at(t)).collect();
    assert!(o.set_base_size([18., 62.]));
    for (t, world) in hinges.into_iter().enumerate() {
        close(o.pivot_at(t as u32), world);
    }
    o.set_position_at(playhead, [-19., 56.]);
    close(o.position_at(playhead), [-19., 56.]);

    let o = &doc.objects[0];
    let posed = corners(o, playhead);
    let (definition, origin) = SymbolDefinition::capture("Hinge", &doc, playhead, &[id]).unwrap();
    let baked = &definition.artwork.objects[0];
    assert_eq!(baked.pivot, o.pivot);
    for (a, b) in corners(baked, 0).into_iter().zip(posed) {
        close([a[0] + origin[0], a[1] + origin[1]], b);
    }
    let o = &mut doc.objects[0];
    assert!(o.reset_pivot_at(playhead));
    assert!(o.pivot.is_none());
    assert!(!o.reset_pivot_at(playhead));
    for (a, b) in posed.into_iter().zip(corners(o, playhead)) {
        close(a, b);
    }
    doc.validate().unwrap();
}

#[test]
fn pivot_follows_motion_and_rig_while_local_rotation_and_scale_keep_it_fixed() {
    let mut doc = Document {
        frames: 9,
        ..Document::default()
    };
    let bone = doc.add_bone("Arm", None, [4., 2.], [28., 2.]).unwrap();
    let id = doc.add_rectangle([10., 20.], [25., 12.], [255; 4]);
    let o = &mut doc.objects[0];
    let mut guide = MotionPath::new(
        [[0., 0.], [12., 0.], [12., 28.]]
            .map(PathNode::corner)
            .to_vec(),
        false,
    )
    .unwrap();
    guide.align_tangent = true;
    o.motion = Some(guide);
    o.animated.progress_keys.extend([(0, 0.), (8, 1.)]);
    o.rotation_keys.extend([(0, -30.), (8, 90.)]);
    o.scale = [-2., 0.6];
    o.animated.skew = 32.;
    doc.bind_rigid_at(id, Some(bone), 3).unwrap();
    doc.set_bone_rotation(bone, 72.).unwrap();
    let o = &mut doc.objects[0];
    let before = corners(o, 3);
    let local = [1.3, -0.2];
    assert!(o.set_pivot_at(3, o.local_to_world(local, 3)));
    for (a, b) in before.into_iter().zip(corners(o, 3)) {
        close(a, b);
    }
    for t in 0..=8 {
        let offset = o.motion_offset_at(t);
        let rest = std::array::from_fn(|i| o.position[i] + offset[i] + local[i] * o.size[i]);
        close(o.local_to_world(local, t), o.rig_point_at(rest, t));
    }
    let before = corners(o, 5);
    doc.bind_rigid_at(id, None, 5).unwrap();
    for (a, b) in before.into_iter().zip(corners(&doc.objects[0], 5)) {
        close(a, b);
    }
}

#[test]
fn invalid_pivot_edits_are_atomic_and_center_is_the_legacy_default() {
    let mut doc = Document::default();
    doc.add_rectangle([3., 2.], [50., 26.], [255; 4]);
    let o = &mut doc.objects[0];
    let old = o.clone();
    assert_eq!(o.pivot_at(0), [28., 15.]);
    assert!(!o.set_pivot_at(0, [28., 15.]));
    assert!(!o.set_pivot_at(0, [f32::INFINITY, 8.]));
    assert_eq!(*o, old);
    assert!(o.set_base_size([100., 20.]));
    assert_eq!(o.position, old.position);
    o.pivot = Some([f32::NAN, 0.5]);
    assert!(doc.validate().is_err());
}
