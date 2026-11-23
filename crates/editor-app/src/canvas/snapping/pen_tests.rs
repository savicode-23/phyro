use super::*;
use document_core::{PathNode, Shape};

fn canvas_view() -> Rect {
    Rect::from_min_max(egui::pos2(-40., -30.), egui::pos2(180., 165.))
}

fn same(actual: Hit, expected: Hit) {
    assert_eq!(actual.point, expected.point);
    assert_eq!(actual.lines, expected.lines);
    assert_eq!(actual.anchor, expected.anchor);
}

#[test]
fn pen_snap_snapshot_matches_live_geometry_across_modes_zoom_and_edits() {
    let mut doc = Document {
        guides_x: vec![10., 12.],
        guides_y: vec![40.],
        ..Default::default()
    };
    let path = Shape::Path {
        nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([1.2, 2.8])],
        closed: false,
    };
    for x in [1., 3.] {
        doc.add_shape([x, 5.], [1., 1.], [255; 4], path.clone(), 1);
    }
    doc.add_shape([28., 30.], [8., 7.], [255; 4], path, 1);
    doc.objects[2].stroke = [30, 40, 20, 255];
    doc.objects[2].stroke_width = 4.;
    doc.objects[2].rotation = 53.;
    doc.objects[2].animated.skew = -23.;
    doc.objects[2].keys.insert(3, [90., 20.]);
    let source = Arc::new(doc);
    let mut cache = None;
    for frame in [0, 3, 1] {
        for flags in 0..16 {
            let settings = SnapSettings {
                enabled: true,
                grid: flags & 1 != 0,
                guides: flags & 2 != 0,
                bounds: flags & 4 != 0,
                anchors: flags & 8 != 0,
                grid_settings: GridSettings {
                    spacing: 3.,
                    ..Default::default()
                },
                ..Default::default()
            };
            for scale in [0.8, 3., 12.] {
                for p in [
                    egui::pos2(2., 5.),
                    egui::pos2(11., 40.),
                    egui::pos2(2.6, 45.),
                    egui::pos2(25., 29.),
                    egui::pos2(93., 23.),
                    egui::pos2(34.3, 37.),
                ] {
                    same(
                        PenTargets::after_paint(
                            &mut cache,
                            p,
                            &source,
                            frame,
                            settings,
                            scale,
                            canvas_view(),
                        ),
                        point(p, &source, frame, settings, scale),
                    );
                    same(
                        PenTargets::before_paint(
                            cache.as_ref(),
                            p,
                            &source,
                            frame,
                            settings,
                            scale,
                        ),
                        point(p, &source, frame, settings, scale),
                    );
                }
            }
        }
    }
    let settings = SnapSettings {
        enabled: true,
        ..Default::default()
    };
    let mut edited = (*source).clone();
    edited.layers[0].visible = false;
    edited.guides_y = vec![7.];
    let p = egui::pos2(2., 5.);
    same(
        PenTargets::before_paint(cache.as_ref(), p, &edited, 1, settings, 3.),
        point(p, &edited, 1, settings, 3.),
    );
    let edited = Arc::new(edited);
    same(
        PenTargets::after_paint(&mut cache, p, &edited, 1, settings, 3., canvas_view()),
        point(p, &edited, 1, settings, 3.),
    );
    assert_eq!(
        cache.as_ref().unwrap().source.as_ptr(),
        Arc::as_ptr(&edited)
    );
}

#[test]
fn anchor_and_axis_ties_follow_the_original_order() {
    let mut doc = Document {
        guides_x: vec![10., 12.],
        ..Default::default()
    };
    let path = Shape::Path {
        nodes: vec![PathNode::corner([0., 0.]), PathNode::corner([0., 1.])],
        closed: false,
    };
    for x in [1., 3.] {
        doc.add_shape([x, 5.], [4., 8.], [255; 4], path.clone(), 1);
    }
    let snapshot = Arc::new(doc);
    let settings = SnapSettings {
        enabled: true,
        bounds: false,
        ..Default::default()
    };
    let mut cache = None;
    let hits = PenTargets::after_paint(
        &mut cache,
        egui::pos2(2., 5.),
        &snapshot,
        0,
        settings,
        1.,
        canvas_view(),
    );
    assert_eq!(hits.anchor, Some(egui::pos2(3., 5.)));
    let hits = PenTargets::after_paint(
        &mut cache,
        egui::pos2(11., 20.1),
        &snapshot,
        0,
        settings,
        1.,
        canvas_view(),
    );
    assert_eq!(hits.lines, [Some(10.), None]);
    assert_eq!(hits.point, egui::pos2(10., 20.));
}

#[test]
fn cached_coverage_preserves_off_canvas_axes_and_press_before_paint() {
    let mut doc = Document::default();
    for pos in [[21., 400.], [400., 22.], [220., 220.], [640., 720.]] {
        doc.add_rectangle(pos, [11., 13.], [255; 4]);
    }
    let source = Arc::new(doc);
    let s = SnapSettings {
        enabled: true,
        grid: false,
        anchors: false,
        ..Default::default()
    };
    for (p, scale) in [
        (egui::pos2(640.3, 720.3), 3.),
        (egui::pos2(195., 195.), 0.15),
    ] {
        let mut cache = None;
        for inner in [egui::pos2(21.1, 22.2), egui::pos2(31.9, 35.1)] {
            let old = point(inner, &source, 0, s, 3.);
            let hit = PenTargets::after_paint(&mut cache, inner, &source, 0, s, 3., canvas_view());
            assert_eq!((hit.point, hit.lines), (old.point, old.lines));
            assert!(hit.lines.iter().all(Option::is_some));
        }
        assert!(cache.as_ref().unwrap().bounds_region.is_some());
        let expected = point(p, &source, 0, s, scale);
        assert!(expected.lines.iter().all(Option::is_some));
        for hit in [
            PenTargets::before_paint(cache.as_ref(), p, &source, 0, s, scale),
            PenTargets::after_paint(&mut cache, p, &source, 0, s, scale, canvas_view()),
        ] {
            assert_eq!((hit.point, hit.lines), (expected.point, expected.lines));
        }
        assert!(cache.as_ref().unwrap().covers(p, s.tolerance / scale));
    }
}
#[test]
fn visible_pasteboard_reuses_targets_and_captured_drag_rebuilds_near_the_pointer() {
    let mut doc = Document::default();
    // Each pair supplies an x and a y independently even though both shapes
    // lie outside the current viewport on their other axis.
    for pos in [
        [145., 450.],
        [520., -20.],
        [640., 1100.],
        [1100., 720.],
        [2300., 2300.],
    ] {
        doc.add_rectangle(pos, [7., 11.], [255; 4]);
    }
    let source = Arc::new(doc);
    let s = SnapSettings {
        enabled: true,
        grid: false,
        guides: false,
        anchors: false,
        ..Default::default()
    };
    let scale = 2.;
    let mut cache = None;
    let view = canvas_view();
    let middle = egui::pos2(52., 61.);
    same(
        PenTargets::after_paint(&mut cache, middle, &source, 0, s, scale, view),
        point(middle, &source, 0, s, scale),
    );
    let initial = cache.as_ref().unwrap().bounds_region;
    let outside = egui::pos2(145.3, -20.2);
    let expected = point(outside, &source, 0, s, scale);
    assert_eq!(expected.lines, [Some(145.), Some(-20.)]);
    same(
        PenTargets::before_paint(cache.as_ref(), outside, &source, 0, s, scale),
        point(outside, &source, 0, s, scale),
    );
    same(
        PenTargets::after_paint(&mut cache, outside, &source, 0, s, scale, view),
        expected,
    );
    assert_eq!(cache.as_ref().unwrap().bounds_region, initial);

    let far = egui::pos2(640.2, 719.8);
    // First a captured drag beyond the old viewport, then a fresh pan/zoom
    // with both the pointer and visible viewport in a distant part of the drawing.
    for (region, zoom) in [
        (view, 2.),
        (Rect::from_center_size(far, Vec2::splat(50.)), 7.),
    ] {
        let mut cache = None;
        PenTargets::after_paint(&mut cache, middle, &source, 0, s, scale, view);
        let expected = point(far, &source, 0, s, zoom);
        assert_eq!(expected.lines, [Some(640.), Some(720.)]);
        same(
            PenTargets::before_paint(cache.as_ref(), far, &source, 0, s, zoom),
            point(far, &source, 0, s, zoom),
        );
        same(
            PenTargets::after_paint(&mut cache, far, &source, 0, s, zoom, region),
            expected,
        );
        let active = cache.as_ref().unwrap();
        assert!(active.covers(far, s.tolerance / zoom));
        assert!(!active.geometry.axes[0].contains(&2300.));
    }
}
