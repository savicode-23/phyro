use super::*;
use std::time::{Duration, Instant};
mod duplicates;
mod transforms;

pub(super) fn wait_for(window: &mut EditorWindow, ready: impl Fn(&EditorWindow) -> bool) {
    let start = Instant::now();
    while !ready(window) {
        window.frame(vec![]);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{}",
            window.editor.status
        );
        std::thread::sleep(Duration::from_millis(4));
    }
}

fn filename_tile(window: &mut EditorWindow) -> egui::Pos2 {
    window
        .frame(vec![])
        .shapes
        .iter()
        .find_map(|s| {
            if let egui::Shape::Text(t) = &s.shape {
                let point = t.galley.rect.translate(t.pos.to_vec2()).center();
                if t.galley.text() == "fox.vanim" && s.clip_rect.contains(point) {
                    return Some(point);
                }
            }
            None
        })
        .expect("visible library file")
}

fn has_label(output: &egui::FullOutput, label: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == label))
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_library_clip_drops_into_the_scene_and_the_folder_survives_opening_its_source() {
    let assets = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = assets.path().join("fox.vanim");
    let mut source = AnimationProject::from_document(Document {
        width: 160,
        height: 128,
        ..Default::default()
    });
    source.clips[0].name = "Idle".into();
    source.clips[0].set_length(2).unwrap();
    source.clips[0].set_fps(24).unwrap();
    let run = source.add_clip("Run").unwrap();
    source.clips[run].set_length(12).unwrap();
    source.clips[run]
        .drawing_at_mut(0)
        .add_rectangle([24., 16.], [60., 48.], [240, 100, 35, 255]);
    project_io::save_animation(&path, &source, false).unwrap();

    let mut window = EditorWindow::new(egui::vec2(1250., 820.));
    window
        .editor
        .library
        .set_cache_directory(cache.path().to_owned());
    window
        .editor
        .library
        .set_directory(assets.path().to_owned());
    window.click("Library");
    window.click("Folders");
    wait_for(&mut window, |w| w.editor.library.indexed_assets() == 1);
    window.click("fox.vanim");
    // Native metadata and the first thumbnail can arrive on different frames.
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let output = window.frame(vec![]);
        if output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Run"))
        {
            break;
        }
        assert!(Instant::now() < until, "missing Run clip selector");
        std::thread::sleep(Duration::from_millis(4));
    }
    window.click("Run");
    window.click("Play");
    wait_for(&mut window, |w| {
        w.editor
            .library
            .playing_preview_frame()
            .is_some_and(|frame| frame > 0)
    });
    assert_eq!(
        window.editor.timeline.frame, 0,
        "asset preview must have its own playhead"
    );
    window.click("Pause");
    assert_eq!(window.editor.library.playing_preview_frame(), None);
    window.click("Play");
    window.click("Idle");
    assert_eq!(window.editor.library.playing_preview_frame(), None);
    window.click("Play");
    wait_for(&mut window, |w| {
        w.editor.library.playing_preview_frame() == Some(1)
    });
    window.click("Run");
    window.click("Play");
    window.click("Symbols");
    assert_eq!(window.editor.library.playing_preview_frame(), None);
    window.click("Folders");
    window.control("Play");
    assert_eq!(window.editor.library.playing_preview_frame(), None);
    let initial = window.editor.project.clone();
    let destination = window.board().center() + egui::vec2(37., 22.);
    let start = filename_tile(&mut window);
    window.frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
    let outside = window.frame(vec![egui::Event::PointerMoved(start - egui::vec2(13., 0.))]);
    assert!(has_label(&outside, "Drag onto canvas"));
    let held = window.frame(vec![egui::Event::PointerMoved(destination)]);
    assert!(has_label(&held, "Release to place"));
    assert!(has_label(&held, "fox.vanim · Run"));
    assert_eq!(held.platform_output.cursor_icon, egui::CursorIcon::Copy);
    let payload = egui::DragAndDrop::payload::<crate::library::AssetDrop>(&window.ctx).unwrap();
    let texture = payload.thumbnail.as_ref().expect("cached Run preview");
    assert!(held
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Mesh(m)
        if m.texture_id == texture.id() && m.calc_bounds().center().distance(destination) < 80.)));
    assert!(held
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Circle(c)
        if c.center == destination && c.stroke.color == crate::theme::MINT)));
    assert_eq!(window.editor.project, initial);
    let released = window.frame(vec![button(destination, false)]);
    assert!(!has_label(&released, "Release to place"));
    wait_for(&mut window, |w| !w.editor.scene().objects.is_empty());
    assert_eq!(window.editor.scene().objects.len(), 1);
    let child = window.editor.scene().objects[0]
        .clip_instance
        .as_ref()
        .unwrap()
        .clip_id;
    let imported = window
        .editor
        .project
        .clips
        .iter()
        .find(|c| c.id == child)
        .unwrap();
    assert_eq!(imported.frames, 12);
    assert_eq!(
        window.editor.selection,
        [window.editor.scene().objects[0].id]
    );
    transforms::check_placed_clip(&mut window);
    window.click_edit("Undo");
    assert_eq!(window.editor.project, initial);

    // A browser double-click uses normal open semantics. Opening its source keeps
    // the library location and must not create a second nested placement.
    let later = window.ctx.input(|i| i.time) + 1.;
    window.frame_with_input(egui::RawInput {
        time: Some(later),
        ..Default::default()
    });
    let start = filename_tile(&mut window);
    window.click_at(start);
    window.click_at(start);
    wait_for(&mut window, |w| w.editor.path.as_ref() == Some(&path));
    assert_eq!(window.editor.project, source);
    assert_eq!(window.editor.library.directory(), Some(assets.path()));
    assert!(window.editor.library.folders_active());
}

#[test]
fn locked_layer_and_cancelled_asset_drags_leave_the_drawing_untouched() {
    let mut window = EditorWindow::new(egui::vec2(850., 550.));
    let at = window.board().center();
    window.editor.project.clips[0].drawing_at_mut(0).layers[0].locked = true;
    let original = window.editor.project.clone();
    window.frame(vec![egui::Event::PointerMoved(at), button(at, true)]);
    egui::DragAndDrop::set_payload(
        &window.ctx,
        crate::library::AssetDrop {
            source: crate::library::AssetSource::File {
                path: "uncached.svg".into(),
                clip_id: None,
            },
            label: "uncached.svg".into(),
            thumbnail: None,
        },
    );
    let held = window.frame(vec![]);
    assert!(has_label(&held, "uncached.svg"));
    assert!(has_label(&held, "Choose a visible, unlocked layer"));
    assert_eq!(
        held.platform_output.cursor_icon,
        egui::CursorIcon::NotAllowed
    );
    let cancelled = window.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(!has_label(&cancelled, "uncached.svg"));
    window.frame(vec![button(at, false)]);
    assert!(!window.editor.folder_assets.is_pending());
    assert_eq!(window.editor.project, original);
}
