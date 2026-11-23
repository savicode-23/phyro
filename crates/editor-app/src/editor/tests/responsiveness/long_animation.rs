use super::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum Drawings {
    Held,
    Keyed,
    Independent,
}

fn project(frames: u32, kind: Drawings) -> AnimationProject {
    let mut drawing = Document {
        frames,
        fps: 24,
        ..Default::default()
    };
    for i in 0..32 {
        drawing.add_rectangle(
            [5. + (i % 8) as f32 * 14., 5. + (i / 8) as f32 * 14.],
            [8.; 2],
            [55, i + 20, 182, 255],
        );
    }
    if matches!(kind, Drawings::Keyed) {
        drawing.objects[0].keys = (0..frames)
            .map(|t| (t, [5. + (t % 8) as f32, 5.]))
            .collect();
    }
    let mut project = AnimationProject::from_document(drawing);
    if matches!(kind, Drawings::Independent) {
        let clip = &mut project.clips[0];
        let template = Arc::clone(&clip.cels[&0]);
        clip.cels = (0..frames)
            .map(|t| {
                let mut cel = (*template).clone();
                cel.objects[0].fill[0] = (t % 256) as u8;
                (t, Arc::new(cel))
            })
            .collect();
    }
    project
}

fn paint(window: &mut EditorWindow, tick: u32) -> Duration {
    window.editor.timeline.frame = tick;
    let start = Instant::now();
    let output = window.frame(vec![]);
    std::hint::black_box(
        window
            .ctx
            .tessellate(output.shapes, output.pixels_per_point),
    );
    start.elapsed()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "release timing benchmark"]
fn onion_skin_stepping() {
    // A held pose sheet with 16 different drawings on the same property tracks.
    // Each step must look past held frames for four distinct poses on either side.
    let mut reference = project(1000, Drawings::Held);
    let doc = reference.clips[0].drawing_at_mut(0);
    for (i, object) in doc.objects.iter_mut().enumerate() {
        object.interpolation = document_core::Interpolation::Hold;
        for pose in 0..16 {
            object
                .animated
                .fill_keys
                .insert(pose * 64, [(pose * 13) as u8, i as u8, 140, 255]);
        }
    }
    for (panels, layout) in [
        ("Properties", Default::default()),
        (
            "Properties and Layers",
            crate::editor::panel_layout::PanelLayout::stacked(),
        ),
    ] {
        let mut view = EditorWindow::new(egui::vec2(1100., 740.));
        view.editor.panel_layout = layout;
        view.editor.replace_project(reference.clone(), None);
        view.editor.timeline.onion = true;
        view.editor.timeline.previous_onions = 4;
        view.editor.timeline.next_onions = 4;
        for tick in 410..414 {
            paint(&mut view, tick);
        }
        let mut samples = Vec::new();
        for tick in 415..455 {
            samples.push(paint(&mut view, tick));
        }
        samples.sort_unstable();
        println!("1,000-frame shot · 32 animated objects · 4 previous + 4 following ghost poses · {panels}: frame step p50 {:.2?}, p95 {:.2?}",samples[20],samples[37]);
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn long_animation_authoring() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("long-animation.vanim");
    for frames in [24, 100, 1_000] {
        for kind in [Drawings::Held, Drawings::Keyed, Drawings::Independent] {
            let mut window = EditorWindow::new(egui::vec2(1100., 740.));
            let reference = project(frames, kind);
            window.editor.replace_project(reference.clone(), None);
            for _ in 0..4 {
                paint(&mut window, 0);
            }
            let first = median((0..11).map(|_| paint(&mut window, 0)).collect());
            let sweep = median((0..frames).map(|t| paint(&mut window, t)).collect());
            let last = frames - 1;
            let idle = median((0..11).map(|_| paint(&mut window, last)).collect());
            let editor = &mut window.editor;
            let detached = editor.project.clips[0]
                .cels
                .iter()
                .filter(|(t, cel)| !Arc::ptr_eq(cel, &editor.baseline.clips[0].cels[t]))
                .count();
            assert!(detached <= 1);
            let mut edits = Vec::new();
            let mut undos = Vec::new();
            for _ in 0..7 {
                let start = Instant::now();
                editor.project.clips[0].drawing_at_mut(last).objects[0].position[1] += 1.;
                editor.finish_edit();
                edits.push(start.elapsed());
                let start = Instant::now();
                editor.restore(false);
                undos.push(start.elapsed());
                assert_eq!(editor.project, reference);
            }
            let start = Instant::now();
            project_io::save_animation(&path, &editor.project, true).unwrap();
            let save = start.elapsed();
            let start = Instant::now();
            let reopened = project_io::load_animation(&path).unwrap();
            let open = start.elapsed();
            assert_eq!(reopened, reference);
            println!("{kind:?} {frames} frames, median CPU times: initial {first:.2?}, sweep {sweep:.2?}, final {idle:.2?}, edit {:.2?}, undo {:.2?}; save {save:.2?}, open {open:.2?}", median(edits), median(undos));
        }
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn named_layer_timeline_scroll_and_step() {
    for layers in [10, 1000] {
        let mut scene = project(1000, Drawings::Held);
        let clip = &mut scene.clips[0];
        let drawing = clip.drawing_at_mut(0);
        for i in 2..=layers {
            drawing.layers.push(document_core::Layer {
                id: i,
                name: format!("Artwork {i}"),
                ..Default::default()
            });
        }
        for (i, o) in drawing.objects.iter_mut().enumerate() {
            o.layer = layers - i as u64 % layers;
        }
        let snapshot = clip.cels[&0].clone();
        for t in 1..1000 {
            clip.cels.insert(t, snapshot.clone());
            clip.layer_starts
                .insert(t, [1 + u64::from(t) % layers].into());
        }
        let mut view = Viewport {
            window: EditorWindow::new(egui::vec2(1100., 740.)),
            dpi: 1.,
            time: 0.,
            board: egui::Rect::NOTHING,
        };
        view.window.editor.replace_project(scene, None);
        view.window.editor.selection = vec![snapshot.objects[0].id];
        view.frames(12, |_| Vec::new());
        let row = view.window.control(&format!("Artwork {layers}"));
        let x = view.window.control("1").x;
        let pointer = egui::pos2(x + 10., row.y);
        view.frame(vec![egui::Event::PointerMoved(pointer)], 1. / 60.);
        println!(
            "1000-frame timeline, {layers} layers, 999 independent layer starts, 1100x740 at 1x"
        );
        report("idle", view.frames(60, |_| Vec::new()));
        for (label, delta) in [
            ("vertical scroll", egui::vec2(0., -22.)),
            ("horizontal scroll", egui::vec2(-36., 0.)),
        ] {
            report(
                label,
                view.frames(60, |_| {
                    vec![egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta,
                        modifiers: egui::Modifiers::NONE,
                    }]
                }),
            );
        }
        let mut frames = Vec::new();
        for i in 400..460 {
            view.window.editor.timeline.frame = i;
            frames.push(view.frame(Vec::new(), 1. / 60.));
        }
        report("frame changes", frames);
    }
}

// Measure the actual dock, canvas and inspector together, including egui tessellation
// and texture submissions. Independent drawings exercise cold thumbnail requests as
// each previously unseen part of the ruler scrolls into view.
#[test]
#[ignore = "release timing benchmark"]
fn timeline_scroll_and_scrub() {
    for (frames, width) in [(24, 18), (1_000, 18), (1_000, 3)] {
        for kind in [Drawings::Held, Drawings::Keyed, Drawings::Independent] {
            let mut scene = project(frames, kind);
            scene.clips[0].events = (0..frames)
                .step_by(8)
                .map(|tick| (tick, vec!["Footstep".to_owned()]))
                .collect();
            let id = scene.clips[0].drawing_at(0).objects[0].id;
            let mut view = Viewport {
                window: EditorWindow::new(egui::vec2(1100., 740.)),
                dpi: 1.,
                time: 0.,
                board: egui::Rect::NOTHING,
            };
            view.window.editor.replace_project(scene, None);
            view.window.editor.selection = vec![id];
            view.frames(4, |_| Vec::new());
            if width == 3 {
                view.window.click("100%");
                view.window.frame(vec![egui::Event::Text("5".into())]);
                for pressed in [true, false] {
                    view.window.frame(vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                }
                view.window.control("5%");
            }
            let ruler_y = view.window.control("1").y;
            let visible_numbers = |window: &mut EditorWindow| {
                window
                    .frame(Vec::new())
                    .shapes
                    .into_iter()
                    .filter_map(|s| {
                        let egui::Shape::Text(text) = s.shape else {
                            return None;
                        };
                        let bounds = text.galley.rect.translate(text.pos.to_vec2());
                        if (bounds.center().y - ruler_y).abs() > 1.
                            || !s.clip_rect.contains_rect(bounds)
                        {
                            return None;
                        }
                        let number = text.galley.text().parse::<u32>().ok()?;
                        Some((number, bounds.center()))
                    })
                    .collect::<Vec<_>>()
            };
            let mut numbers = visible_numbers(&mut view.window);
            numbers.sort_by_key(|(number, _)| *number);
            assert_eq!(numbers[0].0, 1);
            let mid = numbers[numbers.len() / 2].1;
            println!(
                "{kind:?}, {frames} frames at {width} pt, 32 shapes per drawing, 1100×740 at 1×"
            );
            view.frame(vec![egui::Event::PointerMoved(mid)], 1. / 60.);
            if frames == 1_000 {
                let samples = view.frames(120, |_| {
                    vec![egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(-2. * width as f32, 0.),
                        modifiers: egui::Modifiers::NONE,
                    }]
                });
                report("horizontal scroll", samples);
                view.frames(30, |_| Vec::new());
                numbers = visible_numbers(&mut view.window);
                numbers.sort_by_key(|(number, _)| *number);
                assert!(
                    numbers[0].0 > 100 && numbers.last().unwrap().0 < frames,
                    "wheel should traverse the ruler without reaching its end"
                );
            }
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let start = numbers[0].1;
            view.frame(
                vec![egui::Event::PointerMoved(start), button(start, true)],
                1. / 60.,
            );
            let samples = view.frames(96, |i| {
                vec![egui::Event::PointerMoved(numbers[i % numbers.len()].1)]
            });
            let (last, pos) = numbers[95 % numbers.len()];
            view.frame(vec![button(pos, false)], 1. / 60.);
            assert_eq!(view.window.editor.timeline.frame + 1, last);
            assert!(!view.window.editor.can_undo());
            report("ruler scrub", samples);
        }
    }
}
