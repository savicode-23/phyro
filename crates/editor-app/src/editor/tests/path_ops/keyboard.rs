use super::*;
use document_core::BooleanOp;
use egui::{Event, Key, Modifiers};

fn command(mac: bool) -> Modifiers {
    Modifiers::COMMAND
        | if mac {
            Modifiers::MAC_CMD
        } else {
            Modifiers::CTRL
        }
}

fn key(logical: Key, physical: Key, modifiers: Modifiers, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key: logical,
        physical_key: Some(physical),
        modifiers,
        pressed,
        repeat,
    }
}

fn input(window: &mut EditorWindow, modifiers: Modifiers, events: Vec<Event>) {
    window.frame_with_input(egui::RawInput {
        modifiers,
        events,
        ..Default::default()
    });
}

fn tap(window: &mut EditorWindow, logical: Key, physical: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        input(
            window,
            modifiers,
            vec![key(logical, physical, modifiers, pressed, false)],
        );
    }
}

fn window(drawing: &Document, ids: &[u64]) -> EditorWindow {
    let mut window = EditorWindow::new(egui::vec2(1100., 740.));
    window.ctx.style_mut(|style| style.animation_time = 0.);
    window
        .editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    window.editor.set_tool(Tool::Select);
    window.editor.selection = ids.to_vec();
    window.frame(Vec::new());
    window
}

#[test]
fn a_rebound_difference_uses_its_new_key_and_releases_the_old_one() {
    use crate::keymap::{self, Action, Keymap};
    let mut doc = Document::default();
    let base = doc.add_rectangle([12.; 2], [62.; 2], [91; 4]);
    let cutter = doc.add_rectangle([33.; 2], [20.; 2], [255; 4]);
    let mut expected = doc.clone();
    expected
        .boolean(&[base, cutter], 0, BooleanOp::Difference)
        .unwrap();
    let mut view = window(&doc, &[base, cutter]);
    let mut bindings = Keymap::default();
    bindings
        .set(Action::Difference, Key::J, Modifiers::ALT)
        .unwrap();
    keymap::install(&view.ctx, std::sync::Arc::new(bindings));

    tap(&mut view, Key::Minus, Key::Minus, command(false));
    assert_eq!(view.editor.scene(), &doc);
    tap(&mut view, Key::J, Key::J, Modifiers::ALT);
    assert_eq!(view.editor.scene(), &expected);
    tap(&mut view, Key::Z, Key::Z, command(false));
    assert_eq!(view.editor.scene(), &doc);
}

#[test]
fn stacking_shortcuts_cross_the_next_overlap_at_the_playhead_and_undo_one_step() {
    let mut doc = Document::default();
    let selected = doc.add_rectangle([20.; 2], [30.; 2], [200, 45, 20, 255]);
    doc.objects[0].name = "Cutter".into();
    let parked = doc.add_rectangle([400.; 2], [30.; 2], [10; 4]);
    let animated = doc.add_rectangle([200.; 2], [30.; 2], [30, 100, 50, 255]);
    doc.objects[2].keys.extend([(0, [200.; 2]), (2, [22.; 2])]);
    let top = doc.add_rectangle([38.; 2], [20.; 2], [255; 4]);
    let distant = doc.add_rectangle([500.; 2], [10.; 2], [255; 4]);
    let mut w = window(&doc, &[]);
    w.click("Layers");
    w.click("Cutter");
    assert_eq!(w.editor.selection, [selected]);
    let ids = |w: &EditorWindow| {
        w.editor
            .scene()
            .objects
            .iter()
            .map(|o| o.id)
            .collect::<Vec<_>>()
    };
    let plain = Modifiers::NONE;
    tap(&mut w, Key::PageUp, Key::PageUp, plain);
    assert_eq!(ids(&w), [parked, animated, top, selected, distant]);
    tap(&mut w, Key::Z, Key::Z, command(false));
    assert_eq!(w.editor.scene(), &doc);
    assert!(!w.editor.can_undo());

    w.editor.timeline.restore_playhead(0, 2);
    assert_eq!(w.editor.scene().hit_test([30.; 2], 2), Some(animated));
    tap(&mut w, Key::PageUp, Key::PageUp, plain);
    assert_eq!(ids(&w), [parked, animated, selected, top, distant]);
    assert_eq!(w.editor.scene().hit_test([30.; 2], 2), Some(selected));
    tap(&mut w, Key::PageDown, Key::PageDown, plain);
    assert_eq!(ids(&w), [parked, selected, animated, top, distant]);
    tap(&mut w, Key::Home, Key::Home, plain);
    assert_eq!(ids(&w), [parked, animated, top, distant, selected]);
    tap(&mut w, Key::End, Key::End, plain);
    assert_eq!(w.editor.scene(), &doc);
    assert_eq!(w.editor.selection, [selected]);
    tap(&mut w, Key::End, Key::End, plain);
    tap(&mut w, Key::Z, Key::Z, command(false));
    assert_eq!(ids(&w), [parked, animated, top, distant, selected]);
    tap(&mut w, Key::Y, Key::Y, command(false));
    assert_eq!(w.editor.scene(), &doc);
    w.click("Object");
    w.click("Stacking order");
    w.click("Raise one step");
    assert_eq!(ids(&w), [parked, animated, selected, top, distant]);
    tap(&mut w, Key::Z, Key::Z, command(false));
    assert_eq!(w.editor.scene(), &doc);
    let hold = w.field_on_row("Hold", " fr");
    w.click_at(hold);
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    tap(&mut w, Key::Home, Key::Home, plain);
    assert_eq!(w.editor.scene(), &doc);
}

#[test]
fn alignment_shortcut_reveals_the_panel_and_preserves_selection_on_small_windows() {
    let mut doc = Document::default();
    let base = doc.add_rectangle([32., 21.], [68., 74.], [155; 4]);
    let small = doc.add_rectangle([110., 8.], [8., 8.], [200; 4]);
    let other = doc.add_rectangle([3., 5.], [3., 3.], [255; 4]);
    let mut view = window(&doc, &[base, small]);
    view.size = egui::vec2(800., 500.);
    let board = view.board();
    view.click("Layers");
    assert_eq!(view.editor.inspector_tab, crate::inspector::Tab::Layers);
    tap(&mut view, Key::A, Key::A, command(false) | Modifiers::SHIFT);
    for _ in 0..24 {
        view.frame(Vec::new());
    }
    assert_eq!(view.editor.selection, [base, small]);
    assert_eq!(view.editor.inspector_tab, crate::inspector::Tab::Properties);
    assert_eq!(view.editor.scene(), &doc);
    assert_eq!(view.board(), board);
    view.click("Right");
    assert_eq!(view.editor.scene().objects[1].position, [92., 8.]);
    tap(&mut view, Key::A, Key::A, command(false));
    assert_eq!(view.editor.selection, [base, small, other]);
}

#[test]
fn raising_a_cutter_across_off_canvas_art_changes_the_next_difference() {
    let mut doc = Document::default();
    let cutter = doc.add_rectangle([40.; 2], [16.; 2], [215, 140, 55, 255]);
    doc.add_rectangle([256.; 2], [12.; 2], [90; 4]);
    let base = doc.add_rectangle([20.; 2], [70.; 2], [31, 82, 128, 255]);
    let mut view = window(&doc, &[cutter]);
    assert_eq!(view.editor.scene().hit_test([48.; 2], 0), Some(base));
    tap(&mut view, Key::PageUp, Key::PageUp, Modifiers::NONE);
    assert_eq!(view.editor.scene().hit_test([48.; 2], 0), Some(cutter));
    let raised = view.editor.scene().clone();
    let board = view.board();
    shift_click(&mut view, board.min + board.size() * (26. / 128.));
    tap(&mut view, Key::Minus, Key::Minus, command(false));
    assert_eq!(view.editor.selection, [base]);
    let image = renderer::render(view.editor.scene(), 0).unwrap();
    assert_eq!(image.pixel(48, 48).unwrap().alpha(), 0);
    assert_eq!(image.pixel(26, 26).unwrap().alpha(), 255);
    for expected in [&raised, &doc] {
        tap(&mut view, Key::Z, Key::Z, command(false));
        assert_eq!(view.editor.scene(), expected);
    }
    assert!(!view.editor.can_undo());
}

#[test]
fn center_a_cutter_on_both_axes_and_cut_without_reselecting() {
    let mut doc = Document::default();
    let base = doc.add_rectangle([32., 21.], [68., 74.], [31, 82, 128, 255]);
    let cutter = doc.add_shape(
        [100., 6.],
        [20., 16.],
        [231, 160, 94, 255],
        document_core::Shape::Ellipse,
        1,
    );
    let mut view = window(&doc, &[]);
    view.size = egui::vec2(1100., 1000.);
    let board = view.board();
    let at = |x: f32, y: f32| board.min + board.size() * egui::vec2(x / 128., y / 128.);
    view.click_at(at(110., 14.));
    shift_click(&mut view, at(70., 64.));
    view.click("Align and Distribute");
    view.click("Center X");
    let horizontal = view.editor.scene().clone();
    assert_eq!(horizontal.objects[1].position, [56., 6.]);
    view.click("Center Y");
    let centered = view.editor.scene().clone();
    assert_eq!(centered.objects[1].position, [56., 50.]);
    assert_eq!(centered.objects[0], doc.objects[0]);
    assert_eq!(view.editor.selection, [cutter, base]);
    let cmd = command(false);
    tap(&mut view, Key::Minus, Key::Minus, cmd);
    assert_eq!(view.editor.selection, [base]);
    let image = renderer::render(view.editor.scene(), 0).unwrap();
    assert_eq!(image.pixel(66, 58).unwrap().alpha(), 0);
    assert_eq!(image.pixel(43, 58).unwrap().alpha(), 255);
    for expected in [&centered, &horizontal, &doc] {
        tap(&mut view, Key::Z, Key::Z, cmd);
        assert_eq!(view.editor.scene(), expected);
    }
    assert!(!view.editor.can_undo());
}

#[test]
fn duplicate_selects_an_in_place_copy_that_can_move_and_cut_independently() {
    let mut doc = Document::default();
    let base = doc.add_rectangle([8.; 2], [96.; 2], [140, 60, 20, 255]);
    let cutter = doc.add_rectangle([22.; 2], [20.; 2], [255; 4]);
    for mac in [false, true] {
        let cmd = command(mac);
        let mut view = window(&doc, &[cutter]);
        view.editor.clipboard.copy(&doc, &[base], 0);
        let clipboard = view.editor.clipboard.encode().unwrap();
        tap(&mut view, Key::D, Key::D, cmd | Modifiers::SHIFT);
        assert_eq!(view.editor.scene(), &doc);
        input(&mut view, cmd, vec![key(Key::D, Key::D, cmd, true, false)]);
        let duplicated = view.editor.scene().clone();
        assert_eq!(duplicated.objects.len(), 3);
        let copy = duplicated.objects[2].id;
        assert_ne!(copy, cutter);
        assert_eq!(view.editor.selection, [copy]);
        let mut expected = doc.objects[1].clone();
        expected.id = copy;
        assert_eq!(duplicated.objects[2], expected);
        input(&mut view, cmd, vec![key(Key::D, Key::D, cmd, true, true)]);
        assert_eq!(view.editor.scene(), &duplicated);
        input(&mut view, cmd, vec![key(Key::D, Key::D, cmd, false, false)]);
        assert_eq!(view.editor.clipboard.encode().unwrap(), clipboard);

        let board = view.board();
        let at = |x: f32, y: f32| board.min + board.size() * egui::vec2(x / 128., y / 128.);
        let body = at(27., 27.);
        view.drag(body, at(57., 44.) - body);
        let moved = view.editor.scene().clone();
        assert_eq!(&moved.objects[..2], &doc.objects);
        assert_ne!(moved.objects[2].position, expected.position);
        shift_click(&mut view, at(16., 19.));
        tap(&mut view, Key::Minus, Key::Minus, cmd);
        assert_eq!(view.editor.selection, [base]);
        assert_eq!(view.editor.scene().hit_test([62., 49.], 0), None);
        assert_eq!(view.editor.scene().hit_test([32., 32.], 0), Some(cutter));
        for expected in [&moved, &duplicated, &doc] {
            tap(&mut view, Key::Z, Key::Z, cmd);
            assert_eq!(view.editor.scene(), expected);
        }
        assert!(!view.editor.can_undo());
        tap(&mut view, Key::Y, Key::Y, cmd);
        assert_eq!(view.editor.scene(), &duplicated);
    }
}

#[test]
fn duplicate_leaves_a_focused_timeline_field_and_its_drawing_alone() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([12.; 2], [20.; 2], [255; 4]);
    let mut view = window(&doc, &[id]);
    let hold = view.field_on_row("Hold", " fr");
    view.click_at(hold);
    view.frame(Vec::new());
    assert!(view.ctx.wants_keyboard_input());
    tap(&mut view, Key::D, Key::D, command(false));
    assert_eq!(view.editor.scene(), &doc);
    assert_eq!(view.editor.selection, [id]);
}

#[test]
fn platform_shortcuts_apply_all_six_model_operations() {
    let mut original = Document::default();
    let ids = [
        original.add_rectangle([16., 20.], [64., 48.], [32, 99, 149, 255]),
        original.add_rectangle([55., 31.], [40., 50.], [221, 115, 84, 255]),
    ];
    let plain = Modifiers::NONE;
    for mac in [false, true] {
        for (op, logical, physical, extra) in [
            (BooleanOp::Union, Key::Plus, Key::Plus, plain),
            (BooleanOp::Union, Key::Plus, Key::Equals, Modifiers::SHIFT),
            (BooleanOp::Union, Key::Equals, Key::Equals, Modifiers::SHIFT),
            (BooleanOp::Difference, Key::Minus, Key::Minus, plain),
            (
                BooleanOp::Intersection,
                Key::Num8,
                Key::Num8,
                Modifiers::SHIFT,
            ),
            (BooleanOp::Exclusion, Key::Num6, Key::Num6, Modifiers::SHIFT),
            (BooleanOp::Division, Key::Slash, Key::Slash, plain),
            (BooleanOp::CutPath, Key::Slash, Key::Slash, Modifiers::ALT),
        ] {
            let mut expected = original.clone();
            let result_ids = expected.boolean(&ids, 0, op).unwrap();
            let mut view = window(&original, &ids);
            view.ctx.set_os(if mac {
                egui::os::OperatingSystem::Mac
            } else {
                egui::os::OperatingSystem::Windows
            });
            let zoom = view.ctx.zoom_factor();
            tap(&mut view, logical, physical, command(mac) | extra);
            assert_eq!(
                view.editor.scene(),
                &expected,
                "{op:?} logical={logical:?} mac={mac}"
            );
            assert_eq!(view.editor.selection, result_ids);
            assert_eq!(
                view.ctx.zoom_factor(),
                zoom,
                "{op:?} also zoomed the interface"
            );
            tap(&mut view, Key::Z, Key::Z, command(mac));
            assert_eq!(view.editor.scene(), &original);
            assert!(
                !view.editor.can_undo(),
                "{op:?} created multiple undo steps"
            );
        }
    }
}

#[test]
fn consecutive_keyboard_differences_preserve_selection_history_and_zoom() {
    let mut drawing = Document::default();
    let base = drawing.add_rectangle([14., 18.], [92., 80.], [41, 86, 131, 255]);
    let first = drawing.add_rectangle([36., 42.], [16., 18.], [210; 4]);
    let second = drawing.add_rectangle([72., 54.], [16., 18.], [210; 4]);
    let mut once = drawing.clone();
    once.boolean(&[base, first], 0, BooleanOp::Difference)
        .unwrap();
    let mut twice = once.clone();
    twice
        .boolean(&[base, second], 0, BooleanOp::Difference)
        .unwrap();
    let mut view = window(&drawing, &[base, first]);
    let zoom = view.ctx.zoom_factor();
    let cmd = command(false);
    // Hold the first shortcut while picking the next cutter. Auto-repeat must
    // consume neither the newly prepared pair nor egui's interface zoom.
    input(
        &mut view,
        cmd,
        vec![key(Key::Minus, Key::Minus, cmd, true, false)],
    );
    assert_eq!(view.editor.scene(), &once);
    assert_eq!(view.editor.selection, [base]);
    input(
        &mut view,
        cmd,
        vec![key(Key::Minus, Key::Minus, cmd, true, true)],
    );
    let board = view.board();
    let point = board.min + board.size() * egui::vec2(80. / 128., 63. / 128.);
    shift_click(&mut view, point);
    assert_eq!(view.editor.selection, [base, second]);
    input(
        &mut view,
        cmd,
        vec![key(Key::Minus, Key::Minus, cmd, true, true)],
    );
    assert_eq!(view.editor.scene(), &once);
    input(
        &mut view,
        Modifiers::NONE,
        vec![key(Key::Minus, Key::Minus, Modifiers::NONE, false, false)],
    );
    tap(&mut view, Key::Minus, Key::Minus, cmd);
    assert_eq!(view.editor.scene(), &twice);
    assert_eq!(view.editor.selection, [base]);
    assert_eq!(view.ctx.zoom_factor(), zoom);
    for expected in [&once, &drawing] {
        tap(&mut view, Key::Z, Key::Z, cmd);
        assert_eq!(view.editor.scene(), expected);
    }
    assert!(!view.editor.can_undo());
    for expected in [&once, &twice] {
        tap(&mut view, Key::Z, Key::Z, cmd | Modifiers::SHIFT);
        assert_eq!(view.editor.scene(), expected);
    }
    assert!(!view.editor.history.can_redo());
}

#[test]
fn minus_shortcut_still_zooms_with_one_selected_object() {
    let mut drawing = Document::default();
    let id = drawing.add_rectangle([16., 20.], [64., 48.], [65; 4]);
    let mut view = window(&drawing, &[id]);
    let zoom = view.ctx.zoom_factor();
    tap(&mut view, Key::Minus, Key::Minus, command(false));
    assert!(view.ctx.zoom_factor() < zoom);
    assert_eq!(view.editor.scene(), &drawing);
    assert!(!view.editor.can_undo());
}

#[test]
fn unfocused_window_text_entry_and_playback_preserve_the_operands() {
    let mut drawing = Document::default();
    let ids = [
        drawing.add_rectangle([16., 20.], [64., 48.], [65; 4]),
        drawing.add_rectangle([55., 31.], [40., 50.], [180; 4]),
    ];
    let modifiers = command(false) | Modifiers::SHIFT;
    let mut view = window(&drawing, &ids);
    view.frame_with_input(egui::RawInput {
        focused: false,
        modifiers,
        events: vec![
            Event::WindowFocused(false),
            key(Key::Num8, Key::Num8, modifiers, true, false),
            key(Key::Num8, Key::Num8, modifiers, false, false),
        ],
        ..Default::default()
    });
    assert_eq!(view.editor.scene(), &drawing);
    view.frame(vec![Event::WindowFocused(true)]);

    view.click("Text");
    let at = view.board().center();
    view.click_at(at);
    assert!(view.editor.canvas.text_active() && view.ctx.wants_keyboard_input());
    tap(&mut view, Key::Num8, Key::Num8, modifiers);
    assert_eq!(view.editor.scene(), &drawing);
    tap(&mut view, Key::Escape, Key::Escape, Modifiers::NONE);
    let play = view.timeline_icon("Play");
    view.click_at(play);
    assert!(view.editor.timeline.is_playing());
    tap(&mut view, Key::Num8, Key::Num8, modifiers);
    assert_eq!(view.editor.scene(), &drawing);
    assert!(!view.editor.can_undo());
}

#[test]
fn duplicate_merge_and_edit_a_curve_then_reopen_the_result() {
    use document_core::{path_edit, Shape};
    let mut doc = Document::default();
    let ellipse = path_edit::primitive_path(&Shape::Ellipse).unwrap();
    let first = doc.add_shape([16., 32.], [52., 54.], [41, 120, 95, 255], ellipse, 1);
    let mut view = window(&doc, &[first]);
    let cmd = command(false);
    tap(&mut view, Key::D, Key::D, cmd);
    let board = view.board();
    let at = |x: f32, y: f32| board.min + board.size() * egui::vec2(x / 128., y / 128.);
    let body = at(35., 52.);
    view.drag(body, at(65., 52.) - body);
    let sources = view.editor.scene().clone();
    shift_click(&mut view, at(29., 60.));
    tap(&mut view, Key::Plus, Key::Plus, cmd);
    assert_eq!(view.editor.selection, [first]);
    let union = view.editor.scene().clone();
    assert_eq!(union.objects.len(), 1);
    let nodes = &union.objects[0].shape.contours()[0];
    let count = nodes.len();
    assert!(count <= 16 && nodes.iter().any(|n| n.incoming != n.point));
    // The two source fills define the expected silhouette independently of the Boolean solver.
    let expected = renderer::render_scaled(&sources, 0, 4.).unwrap();
    let actual = renderer::render_scaled(&union, 0, 4.).unwrap();
    let width = expected.width() as usize;
    for y in 1..expected.height() as usize - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            let a = expected.pixels()[i].alpha();
            if (a == 0 || a == 255)
                && (y - 1..=y + 1).all(|row| {
                    expected.pixels()[row * width + x - 1..=row * width + x + 1]
                        .iter()
                        .all(|p| p.alpha() == a)
                })
            {
                assert_eq!(
                    actual.pixels()[i].alpha(),
                    a,
                    "changed fill at sample ({x}, {y})"
                );
            }
        }
    }
    tap(&mut view, Key::N, Key::N, Modifiers::NONE);
    view.click_at(at(23.615, 39.908));
    view.click_at(at(23.615, 39.908));
    let edited = view.editor.scene().clone();
    assert_eq!(edited.objects[0].shape.contours()[0].len(), count + 1);
    tap(&mut view, Key::Z, Key::Z, cmd);
    assert_eq!(view.editor.scene(), &union);
    tap(&mut view, Key::Y, Key::Y, cmd);
    assert_eq!(view.editor.scene(), &edited);

    let folder = tempfile::tempdir().unwrap();
    let native = folder.path().join("joined.vanim");
    let svg = folder.path().join("joined.svg");
    project_io::save_animation(&native, &view.editor.project, false).unwrap();
    let reopened = project_io::load_animation(&native).unwrap();
    assert_eq!(reopened.clips[0].drawing_at(0), &edited);
    project_io::export_svg(&svg, &edited, 0, false).unwrap();
    let imported = project_io::import_svg(&svg).unwrap();
    let imported_nodes = &imported.objects[0].shape.contours()[0];
    assert_eq!(imported_nodes.len(), count + 1);
    assert!(imported_nodes.iter().any(|n| n.incoming != n.point));
    let native_pixels = renderer::render(&edited, 0).unwrap();
    let svg_pixels = renderer::render(&imported, 0).unwrap();
    let error: usize = native_pixels
        .data()
        .iter()
        .zip(svg_pixels.data())
        .map(|(a, b)| a.abs_diff(*b) as usize)
        .sum();
    assert!(
        error < 512,
        "SVG round trip changed the raster by {error} total channel levels"
    );
}
