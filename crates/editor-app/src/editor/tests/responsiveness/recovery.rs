use super::*;
use std::sync::Arc;

#[test]
#[ignore = "release timing benchmark"]
fn draft_checkpoint_during_timeline_scrub_and_artwork_drag() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lantern-walk.vanim");
    let mut project = project_io::load_animation(&example).unwrap();
    let clip = &mut project.clips[0];
    clip.set_length(1000).unwrap();
    let drawing = clip.drawing_at_mut(0);
    let layer = commands::add_layer(drawing);
    let mark = drawing.add_shape(
        [3.; 2],
        [16.; 2],
        [255; 4],
        document_core::Shape::Rectangle,
        layer,
    );
    let template = drawing.clone();
    for tick in 1..1000 {
        let mut drawing = template.clone();
        drawing.objects.last_mut().unwrap().fill[0] = (tick % 256) as u8;
        clip.cels.insert(tick, Arc::new(drawing));
    }
    for scrub in [false, true] {
        for checkpoint in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut view = Viewport {
                window: EditorWindow::new(egui::vec2(1100., 740.)),
                time: 0.,
                dpi: 1.,
                board: egui::Rect::NOTHING,
            };
            view.window
                .editor
                .enable_draft_recovery(dir.path().to_owned());
            view.window
                .editor
                .replace_project(project.clone(), Some(example.clone()));
            view.window.editor.tool = Tool::Select;
            view.frames(5, |_| vec![]);
            let start = if scrub {
                view.window.control("1")
            } else {
                view.board.min
                    + view.board.size()
                        * egui::vec2(10. / template.width as f32, 10. / template.height as f32)
            };
            let press = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            view.frame(
                vec![egui::Event::PointerMoved(start), press(start, true)],
                1. / 60.,
            );
            if !scrub {
                assert!(view.window.editor.selection.contains(&mark));
            }
            let writer =
                |view: &Viewport| view.window.editor.drafts.as_ref().unwrap().writer.writing();
            if checkpoint {
                let editor = &mut view.window.editor;
                editor
                    .drafts
                    .as_mut()
                    .unwrap()
                    .writer
                    .checkpoint(editor.path.clone(), editor.baseline.clone())
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !writer(&view) {
                    assert!(Instant::now() < deadline, "checkpoint did not start");
                    std::thread::yield_now();
                }
            }
            let mut samples = Vec::new();
            let mut pos = start;
            let mut last_frame_seen = 0;
            for tick in 1..=96 {
                let offset = if scrub {
                    (tick % 12) as f32 * 18.
                } else {
                    (tick % 36) as f32
                };
                pos = start + egui::vec2(offset, 0.);
                let sample = view.frame(vec![egui::Event::PointerMoved(pos)], 1. / 60.);
                if checkpoint && !writer(&view) {
                    break;
                }
                last_frame_seen = last_frame_seen.max(view.window.editor.timeline.frame);
                samples.push(sample);
            }
            assert!(samples.len() >= 5);
            view.frame(vec![press(pos, false)], 1. / 60.);
            if scrub {
                assert!(
                    last_frame_seen > 0,
                    "the pointer must scrub the frame ruler"
                );
            } else {
                assert!(
                    view.window.editor.can_undo(),
                    "the drag must move the selected artwork"
                );
            }
            if checkpoint {
                view.window
                    .editor
                    .drafts
                    .as_ref()
                    .unwrap()
                    .writer
                    .wait_written()
                    .unwrap();
            }
            let interaction = if scrub { "Scrub" } else { "Drag" };
            let state = if checkpoint {
                "checkpoint writing"
            } else {
                "checkpoint idle"
            };
            report(
                &format!("{interaction}, 1,000 Lantern Walk drawings, {state}"),
                samples,
            );
        }
    }
}
