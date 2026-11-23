use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Handle {
    Scale,
    Rotate,
}

fn measure_handle(
    file: &Path,
    cache: &Path,
    dpi: f32,
    destination: &AnimationProject,
    handle: Handle,
) {
    let mut view = place(file, cache, dpi, destination, false);
    let id = view.window.editor.selection[0];
    drag_handle(&mut view, id, handle, 90);
    view.window.editor.restore(false);
    assert_eq!(&view.window.editor.project, destination);
}

pub(super) fn drag_handle(view: &mut Viewport, id: u64, handle: Handle, steps: usize) {
    let original = view.window.editor.project.clone();
    let object = view
        .window
        .editor
        .scene()
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap();
    let width = object.size[0] * object.scale_at(0)[0];
    let rotation = object.rotation_at(0);
    let screen = |local| {
        let [x, y] = object.local_to_world(local, 0);
        view.board.min + view.board.size() * vec2(x, y) / 1000.
    };
    let center = screen([0.5, 0.5]);
    let start = match handle {
        Handle::Scale => screen([1., 1.]),
        // The rotation grip sits 18 logical screen points above the top edge.
        Handle::Rotate => screen([0.5, 0.]) - vec2(0., 18.),
    };
    let offset = start - center;
    let point = |step: usize| {
        let t = step as f32 / steps as f32;
        match handle {
            Handle::Scale => start - offset * vec2(0.16 + 0.65 * t, 0.13 + 0.48 * t),
            Handle::Rotate => {
                let (sin, cos) = (0.18 + 1.30 * t).sin_cos();
                center
                    + vec2(
                        cos * offset.x - sin * offset.y,
                        sin * offset.x + cos * offset.y,
                    )
            }
        }
    };
    println!("{handle:?} handle");
    view.frame(vec![Event::PointerMoved(start)], 1. / 60.);
    report(
        "Handle press",
        vec![view.frame(vec![button(start, true)], 1. / 60.)],
    );
    report(
        "First handle move",
        vec![view.frame(vec![Event::PointerMoved(point(0))], 1. / 60.)],
    );
    report(
        "Held handle",
        view.frames(steps, |i| vec![Event::PointerMoved(point(i + 1))]),
    );
    report(
        "Handle release",
        vec![view.frame(vec![button(point(steps), false)], 1. / 60.)],
    );
    report(
        "First paint after release",
        vec![view.frame(Vec::new(), 1. / 60.)],
    );

    let edited = view
        .window
        .editor
        .scene()
        .objects
        .iter()
        .find(|o| o.id == id)
        .unwrap();
    let new_width = edited.size[0] * edited.scale_at(0)[0];
    match handle {
        Handle::Scale => {
            assert!(new_width < width * 0.8, "scale grip must shrink the clip");
            assert_eq!(edited.rotation_at(0), rotation);
        }
        Handle::Rotate => {
            assert!(
                (edited.rotation_at(0) - rotation - 85.).abs() < 4.,
                "rotation grip must turn the clip"
            );
            assert_eq!(new_width, width);
        }
    }
    view.window.editor.restore(false);
    assert_eq!(view.window.editor.project, original);
}

#[test]
#[ignore = "release timing benchmark of scaling and rotating freshly placed native clips"]
fn placed_library_clip_handles() {
    let fixtures = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let keeper = project_io::load_animation(&examples.join("lantern-keeper.vanim")).unwrap();
    let art = keeper.clips[0].drawing_at(0);
    let destination = layered_destination(art);
    for file in clip_fixtures(fixtures.path(), &examples, art) {
        for dpi in [1., 2.] {
            for handle in [Handle::Scale, Handle::Rotate] {
                measure_handle(&file, cache.path(), dpi, &destination, handle);
            }
        }
    }
}
