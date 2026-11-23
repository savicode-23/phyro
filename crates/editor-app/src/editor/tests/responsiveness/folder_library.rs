use super::*;
use std::fs;

const ASSETS: usize = 10_000;

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    }
}

fn measure(folder: &Path, show_folder: bool, scrub: bool) {
    let cache = tempfile::tempdir().unwrap();
    let mut view = Viewport {
        window: EditorWindow::new(egui::vec2(1100., 740.)),
        dpi: 1.,
        time: 0.,
        board: egui::Rect::NOTHING,
    };
    let walk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    view.window.editor.open_project(walk);
    view.window.editor.project.clips[0]
        .set_length(1000)
        .unwrap();
    view.window.editor.finish_edit();
    for index in 0..7 {
        view.window
            .editor
            .open_project(folder.join(format!("character-{index:05}.vanim")));
        view.frame(vec![], 1. / 60.);
    }
    assert_eq!(view.window.editor.document_tab_count(), 8);
    view.window.editor.select_document_tab(0);
    view.window.editor.inspector_tab = crate::inspector::Tab::Library;
    view.window
        .editor
        .library
        .set_cache_directory(cache.path().to_owned());
    view.window.editor.set_tool(Tool::Pencil);
    let object_count = view.window.editor.scene().objects.len();
    view.frames(4, |_| vec![]);
    let from = if scrub {
        view.window.control("1")
    } else {
        view.board.center() - egui::vec2(80., 20.)
    };
    let step = if scrub {
        view.window.editor.timeline.frame_width()
    } else {
        1.
    };
    view.window.editor.library.set_directory(folder.to_owned());
    view.window.editor.library.set_folders_active(show_folder);
    let first = view.frame(vec![egui::Event::PointerMoved(from)], 1. / 60.);
    let mut samples = vec![view.frame(vec![button(from, true)], 1. / 60.)];
    let pos = |i: usize| {
        if scrub {
            from + egui::vec2(step * ((i / 2) % 25) as f32, 0.)
        } else {
            from + egui::vec2(i as f32, (i as f32 * 0.07).sin() * 18.)
        }
    };
    samples.extend(view.frames(160, |i| vec![egui::Event::PointerMoved(pos(i))]));
    samples.push(view.frame(vec![button(pos(159), false)], 1. / 60.));
    if scrub {
        assert_eq!(view.window.editor.timeline.frame, (159 / 2) % 25);
    } else {
        assert_eq!(view.window.editor.scene().objects.len(), object_count + 1);
    }
    let panel = if show_folder { "Folders" } else { "Symbols" };
    let action = if scrub { "scrub" } else { "pencil" };
    println!("{panel} · {action} · first panel paint {:.3} ms", first.ms);
    report(&format!("{panel} · {action}"), samples);
    if show_folder {
        let started = Instant::now();
        let mut idle = Vec::new();
        while view.window.editor.library.indexed_assets() < ASSETS
            || view.window.editor.library.resident_previews() < 8
        {
            assert!(
                started.elapsed() < Duration::from_secs(90),
                "browser did not finish indexing and preview visible assets"
            );
            idle.push(view.frame(vec![], 1. / 60.));
            std::thread::sleep(Duration::from_millis(4));
        }
        if !idle.is_empty() {
            report("finishing the cold scan and first thumbnails", idle);
        }
        view.frames(2, |_| Vec::new());
        let resident = view.window.editor.library.resident_previews();
        assert!(
            (1..128).contains(&resident),
            "{resident} previews for one viewport"
        );
        println!(
            "{} assets, {resident} resident previews",
            view.window.editor.library.indexed_assets()
        );
        // Also measure drawing/scrubbing once actual filenames and textures are on screen.
        let start = if scrub {
            from
        } else {
            from + egui::vec2(0., 40.)
        };
        view.frame(vec![egui::Event::PointerMoved(start)], 1. / 60.);
        let mut warm = vec![view.frame(vec![button(start, true)], 1. / 60.)];
        warm.extend(view.frames(160, |i| {
            vec![egui::Event::PointerMoved(pos(i) + (start - from))]
        }));
        warm.push(view.frame(vec![button(pos(159) + (start - from), false)], 1. / 60.));
        report(&format!("Folders populated · {action}"), warm);
    } else {
        assert_eq!(view.window.editor.library.indexed_assets(), 0);
    }
}

#[test]
#[ignore = "release timing benchmark with 10,000 real files and a fresh thumbnail cache"]
fn cold_folder_pencil_and_timeline() {
    println!(
        "Lantern Walk, 1,000 frames, eight open documents, 10,000 folder assets · 1100×740 at 1×"
    );
    let folder = tempfile::tempdir().unwrap();
    let project =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-patrol.vanim"))
            .unwrap();
    for i in 0..ASSETS {
        let path = folder.path().join(format!("character-{i:05}.vanim"));
        // Fifty backing files keep the corpus quick to create on both NTFS and ext4.
        if i % 200 == 0
            || fs::hard_link(
                folder
                    .path()
                    .join(format!("character-{:05}.vanim", i / 200 * 200)),
                &path,
            )
            .is_err()
        {
            fs::write(path, &project).unwrap();
        }
    }
    for (visible, scrub) in [(false, false), (true, false), (true, true), (false, true)] {
        measure(folder.path(), visible, scrub);
    }
}
