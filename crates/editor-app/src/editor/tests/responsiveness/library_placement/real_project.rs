use super::*;

// Open a caller-supplied scene so reported placement stalls can be reproduced
// without committing the user's artwork. All pointer edits stay in memory.
fn grab_ink(view: &Viewport, id: u64) -> Option<Pos2> {
    let editor = &view.window.editor;
    let frame = editor.timeline.frame;
    let doc = editor.scene();
    let object = doc.objects.iter().find(|o| o.id == id)?;
    let world = object.local_to_world_at(frame);
    let to_screen = |p: [f32; 2]| {
        view.board.min + view.board.size() * vec2(p[0] / doc.width as f32, p[1] / doc.height as f32)
    };
    for y in [0.35, 0.65, 0.42, 0.58, 0.21, 0.79] {
        for x in [0.35, 0.65, 0.42, 0.58, 0.21, 0.79] {
            let point = world([x, y]);
            let screen = to_screen(point);
            if screen.distance(to_screen(world([0.5; 2]))) > 9.
                && view.board.contains(screen)
                && doc.hit_test(point, frame) == Some(id)
            {
                return Some(screen);
            }
        }
    }
    None
}

fn grab(view: &mut Viewport, id: u64, phase: &str) {
    let Some(start) = grab_ink(view, id) else {
        println!("Skipped placement {id} with no accessible ink during {phase}");
        return;
    };
    let before = view
        .window
        .editor
        .scene()
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap()
        .position_at(view.window.editor.timeline.frame);
    let first = start + vec2(8., 3.);
    let end = first + vec2(4., 1.);
    let hover = view.frame(vec![Event::PointerMoved(start)], 0.5);
    let press = view.frame(vec![button(start, true)], 1. / 60.);
    let movement = view.frame(vec![Event::PointerMoved(first)], 1. / 60.);
    let held = view.frame(vec![Event::PointerMoved(end)], 1. / 60.);
    let release = view.frame(vec![button(end, false)], 1. / 60.);
    let editor = &view.window.editor;
    assert_eq!(editor.selection, [id]);
    let object = editor.scene().objects.iter().find(|o| o.id == id).unwrap();
    assert_ne!(object.position_at(editor.timeline.frame), before);
    println!(
        "GRAB {} {phase} tick={} id={id} hover={:.3} press={:.3} first={:.3} held={:.3} release={:.3} first_updates={} first_bytes={}",
        view.dpi, editor.timeline.frame, hover.ms, press.ms, movement.ms, held.ms,
        release.ms, movement.uploads, movement.bytes
    );
}

fn settle(view: &mut Viewport) {
    for _ in 0..24 {
        view.frame(Vec::new(), 1. / 60.);
        std::thread::sleep(Duration::from_millis(8));
    }
}

#[test]
#[ignore = "set PHYRO_PROFILE_PROJECT to a saved scene containing placed clips"]
fn profile_saved_clip_grabs() {
    let file = std::env::var_os("PHYRO_PROFILE_PROJECT").expect("set PHYRO_PROFILE_PROJECT");
    let project = project_io::load_animation(Path::new(&file)).unwrap();
    for dpi in [1., 2.] {
        let mut view = Viewport {
            window: EditorWindow::new(vec2(1920., 1080.)),
            dpi,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        view.window.editor.replace_project(project.clone(), None);
        view.window.editor.inspector_tab = crate::inspector::Tab::Properties;
        view.frame(Vec::new(), 1. / 60.);
        let ids: Vec<_> = view
            .window
            .editor
            .scene()
            .objects
            .iter()
            .filter(|o| o.clip_instance.is_some())
            .map(|o| o.id)
            .collect();
        assert!(!ids.is_empty(), "choose a scene containing placed clips");
        grab(&mut view, ids[0], "cold");
        settle(&mut view);
        for &id in ids.iter().rev() {
            grab(&mut view, id, "switch");
        }
        settle(&mut view);
        view.window.editor.timeline.frame = (project.clips[0].frames - 1).min(5);
        view.frame(Vec::new(), 1. / 60.);
        grab(&mut view, ids[0], "scrub");
        for id in ids {
            settle(&mut view);
            grab(&mut view, id, "idle");
        }
        let doc = project.clips[0].drawing_at(0);
        let density = view.board.width() * dpi / doc.width as f32;
        for object in &doc.objects {
            if object.clip_instance.is_some() {
                let start = Instant::now();
                if let Some(sprite) = renderer::move_sprite(object, 0, density) {
                    println!(
                        "CAPTURE {dpi} id={} density={density:.2} width={} height={} bytes={} ms={:.3}",
                        object.id, sprite.pixels.width(), sprite.pixels.height(),
                        sprite.pixels.data().len(), start.elapsed().as_secs_f64() * 1000.
                    );
                }
            }
        }
    }
}
