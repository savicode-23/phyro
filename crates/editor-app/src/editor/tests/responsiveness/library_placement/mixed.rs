use super::handles::{drag_handle, Handle};
use super::*;

fn press_duplicate(view: &mut Viewport) -> u64 {
    let before = view.window.editor.scene().objects.len();
    let key = |pressed| Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed,
        repeat: false,
        modifiers: Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        },
    };
    report("Ctrl+D", vec![view.frame(vec![key(true)], 1. / 60.)]);
    report(
        "First duplicate paint",
        vec![view.frame(vec![key(false)], 1. / 60.)],
    );
    let editor = &view.window.editor;
    assert_eq!(editor.scene().objects.len(), before + 1);
    assert_eq!(editor.selection.len(), 1);
    editor.selection[0]
}

fn body(view: &Viewport, id: u64) -> Pos2 {
    let doc = view.window.editor.scene();
    let object = doc.objects.iter().find(|o| o.id == id).unwrap();
    let to_screen =
        |world: [f32; 2]| view.board.min + view.board.size() * egui::Vec2::from(world) / 1000.;
    let world = object.local_to_world_at(0);
    let center = to_screen(world([0.5; 2]));
    let portion = [0.34, 0.66, 0.40, 0.60, 0.24, 0.76, 0.17, 0.83, 0.50];
    for y in portion {
        for x in portion {
            let p = world([x, y]);
            let screen = to_screen(p);
            if screen.distance(center) > 8. && doc.hit_test(p, 0) == Some(id) {
                return screen;
            }
        }
    }
    panic!("no unobscured body for object {id}");
}

fn select(view: &mut Viewport, id: u64) {
    let start = body(view, id);
    view.frame(vec![Event::PointerMoved(start)], 0.5);
    report(
        "Select press",
        vec![view.frame(vec![button(start, true)], 1. / 60.)],
    );
    report(
        "Select release",
        vec![view.frame(vec![button(start, false)], 1. / 60.)],
    );
    assert_eq!(view.window.editor.selection, [id]);
}

fn move_ink(view: &mut Viewport, id: u64, delta: egui::Vec2, steps: usize) {
    let before = view
        .window
        .editor
        .scene()
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap()
        .position_at(0);
    let start = body(view, id);
    let point = |i: usize| start + delta * (0.30 + 0.70 * i as f32 / steps as f32);
    view.frame(vec![Event::PointerMoved(start)], 0.5);
    report(
        "Move press",
        vec![view.frame(vec![button(start, true)], 1. / 60.)],
    );
    report(
        "First move",
        vec![view.frame(vec![Event::PointerMoved(point(0))], 1. / 60.)],
    );
    report(
        "Held move",
        view.frames(steps, |i| vec![Event::PointerMoved(point(i + 1))]),
    );
    report(
        "Move release",
        vec![view.frame(vec![button(point(steps), false)], 1. / 60.)],
    );
    report("First moved paint", vec![view.frame(Vec::new(), 1. / 60.)]);
    let editor = &view.window.editor;
    assert_eq!(editor.selection, [id]);
    let after = editor
        .scene()
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap()
        .position_at(0);
    assert!(
        Pos2::from(before).distance(after.into()) > 7.,
        "the gesture must move the selected clip"
    );
}

#[test]
#[ignore = "release timing benchmark for switching between nine overlapping native clip instances"]
fn nine_library_clip_instances() {
    let fixtures = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let names = ["walk", "elbow", "patrol"];
    let files = names.map(|name| {
        let filename = format!("lantern-{name}.vanim");
        let file = asset_file(fixtures.path(), name, &filename);
        fs::copy(examples.join(filename), &file).unwrap();
        file
    });
    for dpi in [1., 2.] {
        let mut view = Viewport {
            window: EditorWindow::new(vec2(1100., 740.)),
            dpi,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        let page = Document {
            width: 1000,
            height: 1000,
            ..Default::default()
        };
        view.window
            .editor
            .replace_project(AnimationProject::from_document(page), None);
        let mut ids = [[0u64; 3]; 3];
        for (i, file) in files.iter().enumerate() {
            println!("\n{dpi}× drop {}", names[i]);
            ids[i][0] = drop_asset(
                &mut view,
                file,
                cache.path(),
                [0.34 + 0.074 * i as f32, 0.34 + 0.018 * i as f32],
            );
        }
        for (i, family) in ids.iter_mut().enumerate() {
            select(&mut view, family[0]);
            for (copy_index, slot) in family.iter_mut().enumerate().skip(1) {
                println!("\n{dpi}× duplicate {} {copy_index}", names[i]);
                *slot = press_duplicate(&mut view);
                let delta = view.board.size() * vec2(0.015, 0.085);
                move_ink(&mut view, *slot, delta, 12);
            }
        }
        assert_eq!(view.window.editor.scene().objects.len(), 9);
        if dpi == 1. {
            let saved = fixtures.path().join("nine-lanterns.vanim");
            project_io::save_animation(&saved, &view.window.editor.project, false).unwrap();
            let mut cold = Viewport {
                window: EditorWindow::new(vec2(1100., 740.)),
                dpi,
                time: 0.,
                board: egui::Rect::NOTHING,
            };
            cold.window.editor.open_project(saved);
            assert_eq!(cold.window.editor.scene().objects.len(), 9);
            println!("\n{dpi}× reopened elbow 0");
            report(
                "First reopened paint",
                vec![cold.frame(Vec::new(), 1. / 60.)],
            );
            let delta = cold.board.size() * vec2(0.031, 0.008);
            move_ink(&mut cold, ids[1][0], delta, 30);
        }
        // Alternate between foreground/background and originals/both generations of copies.
        for (i, generation) in [
            (2, 2),
            (0, 0),
            (1, 1),
            (0, 2),
            (2, 0),
            (1, 2),
            (0, 1),
            (2, 1),
            (1, 0),
        ] {
            println!("\n{dpi}× switch {} {generation}", names[i]);
            let id = ids[i][generation];
            // Cover both clicking to select before the drag and grabbing unselected ink directly.
            if generation == 1 {
                select(&mut view, id);
            }
            let delta = view.board.size() * vec2(0.031, 0.008);
            move_ink(&mut view, id, delta, 30);
        }
        for (i, generation) in [(0, 0), (1, 1), (2, 2)] {
            let id = ids[i][generation];
            for handle in [Handle::Scale, Handle::Rotate] {
                println!("\n{dpi}× transform {} {generation}", names[i]);
                select(&mut view, id);
                drag_handle(&mut view, id, handle, 30);
            }
        }
    }
}
