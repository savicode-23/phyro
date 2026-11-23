use super::*;
mod keyboard;

pub(super) fn shift_click(window: &mut EditorWindow, point: egui::Pos2) {
    window.frame(vec![egui::Event::PointerMoved(point)]);
    for pressed in [true, false] {
        window.frame_with_input(egui::RawInput {
            modifiers: egui::Modifiers::SHIFT,
            events: vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::SHIFT,
            }],
            ..Default::default()
        });
    }
}

#[test]
fn inspector_difference_selects_the_editable_result_and_undo_restores_the_operands() {
    let mut drawing = Document::default();
    let base = drawing.add_shape(
        [20.; 2],
        [72., 62.],
        [41, 86, 131, 255],
        document_core::Shape::Ellipse,
        1,
    );
    let cutter = drawing.add_shape(
        [40., 36.],
        [18., 22.],
        [244, 198, 108, 255],
        document_core::Shape::Ellipse,
        1,
    );
    let mut window = EditorWindow::new(egui::vec2(1100., 1000.));
    window.ctx.style_mut(|style| style.animation_time = 0.);
    window
        .editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    window.editor.set_tool(Tool::Select);
    let board = window.board();
    let on_board = |x: f32, y: f32| board.min + board.size() * egui::vec2(x / 128., y / 128.);
    window.click_at(on_board(72., 50.));
    shift_click(&mut window, on_board(49., 47.));
    assert_eq!(window.editor.selection, [base, cutter]);
    window.click("Path operations");
    window.click("Difference");
    assert_eq!(window.editor.selection, [base]);
    let result = window.editor.scene().clone();
    assert_eq!(result.objects.len(), 1);
    assert_eq!(result.objects[0].shape.contours().len(), 2);
    assert_eq!(result.hit_test([49., 47.], 0), None);
    assert_eq!(result.hit_test([72., 50.], 0), Some(base));
    window.click_edit("Undo");
    assert_eq!(window.editor.scene(), &drawing);
    assert!(!window.editor.can_undo());
    window.click_edit("Redo");
    assert_eq!(window.editor.scene(), &result);
}

fn successive_cuts(extra_objects: usize, on_artboard: bool) -> [f64; 2] {
    let mut drawing = Document::default();
    if on_artboard {
        drawing.width = 256;
        drawing.height = 256;
    }
    let base = drawing.add_rectangle([14., 18.], [92., 80.], [41, 86, 131, 255]);
    for at in [[36., 42.], [72., 54.]] {
        drawing.add_shape(at, [16., 18.], [210; 4], document_core::Shape::Ellipse, 1);
    }
    let mut extra = drawing.objects[0].clone();
    extra.position = [400., 400.];
    for index in 0..extra_objects {
        if on_artboard {
            extra.position = [142. + (index % 100) as f32, 136. + (index / 100) as f32];
            extra.size = [1.7, 1.4];
            extra.shape = document_core::Shape::Ellipse;
        }
        extra.id = drawing.next_id;
        drawing.next_id += 1;
        drawing.objects.push(extra.clone());
    }
    let mut window = EditorWindow::new(egui::vec2(1000., 700.));
    window.ctx.style_mut(|style| style.animation_time = 0.);
    window
        .editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    let at = |w: &mut EditorWindow, x: f32, y: f32| {
        w.frame(Vec::new());
        w.frame(Vec::new());
        let board = w.board();
        let scene = w.editor.scene();
        board.min + board.size() * egui::vec2(x / scene.width as f32, y / scene.height as f32)
    };
    let start = at(&mut window, 25., 32.);
    window.click_at(start);
    let mut timings = [0.; 2];
    let mut prior = Vec::new();
    for (index, (x, y)) in [(44., 51.), (80., 63.)].into_iter().enumerate() {
        prior.push(window.editor.scene().clone());
        let p = at(&mut window, x, y);
        shift_click(&mut window, p);
        assert_eq!(
            window.editor.selection,
            [base, index as u64 + 2],
            "cut {index}, {extra_objects} extra objects"
        );
        window.click("Path");
        let button = window.control("Difference");
        window.frame(vec![egui::Event::PointerMoved(button)]);
        let press = |pressed| egui::Event::PointerButton {
            pos: button,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        window.frame(vec![press(true)]);
        let begin = std::time::Instant::now();
        let out = window.frame(vec![press(false)]);
        std::hint::black_box(window.ctx.tessellate(out.shapes, out.pixels_per_point));
        timings[index] = begin.elapsed().as_secs_f64() * 1000.;
        assert_eq!(window.editor.selection, [base]);
        let scene = window.editor.scene();
        assert_eq!(scene.objects[0].shape.contours().len(), index + 2);
        assert_eq!(scene.hit_test([x, y], 0), None);
        assert_eq!(
            renderer::render_scaled(scene, 0, 1.)
                .unwrap()
                .pixel(x as u32, y as u32)
                .unwrap()
                .alpha(),
            0
        );
        if index == 0 {
            let start = at(&mut window, 25., 32.);
            let end = at(&mut window, 30., 38.);
            window.drag(start, end - start);
            assert_ne!(
                window.editor.scene().objects[0].position_at(0),
                prior[0].objects[0].position_at(0)
            );
        }
    }
    let final_state = window.editor.scene().clone();
    window.click_edit("Undo");
    assert_eq!(window.editor.scene(), &prior[1]);
    window.click_edit("Undo"); // move
    window.click_edit("Undo"); // first hole
    assert_eq!(window.editor.scene(), &drawing);
    for _ in 0..3 {
        window.click_edit("Redo");
    }
    assert_eq!(window.editor.scene(), &final_state);
    timings
}

#[test]
fn path_menu_can_move_and_cut_the_same_result_again() {
    successive_cuts(0, false);
}

#[test]
#[ignore = "release timing benchmark"]
fn boolean_editor_responsiveness() {
    for count in [0, 1_000, 10_000] {
        let mut frames = Vec::new();
        for _ in 0..5 {
            frames.extend(successive_cuts(count, false));
        }
        frames.sort_by(f64::total_cmp);
        println!("{count} off-artboard, Difference click through tessellation: median {:.2} ms, worst {:.2} ms", (frames[4] + frames[5]) / 2., frames[9]);
    }
}

#[test]
#[ignore = "release timing benchmark"]
fn boolean_editor_dense_artboard() {
    for count in [1_000, 10_000] {
        let mut frames = Vec::new();
        for _ in 0..5 {
            frames.extend(successive_cuts(count, true));
        }
        frames.sort_by(f64::total_cmp);
        println!("{count} visible ellipses, Difference click through tessellation: median {:.2} ms, worst {:.2} ms", (frames[4] + frames[5]) / 2., frames[9]);
    }
}
