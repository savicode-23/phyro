//! Opt-in release benchmark of editor input, painting and egui tessellation.
use super::*;
use std::time::{Duration, Instant};
mod assistant;
mod folder_library;
mod library_placement;
mod long_animation;
mod marquee;
mod recovery;
mod transforms;

struct Viewport {
    window: EditorWindow,
    dpi: f32,
    time: f64,
    board: egui::Rect,
}

struct Sample {
    ms: f64,
    uploads: usize,
    bytes: usize,
}

pub(super) fn is_artwork_texture(name: &str) -> bool {
    matches!(
        name,
        "artboard artwork"
            | "pasteboard artwork"
            | "artboard detail"
            | "transformed artboard"
            | "moving artwork"
            | "artwork above move"
            | "selection transform"
    )
}

impl Viewport {
    fn input(&mut self, events: Vec<egui::Event>, dt: f64) -> egui::RawInput {
        self.time += dt;
        let modifiers = events.iter().rev().find_map(|event| match event {
            egui::Event::PointerButton { modifiers, .. }
            | egui::Event::MouseWheel { modifiers, .. }
            | egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        });
        let mut input = egui::RawInput {
            events,
            modifiers: modifiers.unwrap_or_default(),
            time: Some(self.time),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(self.dpi);
        input
    }

    fn frame(&mut self, events: Vec<egui::Event>, dt: f64) -> Sample {
        let input = self.input(events, dt);
        let start = Instant::now();
        let output = self.window.frame_with_input(input);
        self.board = EditorWindow::board_rect(&output);
        std::hint::black_box(
            self.window
                .ctx
                .tessellate(output.shapes, output.pixels_per_point),
        );
        let ms = start.elapsed().as_secs_f64() * 1000.;
        let textures = self.window.ctx.tex_manager();
        let textures = textures.read();
        let mut sample = Sample {
            ms,
            uploads: 0,
            bytes: 0,
        };
        for (id, delta) in output.textures_delta.set {
            if textures
                .meta(id)
                .is_some_and(|m| is_artwork_texture(&m.name))
            {
                sample.uploads += 1;
                sample.bytes += delta.image.size().into_iter().product::<usize>() * 4;
            }
        }
        sample
    }

    fn frames(
        &mut self,
        count: usize,
        mut events: impl FnMut(usize) -> Vec<egui::Event>,
    ) -> Vec<Sample> {
        (0..count)
            .map(|i| self.frame(events(i), 1. / 60.))
            .collect()
    }

    fn drag(&mut self, start: egui::Pos2, tool: Tool, label: &str) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.window.editor.tool = tool;
        self.frame(vec![egui::Event::PointerMoved(start)], 1. / 60.);
        self.frame(vec![button(start, true)], 1. / 60.);
        let point = |i| start + egui::vec2(2. * (i + 1) as f32, i as f32);
        report(
            label,
            self.frames(12, |i| vec![egui::Event::PointerMoved(point(i))]),
        );
        self.frame(vec![button(point(11), false)], 1. / 60.);
    }
}

fn report(label: &str, samples: Vec<Sample>) {
    let total: f64 = samples.iter().map(|s| s.ms).sum();
    let mut times: Vec<_> = samples.iter().map(|s| s.ms).collect();
    times.sort_by(f64::total_cmp);
    let len = times.len();
    let (p50, p95, worst) = (
        times[len.div_ceil(2) - 1],
        times[(95 * len).div_ceil(100) - 1],
        times[len - 1],
    );
    let count: usize = samples.iter().map(|s| s.uploads).sum();
    let bytes: usize = samples.iter().map(|s| s.bytes).sum();
    println!("{label}: {len} frames, mean {:.3} ms, p50 {p50:.3} ms, p95 {p95:.3} ms, max {worst:.3} ms, {count} artwork texture updates ({:.2} MiB)", total/len as f64, bytes as f64 / 1_048_576.);
}

fn tiled_scene(keeper: &Document, count: usize) -> Document {
    let side = (count.div_ceil(keeper.objects.len()) as f64).sqrt().ceil() as u32;
    let mut doc = keeper.clone();
    doc.width *= side;
    doc.height *= side;
    doc.objects.clear();
    for (i, source) in keeper.objects.iter().cycle().take(count).enumerate() {
        let tile = i as u32 / keeper.objects.len() as u32;
        let mut object = source.clone();
        object.id = doc.next_id;
        doc.next_id += 1;
        object.group = None;
        object.position[0] += (tile % side * keeper.width) as f32;
        object.position[1] += (tile / side * keeper.height) as f32;
        doc.objects.push(object);
    }
    doc
}

fn scenes() -> Vec<(String, AnimationProject)> {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let walk = project_io::load_animation(&examples.join("lantern-walk.vanim")).unwrap();
    let keeper = project_io::load_animation(&examples.join("lantern-keeper.vanim")).unwrap();
    let art = keeper.clips[0].drawing_at(0);
    let mut scenes = vec![("walk".to_owned(), walk)];
    for n in [1_000, 10_000] {
        scenes.push((
            format!("{n} objects"),
            AnimationProject::from_document(tiled_scene(art, n)),
        ));
    }
    scenes
}

#[test]
#[ignore = "release timing benchmark"]
fn viewport_responsiveness_timeline_resize() {
    for (name, project) in scenes() {
        let mut view = Viewport {
            window: EditorWindow::new(egui::vec2(1100., 740.)),
            dpi: 2.,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        view.window.editor.replace_project(project.clone(), None);
        view.frames(5, |_| Vec::new());
        let panel = |v: &Viewport| {
            egui::containers::panel::PanelState::load(&v.window.ctx, egui::Id::new("timeline"))
                .unwrap()
                .rect
        };
        let initial = panel(&view);
        let start = initial.center_top();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        };
        view.frame(vec![egui::Event::PointerMoved(start)], 1. / 60.);
        view.frame(vec![button(start, true)], 1. / 60.);
        let point = |i: usize| start - egui::vec2(0., (i + 1) as f32 * 4.);
        println!("{name}, timeline resize, 1100x740 at 2x DPI");
        let motion = view.frames(20, |i| vec![egui::Event::PointerMoved(point(i))]);
        report("moving divider", motion);
        view.frame(vec![button(point(19), false)], 1. / 60.);
        assert!((panel(&view).height() - initial.height() - 80.).abs() < 1.);
        assert!(view.window.editor.project == project);
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn large_selection_history_responsiveness() {
    let mut drawing = Document::default();
    drawing.add_rectangle([16., 4.], [2., 2.], [219, 31, 97, 255]);
    let shape = drawing.objects[0].clone();
    for i in 1..10_000 {
        let mut object = shape.clone();
        object.id = drawing.next_id;
        drawing.next_id += 1;
        object.position = [16. + (i % 250) as f32 * 4., 4. + (i / 250) as f32 * 5.];
        drawing.objects.push(object);
    }
    let ids: Vec<_> = drawing.objects.iter().map(|o| o.id).collect();
    let before = AnimationProject::from_document(drawing);
    let mut view = Viewport {
        window: EditorWindow::new(egui::vec2(1100., 740.)),
        dpi: 2.,
        time: 0.,
        board: egui::Rect::NOTHING,
    };
    view.window.editor.replace_project(before.clone(), None);
    view.window.editor.selection.clone_from(&ids);
    for o in &mut view.window.editor.project.clips[0]
        .drawing_at_mut(0)
        .objects
    {
        o.position[0] += 9.;
    }
    view.window.editor.finish_edit();
    let after = view.window.editor.project.clone();
    view.frames(3, |_| Vec::new());
    let mut undo = Vec::new();
    let mut redo = Vec::new();
    for _ in 0..5 {
        for (key, expected, samples) in [(Key::Z, &before, &mut undo), (Key::Y, &after, &mut redo)]
        {
            let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
            let event = |pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            };
            // Ctrl stays down through the key event and its release.
            let time = Instant::now();
            let mut input = view.input(vec![event(true)], 1. / 60.);
            input.modifiers = modifiers;
            let out = view.window.frame_with_input(input);
            std::hint::black_box(view.window.ctx.tessellate(out.shapes, out.pixels_per_point));
            samples.push(time.elapsed().as_secs_f64() * 1000.);
            assert!(view.window.editor.project == *expected);
            assert_eq!(view.window.editor.selection, ids);
            view.frame(vec![event(false)], 1. / 60.);
        }
    }
    for (name, samples) in [("Undo", &mut undo), ("Redo", &mut redo)] {
        samples.sort_by(f64::total_cmp);
        println!(
            "10,000 selected objects, {name}: median {:.2} ms, worst {:.2} ms",
            samples[2], samples[4]
        );
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn viewport_responsiveness() {
    for (name, project) in scenes() {
        for (w, h, dpi) in [(800., 500., 1.), (1100., 740., 2.)] {
            println!("\n{name}, {}x{} logical points, {dpi}x", w as u32, h as u32);
            let mut view = Viewport {
                window: EditorWindow::new(egui::vec2(w, h)),
                dpi,
                time: 0.,
                board: egui::Rect::NOTHING,
            };
            view.window.editor.replace_project(project.clone(), None);
            view.frames(4, |_| Vec::new());
            let board = view.board;
            let center = board.center();
            view.frame(vec![egui::Event::PointerMoved(center)], 1. / 60.);
            view.frames(3, |_| Vec::new());
            report(
                "hover",
                view.frames(20, |i| {
                    vec![egui::Event::PointerMoved(center + egui::vec2(i as f32, 0.))]
                }),
            );
            let width = view.window.editor.scene().width as f32;
            // Pick a point on the hood in a tile near the center of the board.
            let tile = ((width / 128.) as u32 / 2) * 128;
            let hit = board.min
                + board.size()
                    * egui::vec2((tile as f32 + 55.) / width, (tile as f32 + 41.) / width);
            let id = view
                .window
                .editor
                .scene()
                .hit_test([tile as f32 + 55., tile as f32 + 41.], 0)
                .expect("hood on an editable layer");
            let position = |editor: &Editor| {
                editor
                    .scene()
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .unwrap()
                    .position_at(0)
            };
            let from = position(&view.window.editor);
            view.drag(hit, Tool::Select, "move");
            assert_ne!(
                position(&view.window.editor),
                from,
                "selection drag must move the picked artwork"
            );
            let count = view.window.editor.scene().objects.len();
            view.drag(center, Tool::Rectangle, "draw");
            assert_eq!(view.window.editor.scene().objects.len(), count + 1);
            view.frame(vec![egui::Event::PointerMoved(center)], 1. / 60.);
            report(
                "zoom in",
                view.frames(20, |_| {
                    vec![egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., 32.),
                        modifiers: egui::Modifiers::NONE,
                    }]
                }),
            );
            // Use real time here. Smooth wheel input decays and the background
            // rasterizer delivers the final texture on a subsequent frame.
            let started = Instant::now();
            let mut previous = started;
            let mut last_upload = None;
            let mut samples = Vec::new();
            while started.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(8));
                let now = Instant::now();
                let sample = view.frame(Vec::new(), (now - previous).as_secs_f64());
                previous = now;
                if sample.uploads > 0 {
                    last_upload = Some(started.elapsed());
                }
                samples.push(sample);
                if last_upload.is_some() && started.elapsed() > Duration::from_millis(600) {
                    break;
                }
            }
            report("settle", samples);
            println!(
                "Last observed artboard upload {:.1} ms after the last wheel event",
                last_upload.expect("settled zoom texture").as_secs_f64() * 1000.
            );
        }
    }
}
