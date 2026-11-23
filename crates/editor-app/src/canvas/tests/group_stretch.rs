use super::*;
use crate::timeline::KeyMode;
use document_core::{Property, VectorObject};

fn screen(world: Pos2) -> Pos2 {
    egui::pos2(40., 40.) + world.to_vec2() * 2.5
}

fn grouped() -> CanvasSession {
    let mut s = CanvasSession::new(Tool::Select);
    s.doc.add_rectangle([31., 53.], [13., 9.], [255; 4]);
    s.doc.add_rectangle([62., 47.], [12., 15.], [222; 4]);
    for o in &mut s.doc.objects {
        o.group = Some(9);
    }
    let o = &mut s.doc.objects[1];
    o.rotation_keys.extend([(0, 385.), (4, 395.)]);
    o.animated.skew = -19.;
    s.timeline.frame = 3;
    s.timeline.key_mode = KeyMode::Auto;
    assert!(!s.click(screen(egui::pos2(37., 58.))));
    assert_eq!(s.selected.len(), 2);
    s
}

fn bounds(s: &CanvasSession) -> Rect {
    group_bounds(
        &s.doc,
        s.timeline.frame,
        &GestureIds::new(s.selected.clone()),
    )
    .unwrap()
}

fn assert_stretched(world: [f32; 2], actual: [f32; 2], fixed: Pos2, factors: Vec2) {
    let expected = fixed + (Pos2::from(world) - fixed) * factors;
    assert!(
        expected.distance(actual.into()) < 0.001,
        "expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn group_stretch_corner_and_edges_follow_both_pointer_axes_and_preserve_tracks() {
    for index in 0..8 {
        let mut s = grouped();
        let start = s.doc.clone();
        let (handle, fixed) = gesture::group_resize_handles(bounds(&s), 2.5)
            .nth(index)
            .unwrap();
        let span = handle - fixed;
        let scale = egui::vec2(
            if span.x == 0. { 1. } else { 1.45 },
            if span.y == 0. { 1. } else { 0.67 },
        );
        // A diagonal drag on a side handle ignores motion along the fixed axis.
        let stray = egui::vec2(
            if span.x == 0. { 5. } else { 0. },
            if span.y == 0. { 3. } else { 0. },
        );
        assert!(s.drag(screen(handle), screen(fixed + span * scale + stray)));
        s.doc.validate().unwrap();
        for (old, new) in start.objects.iter().zip(&s.doc.objects) {
            for local in [[0.; 2], [1., 0.], [0.2, 0.6], [1.; 2]] {
                assert_stretched(
                    old.local_to_world(local, 3),
                    new.local_to_world(local, 3),
                    fixed,
                    scale,
                );
                assert_eq!(old.local_to_world(local, 0), new.local_to_world(local, 0));
            }
            assert_eq!(new.shape, old.shape);
            assert_eq!(new.group, old.group);
            assert!(new.scale_keys.contains_key(&3));
        }
        assert!(s.doc.objects[0].rotation_keys.is_empty());
        assert!(s.doc.objects[0].animated.skew_keys.is_empty());
        assert!(s.doc.objects[1].rotation_keys[&3] > 360.);
        assert!(s.doc.objects[1].animated.skew_keys.contains_key(&3));
    }
}

#[test]
fn group_stretch_shift_at_press_constrains_proportions_and_manual_keys_veto_atomically() {
    let mut s = grouped();
    s.timeline.key_mode = KeyMode::Manual;
    s.doc.objects[1].animated.skew_keys.insert(0, -19.);
    let original = s.doc.clone();
    let bounds = bounds(&s);
    let target = bounds.max + egui::vec2(5., 8.);
    assert!(!s.drag(screen(bounds.max), screen(target)));
    assert_eq!(s.doc, original);
    s.modifiers = egui::Modifiers::SHIFT;
    assert!(s.drag(screen(bounds.max), screen(target)));
    assert_eq!(s.selected.len(), 2);
    let factor = (target - bounds.min).dot(bounds.size()) / bounds.size().length_sq();
    for (a, b) in original.objects.iter().zip(&s.doc.objects) {
        assert_eq!(a.animated.skew_keys, b.animated.skew_keys);
        assert_eq!(a.rotation_keys, b.rotation_keys);
        assert_stretched(
            a.local_to_world([0.7, 0.1], 3),
            b.local_to_world([0.7, 0.1], 3),
            bounds.min,
            Vec2::splat(factor),
        );
    }
}

#[test]
fn group_stretch_keeps_all_members_together_at_the_skew_limit() {
    let mut s = grouped();
    let original = s.doc.clone();
    let bounds = bounds(&s);
    let target = bounds.left_center() + egui::vec2(bounds.width() * 0.001, 0.);
    assert!(s.drag(screen(bounds.right_center()), screen(target)));
    s.doc.validate().unwrap();
    let scale = Vec2::from(s.doc.objects[0].scale_at(3));
    assert!(scale.x > 0.001 && scale.x < 0.1);
    assert_eq!(scale.y, 1.);
    assert!((s.doc.objects[1].skew_at(3).abs() - 85.).abs() < 0.001);
    for (a, b) in original.objects.iter().zip(&s.doc.objects) {
        assert_stretched(
            a.local_to_world([0.2, 0.6], 3),
            b.local_to_world([0.2, 0.6], 3),
            bounds.left_center(),
            scale,
        );
    }
}

fn posed_controls(object: &VectorObject, t: u32) -> Vec<[f32; 2]> {
    object
        .skinned_shape_at(t)
        .contours()
        .iter()
        .flatten()
        .flat_map(|n| [n.point, n.incoming, n.outgoing])
        .map(object.local_to_world_at(t))
        .collect()
}

#[test]
fn selection_flip_reflects_a_custom_hinge_with_its_rotated_skewed_artwork() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([24., 27.], [38., 20.], [255; 4]);
    let o = &mut doc.objects[0];
    o.scale = [-1.1, 1.3];
    o.rotation = 28.;
    o.animated.skew = 15.;
    let pivot = [-0.25, 1.25];
    assert!(o.set_pivot_at(0, o.local_to_world(pivot, 0)));
    let before = &doc.objects[0];
    let center = before.local_to_world([0.5; 2], 0);
    for horizontal in [true, false] {
        let mut flipped = doc.clone();
        assert!(flip_selection(
            &mut flipped,
            &[id],
            0,
            horizontal,
            KeyMode::Tracked
        ));
        let after = &flipped.objects[0];
        let axis = usize::from(!horizontal);
        for point in [[0., 0.], [0., 1.], [1., 0.], pivot] {
            let mut reflected = before.local_to_world(point, 0);
            reflected[axis] = 2. * center[axis] - reflected[axis];
            assert!(Pos2::from(reflected).distance(after.local_to_world(point, 0).into()) < 1e-3);
        }
        let mut locked = doc.clone();
        Property::Position.set_muted(&mut locked.objects[0], true);
        let authored = locked.clone();
        assert!(!flip_selection(
            &mut locked,
            &[id],
            0,
            horizontal,
            KeyMode::Auto
        ));
        assert_eq!(locked, authored);
    }
    let mut on_axis = Document::default();
    let id = on_axis.add_rectangle([20.; 2], [30.; 2], [255; 4]);
    let o = &mut on_axis.objects[0];
    assert!(o.set_pivot_at(0, o.local_to_world([0.5, 0.], 0)));
    Property::Position.set_muted(o, true);
    assert!(flip_selection(&mut on_axis, &[id], 0, true, KeyMode::Auto));
    assert!(on_axis.objects[0].keys.is_empty());
}

#[test]
fn flip_preserves_the_weighted_pose_and_preflights_all_members_before_keying() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-elbow.vanim");
    let project = project_io::load_animation(&path).unwrap();
    let mut doc = project.clips[0].drawing_at(4).clone();
    let index = doc.objects.iter().position(|o| o.skin.is_some()).unwrap();
    let id = doc.objects[index].id;
    let second = doc.add_rectangle([33., 33.], [5., 5.], [255; 4]);
    let selected = [id, second];
    Property::Geometry.set_muted(&mut doc.objects[index], true);
    let locked = doc.clone();
    assert!(!flip_selection(&mut doc, &selected, 4, true, KeyMode::Auto));
    assert_eq!(doc, locked);
    Property::Geometry.set_muted(&mut doc.objects[index], false);
    let before = doc.objects[index].clone();
    let a = before.bounds(4);
    let b = doc.objects.last().unwrap().bounds(4);
    let twice_center = a[0].min(b[0]) + a[2].max(b[2]);
    assert!(flip_selection(&mut doc, &selected, 4, true, KeyMode::Auto));
    let after = &doc.objects[index];
    assert_eq!(before.skin, after.skin);
    assert_eq!(posed_controls(&before, 0), posed_controls(after, 0));
    for (a, b) in posed_controls(&before, 4)
        .into_iter()
        .zip(posed_controls(after, 4))
    {
        assert!((twice_center - a[0] - b[0]).hypot(a[1] - b[1]) < 0.001);
    }
    doc.validate().unwrap();
}

#[test]
fn group_stretch_preserves_weighted_pose_and_geometry_key_rules() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-elbow.vanim");
    let project = project_io::load_animation(&path).unwrap();
    let mut s = CanvasSession::new(Tool::Select);
    s.doc = project.clips[0].drawing_at(4).clone();
    let sleeve = s.doc.objects.iter().position(|o| o.skin.is_some()).unwrap();
    let id = s.doc.add_rectangle([33., 33.], [5., 5.], [255; 4]);
    s.selected = vec![s.doc.objects[sleeve].id, id];
    s.timeline.frame = 4;
    s.timeline.key_mode = KeyMode::Auto;
    Property::Geometry.set_muted(&mut s.doc.objects[sleeve], true);
    let bounds = bounds(&s);
    let fixed = bounds.min;
    let scale = egui::vec2(1.3, 0.8);
    let drag =
        |s: &mut CanvasSession| s.drag(screen(bounds.max), screen(fixed + bounds.size() * scale));
    let muted = s.doc.clone();
    assert!(!drag(&mut s));
    assert_eq!(s.doc, muted);
    Property::Geometry.set_muted(&mut s.doc.objects[sleeve], false);
    let old = s.doc.objects[sleeve].clone();
    let skeleton = s.doc.skeleton.clone();
    assert!(drag(&mut s));
    s.doc.validate().unwrap();
    let stretched = &s.doc.objects[sleeve];
    assert_eq!(s.doc.skeleton, skeleton);
    assert_eq!(stretched.skin, old.skin);
    assert_eq!(stretched.shape, old.shape);
    assert!(stretched.animated.geometry_keys.contains_key(&4));
    assert_eq!(posed_controls(&old, 0), posed_controls(stretched, 0));
    for (a, b) in posed_controls(&old, 4)
        .into_iter()
        .zip(posed_controls(stretched, 4))
    {
        assert_stretched(a, b, fixed, scale);
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stretched-sleeve.vanim");
    let scene = document_core::AnimationProject::from_document(s.doc.clone());
    project_io::save_animation(&path, &scene, false).unwrap();
    let reloaded = project_io::load_animation(&path).unwrap();
    assert_eq!(reloaded, scene);
    assert_eq!(
        posed_controls(&reloaded.clips[0].drawing_at(4).objects[sleeve], 4),
        posed_controls(stretched, 4)
    );
}
