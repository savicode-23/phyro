use super::*;
use document_core::{PathNode, Shape};

#[test]
fn drag_snap_preserves_moving_bounds_anchor_exclusions_and_mode_changes() {
    let mut doc = Document::default();
    let path = Shape::Path {
        nodes: [[0., 0.], [0.5, 0.8], [1., 0.4]]
            .map(PathNode::corner)
            .to_vec(),
        closed: false,
    };
    let left = doc.add_shape([5., 6.], [10., 8.], [255; 4], path.clone(), 1);
    let right = doc.add_shape([20., 4.], [12., 6.], [255; 4], path, 1);
    doc.guides_x.push(33.);
    doc.objects[0].rotation = 14.;
    doc.objects[1].keys.insert(2, [25., 15.]);
    let selected = GestureIds::new(vec![right, left]);
    let painted = Arc::new(doc.clone());
    for kind in 0..4 {
        let subject = |delta| match kind {
            0 => Subject::Moving(&selected, delta),
            1 => Subject::Object(left),
            2 => Subject::Nodes {
                id: left,
                nodes: &NodeSet::Range((0, 0, 1)),
            },
            _ => Subject::Point,
        };
        let mut cache = None;
        let mut active = doc.clone();
        for edit in 0..2 {
            if edit == 1 {
                active.objects[1].position[1] += 5.;
                active.guides_x[0] += 2.;
            }
            for frame in [0, 2] {
                for flags in 0..16 {
                    let settings = SnapSettings {
                        enabled: true,
                        grid: flags & 1 != 0,
                        guides: flags & 2 != 0,
                        anchors: flags & 4 != 0,
                        bounds: flags & 8 != 0,
                        grid_settings: GridSettings {
                            spacing: 2.,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    for scale in [0.8, 3.] {
                        for delta in [egui::vec2(1., 1.), egui::vec2(8., 6.), egui::vec2(15., -2.)]
                        {
                            let p = egui::pos2(10., 8.) + delta;
                            let expected =
                                resolve(p, &active, frame, settings, scale, subject(delta));
                            let actual = DragTargets::resolve(
                                &mut cache,
                                Some(&painted),
                                p,
                                &active,
                                frame,
                                settings,
                                scale,
                                subject(delta),
                                Rect::from_min_max(Pos2::ZERO, egui::pos2(48., 48.)),
                            );
                            assert_eq!(actual.point, expected.point);
                            assert_eq!(actual.anchor, expected.anchor);
                            assert_eq!(actual.lines, expected.lines);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn dense_snapping_keeps_paint_order_and_finds_anchors_after_leaving_view() {
    let mut doc = Document::default();
    doc.add_shape(
        [9., 10.],
        [2., 8.],
        [255; 4],
        Shape::Path {
            nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([0., 1.])],
            closed: false,
        },
        1,
    );
    let source = doc.objects[0].clone();
    for i in 1..4096 {
        let mut other = source.clone();
        other.id = source.id + i;
        other.position = match i {
            2048 => [13., 900.],
            3000 => [500., 500.],
            4095 => [11., 10.],
            _ => [1500. + i as f32, 900.],
        };
        doc.objects.push(other);
    }
    let doc = Arc::new(doc);
    let visible = Rect::from_min_max(Pos2::ZERO, egui::pos2(64., 64.));
    for (bounds, anchors) in [(false, true), (true, false), (true, true)] {
        let settings = SnapSettings {
            enabled: true,
            bounds,
            anchors,
            guides: false,
            grid: false,
            tolerance: 2.,
            ..Default::default()
        };
        let mut cache = None;
        for p in [
            egui::pos2(10., 10.),
            egui::pos2(13., 27.),
            egui::pos2(501., 500.),
            egui::pos2(10., 10.),
        ] {
            let expected = resolve(p, &doc, 0, settings, 1., Subject::Point);
            let actual = DragTargets::resolve(
                &mut cache,
                Some(&doc),
                p,
                &doc,
                0,
                settings,
                1.,
                Subject::Point,
                visible,
            );
            assert_eq!(
                (actual.point, actual.lines, actual.anchor),
                (expected.point, expected.lines, expected.anchor)
            );
            assert!(cache.as_ref().unwrap().targets.anchors.len() <= 4);
            if anchors && p == egui::pos2(10., 10.) {
                assert_eq!(actual.anchor, Some(egui::pos2(11., 10.)));
            }
            if bounds && p == egui::pos2(13., 27.) {
                assert_eq!(actual.lines[0], Some(13.));
            }
        }
    }
}

#[test]
fn anchors_outside_a_parked_paths_size_box_remain_snap_targets() {
    let mut doc = Document::default();
    doc.add_shape(
        [480., 360.],
        [20., 40.],
        [255; 4],
        Shape::Path {
            nodes: vec![PathNode::corner([-12., 7.]), PathNode::corner([1., 1.])],
            closed: false,
        },
        1,
    );
    let object = &mut doc.objects[0];
    object.scale = [-1., 0.75];
    object.rotation = 32.;
    let anchor = Pos2::from(object.local_to_world([-12., 7.], 0));
    let doc = Arc::new(doc);
    let visible = Rect::from_center_size(anchor, Vec2::splat(20.));
    let settings = SnapSettings {
        enabled: true,
        ..Default::default()
    };
    let pointer = anchor + egui::vec2(1., 0.);
    let hit = DragTargets::resolve(
        &mut None,
        Some(&doc),
        pointer,
        &doc,
        0,
        settings,
        1.,
        Subject::Point,
        visible,
    );
    assert_eq!(hit.anchor, Some(anchor));
    assert_eq!(
        hit.point,
        resolve(pointer, &doc, 0, settings, 1., Subject::Point).point
    );
}

#[test]
fn a_target_edited_between_pointer_events_moves_the_snap_line() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([10., 15.], [4., 8.], [255; 4]);
    let snapshot = Arc::new(doc.clone());
    let settings = SnapSettings {
        enabled: true,
        bounds: true,
        anchors: false,
        grid: false,
        guides: false,
        ..Default::default()
    };
    let mut cache = None;
    let mut hit = |doc: &Document| {
        let hit = DragTargets::resolve(
            &mut cache,
            Some(&snapshot),
            egui::pos2(10., 7.),
            doc,
            0,
            settings,
            1.,
            Subject::Point,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(128., 128.)),
        );
        (
            hit.lines[0],
            Arc::ptr_eq(&cache.as_ref().unwrap().original, &snapshot),
        )
    };
    assert_eq!(hit(&doc), (Some(10.), true));
    doc.objects
        .iter_mut()
        .find(|o| o.id == id)
        .unwrap()
        .position[0] = 12.;
    assert_eq!(hit(&doc), (Some(12.), false));
}

#[test]
fn moving_an_offscreen_group_edge_keeps_distant_axis_guides_as_the_drag_leaves_view() {
    let mut doc = Document::default();
    let near = doc.add_rectangle([20., 20.], [10., 10.], [255; 4]);
    let far = doc.add_rectangle([400., 400.], [6., 6.], [255; 4]);
    let mut group = vec![near, far];
    for i in 0..16 {
        group.push(doc.add_rectangle([50. + i as f32 * 5., 80.], [2., 3.], [255; 4]));
    }
    group.reverse();
    let selected = GestureIds::new(group);
    // These objects can supply an x guide even though their y bounds are offscreen.
    doc.add_rectangle([410., 900.], [10., 10.], [255; 4]);
    doc.add_rectangle([1008., 900.], [10., 10.], [255; 4]);
    doc.add_rectangle([2000., 2000.], [10., 10.], [255; 4]);
    let snapshot = Arc::new(doc);
    let settings = SnapSettings {
        enabled: true,
        tolerance: 2.,
        grid: false,
        ..Default::default()
    };
    let visible = Rect::from_min_max(Pos2::ZERO, egui::pos2(100., 100.));
    let mut cache = None;
    for (offset, line) in [(2., 410.), (600., 1008.), (2., 410.)] {
        let delta = egui::vec2(offset, 0.);
        let p = egui::pos2(20., 20.) + delta;
        let subject = Subject::Moving(&selected, delta);
        let actual = DragTargets::resolve(
            &mut cache,
            Some(&snapshot),
            p,
            &snapshot,
            0,
            settings,
            1.,
            subject,
            visible,
        );
        let expected = resolve(p, &snapshot, 0, settings, 1., subject);
        assert_eq!(actual.lines, [Some(line), None]);
        assert_eq!(actual.lines, expected.lines);
        assert_eq!(actual.point, expected.point);
        assert!(!cache.as_ref().unwrap().targets.axes[0].contains(&2000.));
    }
}

// Opt-in timing of snap target collection on the first pointer movement.
#[test]
#[ignore = "release timing benchmark"]
fn large_selection_snap_target_responsiveness() {
    use super::super::gesture::{Gesture, GestureIds};
    use std::time::Instant;

    let mut doc = Document::default();
    let mut selection = Vec::new();
    let mut target_lines = [Vec::new(), Vec::new()];
    for i in 0..10_000 {
        let x = (i % 200) as f32 * 8.;
        let y = (i / 200) as f32 * 8.;
        let id = doc.add_rectangle([x, y], [4., 4.], [255; 4]);
        if i % 5 == 0 {
            target_lines[0].extend([x, x + 2., x + 4.]);
            target_lines[1].extend([y, y + 2., y + 4.]);
        } else {
            selection.push(id);
        }
    }
    selection.reverse();
    let start = egui::pos2(20., 20.);
    let bounds = Rect::from_min_max(Pos2::ZERO, egui::pos2(1700., 500.));
    let gestures = [
        Gesture::Move {
            start,
            reference: start,
            flips: [false; 2],
            ids: GestureIds::new(selection.clone()),
        },
        Gesture::GroupResize {
            start,
            handle: start,
            fixed: bounds.right_bottom(),
            ids: GestureIds::new(selection),
        },
    ];
    let settings = SnapSettings {
        enabled: true,
        grid: false,
        guides: false,
        anchors: false,
        ..Default::default()
    };
    for (name, gesture) in ["move", "scale"].into_iter().zip(gestures) {
        let (_, subject) = gesture.snap_subject(start + egui::vec2(3., 4.)).unwrap();
        let mut samples = Vec::new();
        for _ in 0..21 {
            let begin = Instant::now();
            let targets = Targets::new(&doc, 0, settings, subject, Some(bounds));
            samples.push(begin.elapsed());
            assert_eq!(targets.axes, target_lines);
            if name == "move" {
                assert_eq!(
                    targets.moving_bounds,
                    Rect::from_min_max(egui::pos2(8., 0.), egui::pos2(1596., 396.))
                );
            }
        }
        samples.sort_unstable();
        println!(
            "8000 selected / 10000 rectangles {name} snap targets p50 {:?}, p95 {:?}",
            samples[10], samples[19]
        );
    }
}
