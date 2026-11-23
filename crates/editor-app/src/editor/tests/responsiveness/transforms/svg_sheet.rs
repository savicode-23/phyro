use super::*;
use egui::{pos2, vec2, Event, PointerButton, Pos2, Rect};

fn mouse(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        pressed,
        button,
        modifiers: Default::default(),
    }
}

fn drag_points(view: &mut Viewport, label: &str, from: Pos2, delta: egui::Vec2) {
    view.frame(vec![Event::PointerMoved(from)], 0.5);
    let press = view.frame(vec![mouse(from, PointerButton::Primary, true)], 1. / 60.);
    let mut motion = view.frames(14, |i| {
        vec![Event::PointerMoved(from + delta * ((i + 1) as f32 / 14.))]
    });
    let release = view.frame(
        vec![mouse(from + delta, PointerButton::Primary, false)],
        1. / 60.,
    );
    let first = motion.remove(0);
    println!(
        "SVG {label}: press {:.3} ms, first {:.3} ms, release {:.3} ms",
        press.ms, first.ms, release.ms
    );
    report("steady", motion);
}

fn pan_to(view: &mut Viewport, center: Pos2) {
    let delta = view.board.center() - on_screen(view, center.into());
    let start = view.board.center() - vec2(delta.x.signum() * 400., 0.);
    view.frame(vec![Event::PointerMoved(start)], 1. / 60.);
    view.frame(vec![mouse(start, PointerButton::Middle, true)], 1. / 60.);
    view.frames(4, |i| {
        vec![Event::PointerMoved(start + delta * ((i + 1) as f32 / 4.))]
    });
    view.frame(
        vec![mouse(start + delta, PointerButton::Middle, false)],
        1. / 60.,
    );
}

fn box_select(view: &mut Viewport, region: Rect) -> Vec<u64> {
    let from = on_screen(view, region.min.into());
    let to = on_screen(view, region.max.into());
    drag_points(view, "Marquee", from, to - from);
    let ids = view.window.editor.selection.clone();
    assert!(
        ids.len() > 1,
        "region must contain multiple selectable paths"
    );
    ids
}

fn body(view: &Viewport, ids: &[u64], region: Rect) -> Pos2 {
    let scene = view.window.editor.scene();
    let selected: std::collections::HashSet<_> = ids.iter().copied().collect();
    for o in scene
        .paint_order()
        .into_iter()
        .rev()
        .filter(|o| selected.contains(&o.id))
    {
        let [l, t, r, b] = o.bounds(0);
        let p = pos2((l + r) * 0.5, (t + b) * 0.5);
        if region.shrink(20.).contains(p)
            && scene
                .hit_test(p.into(), 0)
                .is_some_and(|id| selected.contains(&id))
        {
            return on_screen(view, p.into());
        }
    }
    panic!("no accessible selected ink in the requested SVG region")
}

#[test]
#[ignore = "set PHYRO_PROFILE_SVG and PHYRO_PROFILE_SVG_RECT (x0,y0,x1,y1) to profile a box selection"]
fn profile_svg_pasteboard_selection() {
    let path = std::env::var_os("PHYRO_PROFILE_SVG").expect("set PHYRO_PROFILE_SVG");
    let corners: Vec<f32> = std::env::var("PHYRO_PROFILE_SVG_RECT")
        .expect("set PHYRO_PROFILE_SVG_RECT")
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    let [l, t, r, b] = corners[..] else {
        panic!("expected x0,y0,x1,y1")
    };
    let region = Rect::from_min_max(pos2(l, t), pos2(r, b));
    let doc = project_io::import_svg(Path::new(&path)).unwrap();
    let project = AnimationProject::from_document(doc);
    for dpi in [1., 2.] {
        let mut view = Viewport {
            window: EditorWindow::new(vec2(1920., 1080.)),
            dpi,
            time: 0.,
            board: Rect::NOTHING,
        };
        view.window.editor.replace_project(project.clone(), None);
        view.window.editor.inspector_tab = crate::inspector::Tab::Properties;
        view.frames(3, |_| Vec::new());
        pan_to(&mut view, region.center());
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(10));
            view.frame(Vec::new(), 1. / 60.);
        }
        println!(
            "SVG sheet at {dpi}x, artboard {:?}, region {:?}",
            view.board, region
        );
        let ids = box_select(&mut view, region);
        println!(
            "Selected {} objects of {}",
            ids.len(),
            view.window.editor.scene().objects.len()
        );
        for edit in [Edit::Move, Edit::Scale, Edit::Rotate] {
            let (from, delta) = match edit {
                Edit::Move => (body(&view, &ids, region), vec2(57., 21.)),
                Edit::Scale => (painted_handle(&mut view, edit), vec2(46., 22.)),
                Edit::Rotate => (painted_handle(&mut view, edit), vec2(69., 16.)),
            };
            drag_points(&mut view, &format!("{edit:?}"), from, delta);
            report(
                "first paint after release",
                vec![view.frame(Vec::new(), 1. / 60.)],
            );
            // Let the exact page and any visible signed tiles arrive while the
            // pointer is idle, before Undo invalidates the committed snapshot.
            let settling = (0..24)
                .map(|_| {
                    std::thread::sleep(Duration::from_millis(8));
                    view.frame(Vec::new(), 1. / 60.)
                })
                .collect();
            report("settling release", settling);
            assert_eq!(view.window.editor.selection, ids);
            let original = project.clips[0].drawing_at(0);
            let edited = view.window.editor.scene();
            for &id in &ids {
                let slot = original.objects.iter().position(|o| o.id == id).unwrap();
                let before = &original.objects[slot];
                let after = &edited.objects[slot];
                assert_eq!(after.id, id);
                match edit {
                    Edit::Move => assert_ne!(before.position_at(0), after.position_at(0)),
                    Edit::Scale => assert_ne!(before.scaled_box_at(0).1, after.scaled_box_at(0).1),
                    Edit::Rotate => assert_ne!(before.rotation_at(0), after.rotation_at(0)),
                }
            }
            view.window.editor.restore(false);
            assert!(
                view.window.editor.project == project,
                "Undo must restore the SVG"
            );
            view.window.editor.selection.clone_from(&ids);
            for _ in 0..12 {
                std::thread::sleep(Duration::from_millis(10));
                view.frame(Vec::new(), 1. / 60.);
            }
        }
    }
}
