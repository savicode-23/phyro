use super::*;
use egui::{vec2, Modifiers};

fn screen(p: Pos2) -> Pos2 {
    egui::pos2(40., 40.) + p.to_vec2() * 2.5
}

fn selection(group: bool) -> CanvasSession {
    let mut s = CanvasSession::new(Tool::Select);
    if group {
        s.doc.add_rectangle([32., 40.], [16., 24.], [255; 4]);
        s.doc.add_rectangle([56., 40.], [16., 24.], [150; 4]);
        for object in &mut s.doc.objects {
            object.group = Some(1);
        }
    } else {
        s.doc.add_rectangle([32., 40.], [40., 24.], [255; 4]);
    }
    s.click(screen(egui::pos2(40., 52.)));
    assert_eq!(s.selected.len(), s.doc.objects.len());
    s
}

fn bounds(doc: &Document, ids: &[u64]) -> Rect {
    group_bounds(doc, 0, &GestureIds::new(ids.to_vec())).unwrap()
}

#[test]
fn ctrl_and_shift_ctrl_resize_around_the_center_for_single_and_group_handles() {
    for group in [false, true] {
        for side in [false, true] {
            for uniform in [false, true] {
                let mut s = selection(group);
                let original = bounds(&s.doc, &s.selected);
                let handle = if side {
                    original.right_center()
                } else {
                    original.max
                };
                s.modifiers = Modifiers::CTRL
                    | if uniform {
                        Modifiers::SHIFT
                    } else {
                        Modifiers::NONE
                    };
                let target = handle + egui::vec2(12., 2.);
                assert!(s.drag(screen(handle), screen(target)));
                let resized = bounds(&s.doc, &s.selected);
                assert!(resized.center().distance(original.center()) < 1e-3);
                let scale = resized.size() / original.size();
                if uniform {
                    assert!(scale.x > 1. && (scale.x - scale.y).abs() < 1e-4);
                } else {
                    assert!((resized.right() - target.x).abs() < 1e-3);
                    let bottom = if side { original.bottom() } else { target.y };
                    assert!((resized.bottom() - bottom).abs() < 1e-3);
                }
                s.doc.validate().unwrap();
            }
        }
    }
}

#[test]
fn changing_modifiers_recalculates_the_preview_from_the_start_of_the_drag() {
    for group in [false, true] {
        let mut s = selection(group);
        let authored = s.doc.clone();
        let original = bounds(&authored, &s.selected);
        let start = screen(original.max);
        let pointer = start + egui::vec2(30., 5.);
        s.input(egui::Event::PointerMoved(start));
        s.button(start, true);
        let mut last = original;
        for mods in [
            Modifiers::NONE,
            Modifiers::CTRL,
            Modifiers::CTRL | Modifiers::SHIFT,
            Modifiers::SHIFT,
        ] {
            s.modifiers = mods;
            let (_, out) = s.paint(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
                modifiers: mods,
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            });
            assert_eq!(s.doc, authored);
            // The page can retain its original pixels while selected artwork is
            // drawn separately. Read the resize handles at their displayed pose.
            let view = s.canvas.context_surface.as_ref().unwrap().1;
            last = Rect::NOTHING;
            for shape in &out.shapes {
                if let egui::Shape::Rect(handle) = &shape.shape {
                    if handle.rect.size() == Vec2::splat(7.)
                        && handle.fill == Color32::WHITE
                        && handle.stroke.color == Color32::from_rgb(38, 137, 236)
                    {
                        last.extend_with(view.world(handle.rect.center()));
                    }
                }
            }
            assert!(
                last.is_positive(),
                "selected shape must show resize handles"
            );
            if mods.ctrl {
                assert!(last.center().distance(original.center()) < 1e-3);
            } else {
                assert!(last.min.distance(original.min) < 1e-3);
            }
            let scale = last.size() / original.size();
            assert!(scale.x > 1.);
            assert_eq!((scale.x - scale.y).abs() < 1e-4, mods.shift);
        }
        assert!(s.button(pointer, false));
        let committed = bounds(&s.doc, &s.selected);
        assert!(committed.min.distance(last.min) < 1e-3 && committed.max.distance(last.max) < 1e-3);
    }
}

#[test]
fn moved_pivot_controls_corner_and_side_scales_on_rotated_sheared_artwork() {
    for (handle, ctrl, uniform) in [
        ([1., 1.], true, false),
        ([1., 0.5], true, false),
        ([1., 1.], true, true),
        ([1., 1.], false, false),
    ] {
        let mut s = selection(false);
        let object = &mut s.doc.objects[0];
        object.rotation = 17.;
        object.animated.skew = -12.;
        object.scale = [-1., 1.];
        let pivot = [-0.25, 0.25];
        let fixed = Pos2::from(object.local_to_world(pivot, 0));
        assert!(object.set_pivot_at(0, fixed.into()));
        let world = object.local_to_world_at(0);
        let start = Pos2::from(world(handle));
        let factor = [
            1.35,
            if uniform {
                1.35
            } else if handle[1] == 0.5 {
                1.
            } else {
                0.8
            },
        ];
        let expected = |p: [f32; 2]| {
            Pos2::from(world(std::array::from_fn(|i| {
                pivot[i] + (p[i] - pivot[i]) * factor[i]
            })))
        };
        let mut target = expected(handle);
        if handle[1] == 0.5 {
            // A side grip ignores motion in the other local axis.
            target += (Pos2::from(world([1., 1.])) - Pos2::from(world([1., 0.]))) * 0.3;
        }
        let corner = expected([1., 1.]);
        s.modifiers = (if ctrl {
            Modifiers::CTRL
        } else {
            Modifiers::NONE
        }) | (if uniform {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        });
        assert!(s.drag(screen(start), screen(target)));
        let resized = &s.doc.objects[0];
        assert!(Pos2::from(resized.pivot_at(0)).distance(fixed) < 0.001);
        assert!(Pos2::from(resized.local_to_world([1., 1.], 0)).distance(corner) < 0.001);
        s.doc.validate().unwrap();
    }
}

#[test]
fn a_custom_pivot_on_a_grip_can_be_dragged_from_its_center_or_its_arm() {
    let mut s = selection(false);
    let o = &mut s.doc.objects[0];
    let corner: Pos2 = o.local_to_world([1., 1.], 0).into();
    assert!(o.set_pivot_at(0, corner.into()));
    let original = s.doc.clone();
    // A deliberately constrained resize from the pivot has no lever arm.
    s.modifiers = Modifiers::SHIFT;
    assert!(!s.drag(screen(corner), screen(corner + vec2(8., 5.))));
    assert_eq!(s.doc, original);
    s.modifiers = Modifiers::NONE;
    for offset in [Vec2::ZERO, vec2(8., 0.)] {
        s.doc = original.clone();
        let grab = screen(corner) + offset;
        let delta = vec2(10., -5.);
        assert!(s.drag(grab, grab + delta * 2.5));
        let moved = &s.doc.objects[0];
        assert!(Pos2::from(moved.pivot_at(0)).distance(corner + delta) < 0.001);
        assert_eq!(moved.size, original.objects[0].size);
        assert_eq!(moved.position, original.objects[0].position);
    }
}

#[test]
fn keying_rotation_and_scale_handles_keeps_the_hinge_fixed_between_keys() {
    let mut s = selection(false);
    let o = &mut s.doc.objects[0];
    let hinge = Pos2::from(o.local_to_world([0., 1.], 0));
    assert!(o.set_pivot_at(0, hinge.into()));
    let top: Pos2 = o.local_to_world([0.5, 0.], 0).into();
    let start = gesture::rotation_handle(o, 0, top, 2.5);
    s.timeline.frame = 4;
    s.timeline.key_mode = crate::timeline::KeyMode::Auto;
    s.modifiers = Modifiers::SHIFT;
    assert!(s.drag(
        screen(start),
        screen(gesture::rotate_around(start, hinge, 88.))
    ));
    let o = &s.doc.objects[0];
    assert!((o.rotation_at(4) - 90.).abs() < 1e-3);
    assert!(o.keys.is_empty() && o.scale_keys.is_empty());
    let corner: Pos2 = o.local_to_world([1., 0.], 4).into();
    let target = hinge + (corner - hinge) * 1.3;
    assert!(s.drag(screen(corner), screen(target)));
    let o = &s.doc.objects[0];
    assert!(o.keys.is_empty());
    assert!(Pos2::from(o.local_to_world([1., 0.], 4)).distance(target) < 1e-3);
    for f in 0..=4 {
        assert!(Pos2::from(o.pivot_at(f)).distance(hinge) < 1e-3);
        assert!((o.rotation_at(f) - 22.5 * f as f32).abs() < 1e-3);
        assert!((o.scale_at(f)[0] - (1. + 0.075 * f as f32)).abs() < 1e-3);
    }
    s.doc.validate().unwrap();
}

#[test]
fn group_handles_keep_their_shared_center_when_members_have_custom_pivots() {
    let mut s = selection(true);
    for (i, o) in s.doc.objects.iter_mut().enumerate() {
        let hinge = o.local_to_world([i as f32, -0.25], 0);
        assert!(o.set_pivot_at(0, hinge));
    }
    let before = s.doc.clone();
    let original = bounds(&before, &s.selected);
    let center = original.center();
    let factor = vec2(1.4, 0.8);
    s.modifiers = Modifiers::CTRL;
    assert!(s.drag(
        screen(original.max),
        screen(center + (original.max - center) * factor)
    ));
    let expanded = bounds(&s.doc, &s.selected);
    let rotate = expanded.center_top() - vec2(0., 18. / 2.5);
    s.modifiers = Modifiers::NONE;
    assert!(s.drag(
        screen(rotate),
        screen(gesture::rotate_around(rotate, center, 90.))
    ));
    for (before, after) in before.objects.iter().zip(&s.doc.objects) {
        for local in [[0., 0.], [1., 1.], before.pivot.unwrap()] {
            let start = Pos2::from(before.local_to_world(local, 0));
            let expected = gesture::rotate_around(center + (start - center) * factor, center, 90.);
            assert!(Pos2::from(after.local_to_world(local, 0)).distance(expected) < 1e-3);
        }
    }
    s.doc.validate().unwrap();
}
