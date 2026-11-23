use super::*;
mod offscreen;
mod svg_sheet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Move,
    Scale,
    Rotate,
}

fn wheel_zoom(view: &mut Viewport, at: egui::Pos2) {
    let fit = view.board.width();
    view.frame(vec![egui::Event::PointerMoved(at)], 1. / 60.);
    view.frame(
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 400.),
            modifiers: Default::default(),
        }],
        1. / 60.,
    );
    let mut uploaded = false;
    let start = Instant::now();
    for i in 0..600 {
        std::thread::sleep(Duration::from_millis(8));
        uploaded |= view.frame(Vec::new(), 1. / 60.).uploads > 0;
        if i >= 40 && uploaded {
            break;
        }
    }
    assert!(
        uploaded,
        "zoom did not sharpen within {:?}",
        start.elapsed()
    );
    assert!(
        (view.board.width() / fit - 2.).abs() < 0.04,
        "expected 2x artwork"
    );
}

fn on_screen(view: &Viewport, point: [f32; 2]) -> egui::Pos2 {
    let doc = view.window.editor.scene();
    view.board.min
        + view.board.size() * egui::vec2(point[0] / doc.width as f32, point[1] / doc.height as f32)
}

fn painted_handle(view: &mut Viewport, edit: Edit) -> egui::Pos2 {
    let input = view.input(Vec::new(), 1. / 60.);
    let out = view.window.frame_with_input(input);
    let accent = egui::Color32::from_rgb(38, 137, 236);
    out.shapes
        .iter()
        .find_map(|s| match (&s.shape, edit) {
            (egui::Shape::Rect(r), Edit::Scale)
                if r.stroke.color == accent
                    && r.fill == egui::Color32::WHITE
                    && r.rect.size() == egui::Vec2::splat(7.)
                    && s.clip_rect.contains_rect(r.rect) =>
            {
                Some(r.rect.center())
            }
            (egui::Shape::Circle(c), Edit::Rotate)
                if c.stroke.color == accent
                    && c.fill == egui::Color32::WHITE
                    && c.radius == 4.
                    && s.clip_rect.shrink(4.).contains(c.center) =>
            {
                Some(c.center)
            }
            _ => None,
        })
        .expect("selected artwork must paint the requested handle")
}

fn gesture(
    view: &mut Viewport,
    original: &AnimationProject,
    ids: &[u64],
    point: [f32; 2],
    edit: Edit,
) {
    let editor = &mut view.window.editor;
    editor.tool = Tool::Select;
    // Start a move on the artwork. Select handle targets first, as in the Objects list.
    editor.selection = if edit == Edit::Move {
        Vec::new()
    } else {
        ids.to_vec()
    };
    // Let the inspector and selected track appear before locating the handle.
    view.frames(3, |_| Vec::new());
    let start = if edit == Edit::Move {
        on_screen(view, point)
    } else {
        painted_handle(view, edit)
    };
    let delta = match edit {
        Edit::Move => egui::vec2(24., 11.),
        Edit::Scale => egui::vec2(-22., -16.),
        Edit::Rotate => egui::vec2(24., 6.),
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    view.frame(vec![egui::Event::PointerMoved(start)], 1. / 60.);
    let press = view.frame(vec![button(start, true)], 1. / 60.);
    let targets = if edit == Edit::Move {
        let targets = view.window.editor.selection.clone();
        assert!(targets.contains(&ids[0]), "press must select the target");
        targets
    } else {
        ids.to_vec()
    };
    let previews = view.frames(12, |i| {
        vec![egui::Event::PointerMoved(
            start + delta * ((i + 1) as f32 / 12.),
        )]
    });
    let release = view.frame(vec![button(start + delta, false)], 1. / 60.);
    let worst = previews
        .iter()
        .map(|s| s.ms)
        .fold(press.ms.max(release.ms), f64::max);
    println!("{edit:?} ({} targets) press {:.3} ms, first preview {:.3} ms, release {:.3} ms, worst {:.3} ms",targets.len(),press.ms,previews[0].ms,release.ms,worst);
    report("steady motion", previews.into_iter().skip(1).collect());
    let before = original.clips[0].drawing_at(0);
    for id in targets {
        let a = before.objects.iter().find(|o| o.id == id).unwrap();
        let b = view
            .window
            .editor
            .scene()
            .objects
            .iter()
            .find(|o| o.id == id)
            .unwrap();
        match edit {
            Edit::Move => assert_ne!(
                a.position_at(0),
                b.position_at(0),
                "move did not translate {id}"
            ),
            Edit::Scale => assert_ne!(
                a.scaled_box_at(0).1,
                b.scaled_box_at(0).1,
                "scale did not resize {id}"
            ),
            Edit::Rotate => assert_ne!(
                a.rotation_at(0),
                b.rotation_at(0),
                "rotation did not turn {id}"
            ),
        }
    }
    view.window.editor.restore(false);
    assert!(
        view.window.editor.project == *original,
        "undo failed to restore the project"
    );
    view.frames(2, |_| Vec::new());
}

fn measure(name: &str, project: &AnimationProject) {
    for (w, h, dpi) in [(800., 500., 1.), (1100., 740., 2.)] {
        let mut view = Viewport {
            window: EditorWindow::new(egui::vec2(w, h)),
            dpi,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        view.window.editor.replace_project(project.clone(), None);
        view.frames(5, |_| Vec::new());
        let doc = project.clips[0].drawing_at(0);
        let tile = (doc.width / 128 / 2 * 128) as f32;
        let point = [tile + 55., tile + 41.];
        let id = doc.hit_test(point, 0).unwrap();
        let group = if doc.width == 128 {
            ["Far sleeve", "Sleeve fabric"]
                .map(|name| doc.objects.iter().find(|o| o.name == name).unwrap().id)
                .to_vec()
        } else {
            let other = doc
                .hit_test([tile + 128. + 55., tile + 128. + 41.], 0)
                .unwrap();
            vec![id, other]
        };
        for zoom in [1, 2] {
            if zoom == 2 {
                let center = on_screen(&view, [tile + 55., tile + 62.]);
                wheel_zoom(&mut view, center);
            }
            println!("\n{name} {w}x{h} @{dpi}x, {zoom}x fit");
            let hit = on_screen(&view, point);
            report(
                "hover",
                view.frames(20, |i| {
                    vec![egui::Event::PointerMoved(
                        hit + egui::vec2((i % 4) as f32, 0.),
                    )]
                }),
            );
            for edit in [Edit::Move, Edit::Scale, Edit::Rotate] {
                gesture(&mut view, project, &[id], point, edit);
            }
            for edit in [Edit::Scale, Edit::Rotate] {
                gesture(&mut view, project, &group, point, edit);
            }
        }
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn viewport_responsiveness_transforms() {
    for (name, project) in scenes() {
        measure(&name, &project);
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn viewport_responsiveness_offboard() {
    for_each_offboard_scene(|count, project| {
        measure(&format!("walk + {count} off-artboard objects"), project);
    });
}

fn for_each_offboard_scene(mut check: impl FnMut(usize, &AnimationProject)) {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let walk = project_io::load_animation(&examples.join("lantern-walk.vanim")).unwrap();
    let keeper = project_io::load_animation(&examples.join("lantern-keeper.vanim")).unwrap();
    let source = &keeper.clips[0].drawing_at(0).objects;
    let rest = renderer::render(walk.clips[0].drawing_at(0), 0).unwrap();
    for count in [0, 1_000, 10_000] {
        let mut project = walk.clone();
        let doc = project.clips[0].drawing_at_mut(0);
        for (i, art) in source.iter().cycle().take(count).enumerate() {
            let mut object = art.clone();
            object.id = doc.next_id;
            doc.next_id += 1;
            object.group = None;
            let tile = i / source.len();
            object.position[0] += 256. + (tile % 32 * 128) as f32;
            object.position[1] += (tile / 32 * 128) as f32;
            doc.objects.push(object);
        }
        assert!(renderer::render(doc, 0).unwrap().data() == rest.data());
        check(count, &project);
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn snap_enabled_transform_responsiveness() {
    for_each_offboard_scene(|count, project| {
        for (panels, layout) in [
            ("Properties", Default::default()),
            (
                "Properties and Layers",
                crate::editor::panel_layout::PanelLayout::stacked(),
            ),
        ] {
            let mut view = Viewport {
                window: EditorWindow::new(egui::vec2(1100., 740.)),
                dpi: 2.,
                time: 0.,
                board: egui::Rect::NOTHING,
            };
            view.window.editor.panel_layout = layout;
            view.window.editor.replace_project(project.clone(), None);
            view.window.editor.style.snap.enabled = true;
            view.frames(5, |_| Vec::new());
            let point = [71., 71.];
            let doc = project.clips[0].drawing_at(0);
            let sleeve = doc.hit_test(point, 0).unwrap();
            assert_eq!(
                doc.objects.iter().find(|o| o.id == sleeve).unwrap().name,
                "Sleeve fabric"
            );
            let center = on_screen(&view, point);
            wheel_zoom(&mut view, center);
            println!("Lantern Walk + {count} off-artboard objects, snapping on, 1100x740 at 2x DPI and 2x Fit, {panels}");
            for _ in 0..3 {
                for edit in [Edit::Move, Edit::Scale] {
                    gesture(&mut view, project, &[sleeve], point, edit);
                }
            }
        }
    });
}
