use super::*;
use document_core::Shape;
use egui::{pos2, vec2, Event, Modifiers, Pos2};

fn button(p: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn touch(device: u64, phase: egui::TouchPhase, pos: Pos2, force: Option<f32>) -> Event {
    Event::Touch {
        device_id: egui::TouchDeviceId(device),
        id: egui::TouchId(1),
        phase,
        pos,
        force,
    }
}

#[test]
fn maximum_pencil_smoothing_keeps_the_visible_arch_and_loop_on_mouse_release() {
    use std::f32::consts::{PI, TAU};
    let deviations = [false, true].map(|closed| {
        let mut w = EditorWindow::with_artboard(vec2(1100., 740.), [256, 256]);
        w.editor.set_tool(Tool::Pencil);
        w.editor.pencil_smoothing = 200;
        let board = w.board();
        let scale = board.width() / 256.;
        let color = egui::Color32::from_rgb(210, 32, 134);
        w.editor.style.stroke = color.to_array();
        w.editor.style.stroke_width = 2. / scale;
        w.editor.style.fill = [0; 4];
        let samples: Vec<_> = (0..=90)
            .map(|i| {
                let t = i as f32 / 90.;
                board.center()
                    + if closed {
                        vec2(55. * (TAU * t).cos(), 42. * (TAU * t).sin())
                    } else {
                        vec2(190. * (t - 0.5), -29. * (PI * t).sin())
                    }
            })
            .collect();
        let start = samples[0];
        w.frame(vec![Event::PointerMoved(start), button(start, true)]);
        for batch in samples[1..].chunks(6) {
            w.frame(batch.iter().copied().map(Event::PointerMoved).collect());
        }
        let preview = w.frame(vec![]);
        let trace = preview
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Path(path)
                    if path.stroke.color == egui::epaint::ColorMode::Solid(color) =>
                {
                    Some(&path.points)
                }
                _ => None,
            })
            .expect("live pencil centerline");
        w.frame(vec![button(*samples.last().unwrap(), false)]);
        let doc = w.editor.scene();
        assert!(matches!(&doc.objects[0].shape, Shape::Path { closed: c, .. } if *c == closed));
        let image = renderer::render(doc, 0).unwrap();
        let width = image.width() as usize;
        let ink: Vec<_> = image
            .pixels()
            .iter()
            .enumerate()
            .filter(|(_, pixel)| pixel.alpha() > 64)
            .map(|(i, _)| {
                board.min + vec2((i % width) as f32 + 0.5, (i / width) as f32 + 0.5) * scale
            })
            .collect();
        // Compare the final rendered stroke to the last preview and the original mouse trace.
        trace
            .iter()
            .chain(&samples)
            .map(|p| {
                ink.iter()
                    .map(|q| p.distance_sq(*q))
                    .fold(f32::INFINITY, f32::min)
            })
            .fold(0., f32::max)
            .sqrt()
    });
    assert!(
        deviations.iter().all(|d| *d <= 6.),
        "maximum distance to final ink in screen points (arch, loop): {deviations:?}"
    );
}

#[test]
fn pencil_tablet_pressure_previews_and_commits_one_stroke_despite_mirrored_mouse_events() {
    use egui::TouchPhase::{Cancel, End, Move, Start};
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor.set_tool(Tool::Pencil);
    let board = w.board();
    w.control("Smoothing");
    w.click("Pressure width");
    assert!(w.editor.pencil_pressure);
    let start = board.center() - vec2(60., 0.);
    let middle = start + vec2(60., 0.);
    let end = middle + vec2(60., 0.);
    w.editor.style.stroke = [204, 48, 88, 255];
    w.editor.style.stroke_width = 32. * w.editor.scene().width as f32 / board.width();
    let preview = w.frame(vec![
        touch(1, Start, start, Some(0.)),
        Event::PointerMoved(start),
        button(start, true),
        touch(2, Start, start + vec2(0., 20.), Some(1.)),
        touch(2, Cancel, start + vec2(0., 20.), None),
        touch(1, Move, middle, Some(1.)),
        Event::PointerMoved(middle),
        touch(1, Move, end, Some(0.)),
        Event::PointerMoved(end),
    ]);
    let mesh = preview
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Mesh(m)
                if m.vertices
                    .iter()
                    .any(|v| v.color == egui::Color32::from_rgb(204, 48, 88)) =>
            {
                Some(m)
            }
            _ => None,
        })
        .expect("live pressure mesh");
    let radius = |x: f32| {
        mesh.vertices
            .iter()
            .filter(|v| (v.pos.x - x).abs() < 5.)
            .map(|v| (v.pos.y - start.y).abs())
            .fold(0., f32::max)
    };
    assert!(radius(start.x) < 4. && radius(middle.x) > 14.);
    w.frame(vec![
        touch(1, End, end, Some(0.)),
        button(end, false),
        Event::PointerGone,
    ]);
    let doc = w.editor.scene();
    assert_eq!(doc.objects.len(), 1);
    let o = &doc.objects[0];
    assert!(matches!(o.shape, Shape::Path { ref nodes, closed:false } if nodes.len() == 2));
    assert_eq!(o.stroke_profile[0], [0., 0.12]);
    assert_eq!(*o.stroke_profile.last().unwrap(), [1., 0.12]);
    assert!(o
        .stroke_profile
        .iter()
        .any(|p| (p[0] - 0.5).abs() < 0.03 && p[1] == 1.));
    doc.validate().unwrap();
    w.editor.restore(false);
    assert!(w.editor.scene().objects.is_empty());
}

#[test]
fn pencil_closes_pressure_seams_and_keeps_uniform_width_without_a_pressure_signal() {
    use egui::TouchPhase::{End, Move, Start};
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor.set_tool(Tool::Pencil);
    w.editor.pencil_smoothing = 200;
    let start = w.board().center();
    for (enabled, report, pressure) in [
        (true, true, true),
        (true, false, false),
        (false, true, false),
    ] {
        w.editor.pencil_pressure = enabled;
        w.frame(vec![
            touch(1, Start, start, report.then_some(0.3)),
            touch(1, Move, start + vec2(50., 0.), report.then_some(0.9)),
            touch(1, Move, start + vec2(50., 55.), report.then_some(0.5)),
            touch(1, Move, start + vec2(0., 55.), report.then_some(1.)),
            touch(1, End, start + vec2(0., 2.), report.then_some(0.)),
        ]);
        let o = w.editor.scene().objects.last().unwrap();
        assert!(matches!(o.shape, Shape::Path { closed: true, .. }));
        if pressure {
            assert!(o.stroke_profile.len() >= 3);
            assert_eq!(o.stroke_profile[0][1], o.stroke_profile.last().unwrap()[1]);
            assert!(o.stroke_profile[0][1] > 0.3);
        } else {
            assert!(o.stroke_profile.is_empty());
        }
    }
    w.frame(vec![
        Event::PointerMoved(start),
        button(start, true),
        Event::PointerMoved(start + vec2(50., 0.)),
        button(start + vec2(50., 0.), false),
    ]);
    assert!(w
        .editor
        .scene()
        .objects
        .last()
        .unwrap()
        .stroke_profile
        .is_empty());
}

#[test]
fn pencil_closes_on_release_in_screen_space_and_previews_the_snap() {
    for zoom in [1., 3.] {
        let mut w = EditorWindow::new(vec2(1100., 740.));
        w.editor.set_tool(Tool::Pencil);
        w.editor.style.snap.enabled = zoom == 1.;
        let fit = w.board();
        w.frame(vec![Event::PointerMoved(fit.center()), Event::Zoom(zoom)]);
        let board = w.board();
        if zoom > 1. {
            assert!(board.width() > fit.width() * 2.);
        }
        let scale = board.width() / w.editor.scene().width as f32;
        let start = fit.center() - vec2(36., 28.);
        let inside = start + vec2(0., 11.);
        let outside = start + vec2(0., 13.);
        let circuit = || {
            vec![
                Event::PointerMoved(start),
                button(start, true),
                Event::PointerMoved(start + vec2(70., 0.)),
                Event::PointerMoved(start + vec2(70., 54.)),
                Event::PointerMoved(start + vec2(0., 54.)),
            ]
        };
        let closing_ring = |output: &egui::FullOutput| {
            output.shapes.iter().any(|s| {
                matches!(&s.shape, egui::Shape::Circle(c)
                if c.center.distance(start) < 0.1 && c.radius >= 7. && c.stroke.width > 0.)
            })
        };

        let before = w.editor.project.clone();
        w.frame(circuit());
        assert!(!closing_ring(&w.frame(vec![Event::PointerMoved(outside)])));
        w.frame(vec![button(inside, false)]); // Only the lift moves into range.
        let closed = w.editor.project.clone();
        let obj = &w.editor.scene().objects[0];
        let Shape::Path {
            nodes,
            closed: true,
        } = &obj.shape
        else {
            panic!("the Pencil must snap and close at zoom {zoom}")
        };
        let screen = |node: &document_core::PathNode| {
            board.min + egui::Vec2::from(obj.local_to_world(node.point, 0)) * scale
        };
        assert!(nodes.len() >= 3 && screen(&nodes[0]).distance(start) < 0.01);
        assert!(nodes[1..].iter().all(|n| screen(n).distance(start) > 0.1));
        assert_eq!(w.editor.selection, [obj.id]);

        w.frame(circuit());
        assert!(closing_ring(&w.frame(vec![Event::PointerMoved(inside)])));
        assert!(!closing_ring(&w.frame(vec![Event::PointerMoved(outside)])));
        w.frame(vec![Event::PointerMoved(inside)]);
        w.frame(vec![button(outside, false)]); // Crossing the start doesn't latch closure.
        let open = &w.editor.scene().objects[1];
        let Shape::Path {
            nodes,
            closed: false,
        } = &open.shape
        else {
            panic!("the Pencil must finish open outside the snap radius")
        };
        let end = open.local_to_world(nodes.last().unwrap().point, 0);
        let expected = (outside - board.min) / scale;
        assert!(pos2(end[0], end[1]).distance(expected.to_pos2()) < 0.01);
        assert!(w.editor.tool == Tool::Pencil);
        w.editor.restore(false);
        assert_eq!(w.editor.project, closed);
        w.editor.restore(false);
        assert_eq!(w.editor.project, before);
    }
}

#[test]
fn batched_pencil_gestures_close_only_the_completed_loop() {
    let mut w = EditorWindow::new(vec2(800., 500.));
    w.editor.set_tool(Tool::Pencil);
    let start = w.board().center() - vec2(20., 18.);
    let mut batch = Vec::new();
    for offsets in [
        vec![vec2(4., 0.), vec2(3., 4.), vec2(0., 4.)], // A short mark near its start.
        vec![vec2(50., 0.), vec2(1., 0.)],              // A straight out-and-back stroke.
        vec![vec2(42., 0.), vec2(25., 43.), vec2(2., 1.)],
    ] {
        batch.extend([Event::PointerMoved(start), button(start, true)]);
        batch.extend(
            offsets
                .iter()
                .map(|&offset| Event::PointerMoved(start + offset)),
        );
        batch.push(button(start + *offsets.last().unwrap(), false));
    }
    w.frame(batch);
    let drawing = w.editor.scene();
    assert_eq!(drawing.objects.len(), 3);
    for (object, expected) in drawing.objects.iter().zip([false, false, true]) {
        assert!(
            matches!(object.shape, Shape::Path { closed, .. } if closed == expected),
            "batched stroke {} should have closed={expected}",
            object.id
        );
    }
    w.editor.restore(false);
    assert_eq!(w.editor.scene().objects.len(), 2);
}

#[test]
fn pencil_tablet_batches_match_individual_mouse_events_and_each_stroke_undoes() {
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor.set_tool(Tool::Pencil);
    let board = w.board();
    let fill = [120, 40, 200, 63];
    let stroke = [20, 111, 142, 255];
    w.editor.style.fill = fill;
    w.editor.style.no_fill = false;
    w.editor.style.stroke = stroke;
    w.editor.style.stroke_width = 2.5;
    let before = w.editor.project.clone();
    let samples: Vec<_> = (0..64)
        .map(|i| {
            let t = i as f32 / 63.;
            board.min + board.size() * vec2(0.12 + 0.71 * t, 0.45 + 0.2 * (t * 7.).sin())
        })
        .collect();
    let a = samples[0];
    let b = *samples.last().unwrap();
    let body: Vec<_> = samples[1..]
        .iter()
        .copied()
        .map(Event::PointerMoved)
        .collect();
    w.frame(vec![Event::PointerMoved(a), button(a, true)]);
    for event in body.clone() {
        w.frame(vec![event]);
    }
    assert!(w.editor.scene().objects.is_empty() && !w.editor.can_undo());
    w.frame(vec![button(b, false)]);
    let first = w.editor.project.clone();
    let o = &w.editor.scene().objects[0];
    assert_eq!(w.editor.selection, [o.id]);
    assert_eq!((o.fill, o.stroke, o.stroke_width), (fill, stroke, 2.5));
    let Shape::Path {
        nodes,
        closed: false,
    } = &o.shape
    else {
        panic!("open Bézier path")
    };
    assert!((3..24).contains(&nodes.len()));
    assert!(nodes.iter().any(|n| n.point != n.outgoing));
    for (&node, screen) in [nodes.first().unwrap(), nodes.last().unwrap()]
        .into_iter()
        .zip([a, b])
    {
        let world = o.local_to_world(node.point, 0);
        let expected = (screen - board.min) * (w.editor.scene().width as f32 / board.width());
        assert!(pos2(world[0], world[1]).distance(expected.to_pos2()) < 0.001);
    }
    let mut batch = vec![Event::PointerMoved(a), button(a, true)];
    batch.extend(body);
    batch.push(button(b, false));
    batch.push(Event::PointerGone); // Native tablet lift after the release.
    batch.extend(batch.clone());
    w.frame(batch);
    let scene = w.editor.scene();
    assert_eq!(scene.objects.len(), 3);
    assert_eq!(scene.objects[0].shape, scene.objects[1].shape);
    assert_eq!(scene.objects[0].shape, scene.objects[2].shape);
    assert_eq!(scene.objects[0].position, scene.objects[1].position);
    assert_eq!(w.editor.selection, [scene.objects[2].id]);
    assert!(w.editor.tool == Tool::Pencil);
    w.editor.restore(false);
    assert_eq!(w.editor.scene().objects.len(), 2);
    w.editor.restore(false);
    assert_eq!(w.editor.project, first);
    w.editor.restore(false);
    assert_eq!(w.editor.project, before);
}

#[test]
fn pencil_taps_escape_and_lost_pointer_leave_the_drawing_and_history_intact() {
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor.set_tool(Tool::Pencil);
    let board = w.board();
    let a = board.center();
    let b = a + vec2(32., -19.);
    let initial = w.editor.project.clone();
    w.click_at(a);
    for stop in [
        Event::PointerGone,
        Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        },
    ] {
        w.frame(vec![
            Event::PointerMoved(a),
            button(a, true),
            Event::PointerMoved(b),
        ]);
        assert!(w.editor.canvas.pencil_in_progress());
        w.frame(vec![stop]);
        assert!(!w.editor.canvas.pencil_in_progress());
        w.frame(vec![button(b, false)]);
    }
    assert_eq!(w.editor.project, initial);
    assert!(!w.editor.can_undo());
}

#[test]
fn pencil_can_draw_off_page_and_capture_a_release_outside_the_canvas() {
    let mut w = EditorWindow::new(vec2(1100., 740.));
    w.editor.set_tool(Tool::Pencil);
    let board = w.board();
    let start = board.left_center() - vec2(9., 0.);
    let over_inspector = pos2(1060., start.y + 6.);
    w.frame(vec![Event::PointerMoved(start), button(start, true)]);
    w.frame(vec![
        Event::PointerMoved(over_inspector),
        button(over_inspector, false),
    ]);
    let [o] = w.editor.scene().objects.as_slice() else {
        panic!("one off-page pencil path")
    };
    let nodes = &o.shape.contours()[0];
    let first = o.local_to_world(nodes[0].point, 0);
    let last = o.local_to_world(nodes.last().unwrap().point, 0);
    assert!(first[0] < 0. && last[0] > w.editor.scene().width as f32);
    w.editor.restore(false);
    // A gesture that starts over a panel stays with that panel.
    w.frame(vec![
        Event::PointerMoved(over_inspector),
        button(over_inspector, true),
    ]);
    w.frame(vec![Event::PointerMoved(start), button(start, false)]);
    assert!(w.editor.scene().objects.is_empty());
}
