use super::*;
use egui::{Event, Key, Modifiers};
use std::sync::Arc;

fn window() -> (EditorWindow, u64) {
    let mut first = Document::default();
    let id = first.add_rectangle([20., 20.], [24., 16.], [220, 70, 25, 255]);
    let mut next = first.clone();
    next.objects[0].position = [60., 30.];
    let mut project = AnimationProject::from_document(first);
    project.clips[0].cels.insert(3, Arc::new(next));
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor.replace_project(project, None);
    w.editor.selection = vec![id];
    w.frame(vec![]);
    (w, id)
}

fn key_event(w: &mut EditorWindow, key: Key, modifiers: Modifiers, pressed: bool, repeat: bool) {
    w.frame_with_input(egui::RawInput {
        modifiers,
        events: vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat,
            modifiers,
        }],
        ..Default::default()
    });
}

fn tap(w: &mut EditorWindow, key: Key, modifiers: Modifiers) {
    key_event(w, key, modifiers, true, false);
    key_event(w, key, modifiers, false, false);
}

fn click_with_modifiers(w: &mut EditorWindow, pos: egui::Pos2, modifiers: Modifiers) {
    gesture_with_modifiers(w, pos, pos, modifiers);
}

fn gesture_with_modifiers(
    w: &mut EditorWindow,
    from: egui::Pos2,
    to: egui::Pos2,
    modifiers: Modifiers,
) {
    w.frame(vec![Event::PointerMoved(from)]);
    for (pos, pressed) in [(from, true), (to, false)] {
        if !pressed && from != to {
            w.frame_with_input(egui::RawInput {
                modifiers,
                events: vec![Event::PointerMoved(to)],
                ..Default::default()
            });
        }
        w.frame_with_input(egui::RawInput {
            modifiers,
            events: vec![Event::PointerButton {
                pos,
                pressed,
                modifiers,
                button: egui::PointerButton::Primary,
            }],
            ..Default::default()
        });
    }
}

#[test]
fn tabbed_toolbar_button_keeps_activation_and_allows_editor_shortcuts() {
    let (mut w, _) = window();
    tap(&mut w, Key::Tab, Modifiers::NONE);
    let focused = w.ctx.memory(|memory| memory.focused()).unwrap();
    assert!(egui::TextEdit::load_state(&w.ctx, focused).is_none());

    tap(&mut w, Key::B, Modifiers::NONE);
    assert_eq!(w.editor.tool, Tool::Pen);
    tap(&mut w, Key::Period, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 1);
    assert!(w.ctx.memory(|memory| memory.has_focus(focused)));

    tap(&mut w, Key::Space, Modifiers::NONE);
    assert!(!w.editor.timeline.is_playing());
    assert!(w.editor.menu_bar_open);
}

#[test]
fn rebinding_tools_and_frame_step_replaces_aliases_and_preserves_frame_repeat() {
    use crate::keymap::{self, Action, Keymap};
    let (mut w, _) = window();
    w.editor.set_tool(Tool::Select);
    let mut bindings = Keymap::default();
    bindings
        .set(Action::PenTool, Key::J, Modifiers::NONE)
        .unwrap();
    bindings
        .set(Action::NextFrame, Key::F9, Modifiers::SHIFT)
        .unwrap();
    keymap::install(&w.ctx, Arc::new(bindings));
    tap(&mut w, Key::B, Modifiers::NONE);
    tap(&mut w, Key::F6, Modifiers::SHIFT);
    assert_eq!(w.editor.tool, Tool::Select);
    tap(&mut w, Key::J, Modifiers::NONE);
    assert_eq!(w.editor.tool, Tool::Pen);

    tap(&mut w, Key::Period, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 0);
    key_event(&mut w, Key::F9, Modifiers::SHIFT, true, false);
    key_event(&mut w, Key::F9, Modifiers::SHIFT, true, true);
    key_event(&mut w, Key::F9, Modifiers::SHIFT, false, false);
    assert_eq!(w.editor.timeline.frame, 2);
}

#[test]
fn quick_frame_and_key_commands_apply_in_input_order_between_redraws() {
    let (mut w, _) = window();
    let events = [Key::Period, Key::K, Key::Period, Key::K]
        .into_iter()
        .flat_map(|key| {
            [true, false].map(|pressed| Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            })
        })
        .collect();
    w.frame(events);
    assert_eq!(w.editor.timeline.frame, 2);
    assert!(w.editor.scene().objects[0].keys.contains_key(&1));
    assert!(w.editor.scene().objects[0].keys.contains_key(&2));
    w.editor.restore(false);
    assert!(w.editor.scene().objects[0].keys.contains_key(&1));
    assert!(!w.editor.scene().objects[0].keys.contains_key(&2));
}

#[test]
fn stepping_preserves_a_held_selection_and_clears_it_at_the_next_exposure() {
    let (mut w, id) = window();
    let original = w.editor.project.clone();
    tap(&mut w, Key::Comma, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 0);
    key_event(&mut w, Key::Period, Modifiers::NONE, true, false);
    key_event(&mut w, Key::Period, Modifiers::NONE, true, true);
    assert_eq!(w.editor.timeline.frame, 2);
    assert_eq!(w.editor.selection, [id]);
    key_event(&mut w, Key::Period, Modifiers::NONE, true, true);
    assert_eq!(w.editor.timeline.frame, 3);
    assert_eq!(w.editor.scene().objects[0].id, id);
    assert_eq!(w.editor.scene().objects[0].position, [60., 30.]);
    assert!(w.editor.selection.is_empty());
    key_event(&mut w, Key::Period, Modifiers::NONE, false, false);
    for _ in 0..2 {
        tap(&mut w, Key::Period, Modifiers::NONE);
    }
    assert_eq!(w.editor.timeline.frame, 4);
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
}

#[test]
fn an_unfinished_pen_extension_cannot_land_on_the_same_id_in_another_drawing() {
    use document_core::PathNode;
    let mut source = Document::default();
    let id = source
        .add_artboard_path(
            vec![PathNode::corner([24., 30.]), PathNode::corner([64., 60.])],
            false,
            1,
            [75, 162, 109, 255],
        )
        .unwrap();
    let mut next = source.clone();
    next.objects[0].name = "Other drawing".into();
    let mut project = AnimationProject::from_document(source);
    project.clips[0].cels.insert(1, Arc::new(next));
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(project.clone(), None);
    w.editor.selection = vec![id];
    tap(&mut w, Key::B, Modifiers::NONE);
    let board = w.board();
    let at = |x: f32, y: f32| board.min + board.size() * egui::vec2(x / 128., y / 128.);
    w.click_at(at(64., 60.));
    w.click_at(at(90., 96.));
    assert!(w.editor.canvas.pen_in_progress());
    assert_eq!(w.editor.selection, [id]);
    let next_frame = w.timeline_icon("Next");
    w.click_at(next_frame);
    assert_eq!(w.editor.timeline.frame, 1);
    assert!(!w.editor.canvas.pen_in_progress());
    tap(&mut w, Key::Enter, Modifiers::NONE);
    assert_eq!(w.editor.project, project);
    assert!(!w.editor.can_undo());
}

#[test]
fn key_duplicate_and_blank_shortcuts_each_make_one_undoable_edit() {
    let (mut w, _) = window();
    tap(&mut w, Key::Period, Modifiers::NONE);
    let original = w.editor.project.clone();
    w.click("Track: Position");
    w.click("Opacity");
    tap(&mut w, Key::K, Modifiers::NONE);
    let obj = &w.editor.scene().objects[0];
    assert_eq!(obj.opacity_keys.get(&1), Some(&obj.opacity));
    assert!(obj.keys.is_empty());
    let keyed = w.editor.project.clone();
    let mut duplicated = keyed.clone();
    let copy_tick = duplicated.clips[0].duplicate_layer_frame(1, 1).unwrap();
    key_event(&mut w, Key::D, Modifiers::SHIFT, true, false);
    key_event(&mut w, Key::D, Modifiers::SHIFT, true, true);
    key_event(&mut w, Key::D, Modifiers::SHIFT, false, false);
    assert_eq!(w.editor.timeline.frame, copy_tick);
    assert_eq!(w.editor.project, duplicated);
    let mut blank = duplicated.clone();
    let blank_tick = blank.clips[0].blank_layer_frame(copy_tick, 1).unwrap();
    tap(&mut w, Key::F7, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, blank_tick);
    assert!(w.editor.scene().objects.is_empty());
    assert_eq!(w.editor.project, blank);
    for expected in [&duplicated, &keyed, &original] {
        w.click_edit("Undo");
        assert_eq!(&w.editor.project, expected);
    }
    assert!(!w.editor.can_undo());
    for expected in [&keyed, &duplicated, &blank] {
        w.click_edit("Redo");
        assert_eq!(&w.editor.project, expected);
    }
}

#[test]
fn choosing_a_timeline_layer_targets_its_drawings_and_keeps_another_layers_selection_on_a_hold() {
    let mut doc = Document {
        frames: 1000,
        ..Default::default()
    };
    let actor = doc.add_rectangle([30.; 2], [36.; 2], [230, 90, 15, 255]);
    let effects = crate::commands::add_layer(&mut doc);
    doc.layers[0].name = "Character".into();
    doc.layers.last_mut().unwrap().name = "Sparks".into();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor
        .replace_project(AnimationProject::from_document(doc), None);
    w.editor.set_tool(Tool::Select);
    let board = w.board();
    let actor_at = board.min + board.size() * 0.36;
    w.click_at(actor_at);
    assert_eq!(w.editor.selection, [actor]);
    let sparks = w.control("Sparks");
    w.click_at(sparks);
    assert_eq!(w.editor.style.layer, effects);
    assert!(w.editor.selection.is_empty());
    tap(&mut w, Key::F7, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 1);
    assert_eq!(w.editor.project.clips[0].layer_exposure(1, effects).0, 1);
    assert_eq!(w.editor.project.clips[0].layer_exposure(1, 1), (0, 1000));
    w.click_at(actor_at);
    assert_eq!(w.editor.selection, [actor]);
    tap(&mut w, Key::Comma, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 0);
    assert_eq!(w.editor.selection, [actor]);
    tap(&mut w, Key::Period, Modifiers::NONE);
    assert_eq!(w.editor.selection, [actor]);
    w.control("Position");
}

#[test]
fn selected_wrist_survives_an_effects_cut_and_clears_at_an_independent_rig() {
    let mut doc = Document::default();
    doc.add_bone("Upper arm", None, [0.; 2], [15., 0.]).unwrap();
    let wrist = doc.add_bone("Wrist", None, [18., 0.], [24., 0.]).unwrap();
    doc.set_bone_rotation_key(wrist, 1, 15.).unwrap();
    let effects = crate::commands::add_layer(&mut doc);
    let mut project = AnimationProject::from_document(doc);
    let clip = &mut project.clips[0];
    let duplicate = clip.cels[&0].clone();
    clip.cels.insert(4, duplicate);
    clip.duplicate_layer_frame(2, effects).unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(project, None);
    let session = w.editor.project.clips[0].session_id();
    w.frame(vec![]);
    w.click("Rig");
    w.click("Upper arm");
    w.click("Wrist");
    w.click("Properties");
    assert_eq!(
        w.editor.canvas.rig.selected_bone(),
        Some((session, 0, wrist))
    );
    for frame in 1..=3 {
        tap(&mut w, Key::Period, Modifiers::NONE);
        assert_eq!(w.editor.timeline.frame, frame);
        assert_eq!(
            w.editor.canvas.rig.selected_bone(),
            Some((session, 0, wrist))
        );
    }
    tap(&mut w, Key::Period, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 4);
    assert!(w.editor.canvas.rig.selected_bone().is_none());
}

#[test]
fn drawing_a_fresh_effect_keeps_the_plane_on_its_pose_and_other_frames_intact() {
    let file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/draw-sparks-over-motion.vanim");
    let original = project_io::load_animation(&file).unwrap();
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.editor.replace_project(original.clone(), None);
    w.frame(vec![]);
    assert_eq!(
        w.editor.style.layer, 2,
        "the example opens on its editable FX layer"
    );
    w.click("Layers");
    w.click("Draw sparks here");
    let layer = w.editor.style.layer;
    assert_eq!(
        w.editor
            .scene()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap()
            .name,
        "Draw sparks here"
    );
    w.click("Frame 1");
    w.frame(vec![]);
    w.frame(vec![Event::Text("12".into())]);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    tap(&mut w, Key::F7, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 11);
    assert_eq!(w.editor.style.layer, layer);
    let clip = &w.editor.project.clips[0];
    assert_eq!(clip.frames, 24);
    let mut plane_only = original.clips[0].drawing_at(11).clone();
    plane_only.objects.retain(|object| object.layer != layer);
    for tick in 0..24 {
        let scene = original.clips[0].drawing_at(tick);
        let expected = if tick == 11 { &plane_only } else { scene };
        assert_eq!(
            renderer::render(clip.drawing_at(tick), tick)
                .unwrap()
                .data(),
            renderer::render(expected, tick).unwrap().data(),
            "frame {}",
            tick + 1
        );
    }
    // A new FX mark stays on its own exposure while the child keeps playing.
    let pos = w.board().center();
    w.editor.set_tool(Tool::Rectangle);
    w.drag(pos, egui::vec2(20., 16.));
    assert_eq!(w.editor.scene().objects.last().unwrap().layer, layer);
    for tick in [10, 12] {
        // Independent layer holds share an allocation cursor so new objects cannot
        // collide when they are composited. The surrounding artwork stays identical.
        let mut expected = original.clips[0].drawing_at(tick).clone();
        let current = w.editor.project.clips[0].drawing_at(tick);
        assert!(current.next_id >= expected.next_id);
        expected.next_id = current.next_id;
        assert_eq!(current, &expected);
    }
    w.click_edit("Undo");
    w.click_edit("Undo");
    assert_eq!(w.editor.project, original);
}

#[test]
fn point_text_receives_animation_shortcut_characters_as_lettering() {
    let (mut w, _) = window();
    let original = w.editor.project.clone();
    let board = w.board();
    tap(&mut w, Key::T, Modifiers::NONE);
    w.click_at(board.min + board.size() * 0.6);
    assert!(w.editor.canvas.text_active() && w.ctx.wants_keyboard_input());
    for (key, modifiers, letter) in [
        (Key::Period, Modifiers::NONE, "."),
        (Key::Comma, Modifiers::NONE, ","),
        (Key::K, Modifiers::NONE, "k"),
        (Key::K, Modifiers::SHIFT, "K"),
        (Key::D, Modifiers::SHIFT, "D"),
        (Key::O, Modifiers::NONE, "o"),
    ] {
        key_event(&mut w, key, modifiers, true, false);
        w.frame(vec![Event::Text(letter.into())]);
        key_event(&mut w, key, modifiers, false, false);
    }
    assert_eq!(w.editor.timeline.frame, 0);
    assert!(!w.editor.timeline.onion);
    tap(&mut w, Key::Enter, Modifiers::CTRL | Modifiers::COMMAND);
    let lettering = w
        .editor
        .scene()
        .objects
        .last()
        .unwrap()
        .text
        .as_ref()
        .unwrap();
    assert_eq!(lettering.content, ".,kKDo");
    w.click_edit("Undo");
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
}

#[test]
fn onion_shortcuts_change_each_side_without_an_artwork_edit() {
    let (mut w, id) = window();
    tap(&mut w, Key::O, Modifiers::NONE);
    assert!(w.editor.timeline.onion);
    tap(&mut w, Key::O, Modifiers::NONE);
    assert!(!w.editor.timeline.onion);
    tap(&mut w, Key::Comma, Modifiers::ALT);
    assert!(w.editor.timeline.onion);
    assert_eq!(w.editor.timeline.previous_onions, 2);
    assert_eq!(w.editor.timeline.next_onions, 1);
    for _ in 0..8 {
        tap(&mut w, Key::Period, Modifiers::ALT);
    }
    assert_eq!(w.editor.timeline.next_onions, 0);
    assert_eq!(w.editor.timeline.previous_onions, 2);
    tap(&mut w, Key::O, Modifiers::SHIFT);
    w.control("Onion skin settings");
    tap(&mut w, Key::O, Modifiers::SHIFT);
    assert!(!w.frame(vec![]).shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "Onion skin settings")
    }));
    assert_eq!(w.editor.timeline.frame, 0);
    assert_eq!(w.editor.selection, [id]);
    assert!(!w.editor.can_undo());
}

#[test]
fn focused_fps_entry_keeps_its_keys_and_commits_once_for_the_whole_clip() {
    let (mut w, id) = window();
    let original = w.editor.project.clone();
    w.click("12 fps");
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    for (key, modifiers) in [
        (Key::Period, Modifiers::NONE),
        (Key::K, Modifiers::NONE),
        (Key::K, Modifiers::SHIFT),
        (Key::D, Modifiers::SHIFT),
        (Key::O, Modifiers::NONE),
    ] {
        tap(&mut w, key, modifiers);
    }
    assert_eq!(w.editor.timeline.frame, 0);
    assert_eq!(w.editor.selection, [id]);
    assert!(!w.editor.timeline.onion);
    assert_eq!(w.editor.project, original);
    w.frame(vec![Event::Text("24".into())]);
    assert_eq!(w.editor.project, original);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    let mut expected = original.clone();
    expected.clips[0].set_fps(24).unwrap();
    assert_eq!(w.editor.project, expected);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
    w.click_edit("Redo");
    assert_eq!(w.editor.project, expected);
}

#[test]
fn transport_edits_clip_length_before_fps_with_one_undo() {
    let (mut w, _) = window();
    let before = w.editor.project.clone();
    let frames = before.clips[0].frames;
    let count = w.control(&format!("{frames} frames"));
    let fps = w.control("12 fps");
    assert!(count.x < fps.x && (count.y - fps.y).abs() < 3.);
    w.click_at(count);
    w.frame(vec![Event::Text("1000".into())]);
    assert_eq!(w.editor.project, before);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    assert_eq!(w.editor.project.clips[0].frames, 1000);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, before);
}

#[test]
fn entering_a_frame_number_reveals_that_tick_in_a_long_clip() {
    let (mut w, _) = window();
    w.editor.project.clips[0].set_length(1_000).unwrap();
    let document = w.editor.project.clone();
    w.click("Frame 1");
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    w.frame(vec![Event::Text("945".into())]);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    assert_eq!(w.editor.timeline.frame, 944);
    assert!(
        w.editor.selection.is_empty(),
        "the target is in a different drawing"
    );
    assert_eq!(w.editor.project, document);
    w.control("945");
    let repeat = w.timeline_icon("Loop playback");
    w.click_at(repeat);
    w.click("Frame 945");
    w.frame(vec![Event::Text("1".into())]);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    w.control("1");
    w.click_edit("Undo");
    assert_eq!(w.editor.project, document);
    assert_eq!(w.editor.timeline.frame, 944);
    w.control("945");
}

#[test]
fn timeline_scale_endpoints_stay_within_the_window() {
    for size in [egui::vec2(800., 500.), egui::vec2(1920., 1080.)] {
        let mut w = EditorWindow::new(size);
        w.editor.project.clips[0].set_length(1000).unwrap();
        w.frame(vec![]);
        w.click("Track: Position");
        w.click("Path progress");
        let before = w.editor.project.clone();
        let field = w.control("100%");
        let widgets = w.frame(vec![]).shapes;
        let bounds = |text: &str| {
            widgets
                .iter()
                .find_map(|shape| {
                    let egui::Shape::Text(painted) = &shape.shape else {
                        return None;
                    };
                    (painted.galley.text() == text)
                        .then(|| painted.galley.rect.translate(painted.pos.to_vec2()))
                })
                .unwrap()
        };
        let label = bounds("Timeline scale");
        assert!(bounds("Curve").right() < label.left() && label.right() < field.x);
        let rail = widgets
            .iter()
            .find_map(|shape| {
                let egui::Shape::Rect(painted) = &shape.shape else {
                    return None;
                };
                let rect = painted.rect;
                (rect.left() > field.x
                    && (rect.center().y - field.y).abs() < 1.
                    && rect.height() == w.ctx.style().spacing.slider_rail_height
                    && shape.clip_rect.contains_rect(rect))
                .then_some(rect)
            })
            .expect("fully visible scale rail to the right of the percent");
        let min = rail.left_center() + egui::vec2(1., 0.);
        let max = rail.right_center() - egui::vec2(1., 0.);
        assert!(w.ctx.screen_rect().contains_rect(rail));
        w.click_at(max);
        w.control("200%");
        w.click_at(min);
        w.control("5%");
        w.drag(min, max - min);
        w.control("200%");
        w.drag(max, min - max);
        w.control("5%");
        // Also exercise the original numeric-field drag against the screen edge.
        let field = w.control("5%");
        w.drag(field, max - field);
        w.control("200%");
        assert_eq!(w.editor.project, before);
    }
}

#[test]
fn timeline_keeps_transport_and_drawing_controls_on_distinct_rows() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = EditorWindow::new(size);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let board = w.board();
        let playback = w.timeline_icon("Play").y;
        for name in ["Previous", "Next", "Loop playback", "Onion skin settings"] {
            assert!((w.timeline_icon(name).y - playback).abs() < 4.);
        }
        for label in ["12 fps", "Onion skin", "−1", "+1"] {
            assert!(
                (w.control(label).y - playback).abs() < 4.,
                "{label} at {size:?}"
            );
        }
        let editing = w.control("Blank frame").y;
        assert!(editing > playback + 10.);
        for label in [
            "Duplicate",
            "Add key",
            "Hold",
            "Track: Position",
            "Curve",
            "Timeline scale",
        ] {
            assert!(
                (w.control(label).y - editing).abs() < 4.,
                "{label} at {size:?}"
            );
        }
        let settings = w.timeline_icon("Onion skin settings");
        w.click_at(settings);
        w.control("Show onion skins");
        assert_eq!(w.board(), board);
    }
}

#[test]
fn transport_edits_onion_range_independently_and_saves_the_clip_loop_setting() {
    let (mut w, _) = window();
    let original = w.editor.project.clone();
    let bounds = w.board();
    for (field, value) in [("−1", "2"), ("+1", "3")] {
        w.click(field);
        w.frame(vec![Event::Text(value.into())]);
        tap(&mut w, Key::Enter, Modifiers::NONE);
    }
    assert_eq!(w.editor.timeline.previous_onions, 2);
    assert_eq!(w.editor.timeline.next_onions, 3);
    assert!(w.editor.timeline.onion);
    assert_eq!(w.editor.project, original);
    let repeat = w.timeline_icon("Loop playback");
    w.click_at(repeat);
    assert_ne!(w.editor.project.clips[0].looping, original.clips[0].looping);
    w.click_edit("Undo");
    assert_eq!(w.editor.project, original);
    assert_eq!(w.board(), bounds);
}

#[test]
fn box_selected_keys_move_and_delete_together_without_replacing_neighbors_or_artwork() {
    let mut doc = Document {
        frames: 1_000,
        ..Default::default()
    };
    let id = doc.add_rectangle([10.; 2], [8.; 2], [255; 4]);
    doc.objects[0].keys.extend([(1, [11.; 2]), (2, [12.; 2])]);
    doc.objects[0].rotation_keys.extend([(2, 22.), (4, 44.)]);
    let project = AnimationProject::from_document(doc);
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(project.clone(), None);
    w.editor.selection = vec![id];
    w.frame(vec![]);
    let cell = w.editor.timeline.frame_width();
    let x = w.control("1").x;
    let pos_y = w.control("Position").y;
    let output = w.frame(vec![]);
    let rot_y = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if t.galley.text() == "Rotation" && t.pos.y > pos_y && t.pos.x < x =>
            {
                Some(t.galley.rect.translate(t.pos.to_vec2()).center().y)
            }
            _ => None,
        })
        .expect("rotation track in left timeline gutter");
    let pos_key = egui::pos2(x + cell, pos_y);
    let second = pos_key + egui::vec2(cell, 0.);
    w.click_at(pos_key);
    click_with_modifiers(&mut w, second, Modifiers::CTRL);
    click_with_modifiers(&mut w, egui::pos2(x + 4. * cell, rot_y), Modifiers::COMMAND);
    click_with_modifiers(&mut w, second, Modifiers::CTRL);
    gesture_with_modifiers(
        &mut w,
        egui::pos2(x + cell, rot_y - 3.),
        egui::pos2(x + 2. * cell, rot_y + 3.),
        Modifiers::CTRL,
    );
    assert!(
        !w.editor.can_undo(),
        "toggling and adding diamonds changes only the selection"
    );
    tap(&mut w, Key::Delete, Modifiers::NONE);
    let mut remaining = project.clone();
    let object = &mut remaining.clips[0].drawing_at_mut(0).objects[0];
    object.keys.remove(&1);
    object.rotation_keys.clear();
    assert_eq!(
        w.editor.project, remaining,
        "the toggled-off position key survives"
    );
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(w.editor.project, project);
    assert!(!w.editor.can_undo());
    let start = egui::pos2(x, pos_y - 3.);
    let end = egui::pos2(x + cell * 2. + 2., rot_y + 3.);
    w.drag(start, end - start);
    assert_eq!(w.editor.project, project, "a key marquee is selection only");
    w.drag(pos_key, egui::vec2(2. * cell, 0.));
    assert_eq!(
        w.editor.project, project,
        "collision in the rotation row cancels both rows"
    );
    assert!(!w.editor.can_undo());
    w.drag(pos_key, egui::vec2(cell, 0.));
    let moved = w.editor.project.clone();
    assert_eq!(
        w.editor.scene().objects[0]
            .keys
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        w.editor.scene().objects[0]
            .rotation_keys
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [3, 4]
    );
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.selection, [id]);
    assert!(w.editor.scene().objects[0].keys.is_empty());
    assert_eq!(
        w.editor.scene().objects[0]
            .rotation_keys
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [4]
    );
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(
        w.editor.project, moved,
        "one Undo restores all deleted keys"
    );
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(
        w.editor.project, project,
        "one Undo restores all moved keys"
    );
    w.click_at(pos_key);
    let remove = w.timeline_icon("Remove key");
    w.click_at(remove);
    assert_eq!(
        w.editor.scene().objects[0]
            .keys
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [2]
    );
    let before_text = w.editor.project.clone();
    w.click("100%");
    w.frame(vec![Event::Text("200".into())]);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    w.control("200%");
    assert!((w.editor.timeline.frame_width() - 120.).abs() < 0.001);
    w.click("200%");
    w.frame(vec![Event::Text("5".into())]);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    w.control("5%");
    assert!((w.editor.timeline.frame_width() - 3.).abs() < 0.001);
    assert_eq!(w.editor.project, before_text);
}

#[test]
fn timeline_delete_follows_key_focus_through_scrolling_menus_and_tool_changes() {
    let (mut w, id) = window();
    let original = w.editor.project.clone();
    w.click("Position");
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(
        w.editor.project, original,
        "a track label has no selected keys"
    );
    tap(&mut w, Key::K, Modifiers::NONE);
    assert!(w.editor.scene().objects[0].keys.contains_key(&0));
    // The newly authored key is active immediately. Scrolling over the time grid
    // and opening Edit preserve that target.
    let hover = w.control("1");
    w.frame(vec![
        Event::PointerMoved(hover),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(12., 0.),
            modifiers: Modifiers::NONE,
        },
    ]);
    w.click_edit("Delete");
    assert_eq!(w.editor.selection, [id]);
    assert!(!w.editor.scene().objects[0].keys.contains_key(&0));
    w.click_edit("Undo");
    assert!(w.editor.scene().objects[0].keys.contains_key(&0));
    // The numbered ruler selects a whole frame. Its artwork survives on hold.
    let seek = w.control("2");
    w.click_at(seek);
    let length = w.editor.project.clips[0].frames;
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.project.clips[0].frames, length - 1);
    assert_eq!(w.editor.selection, [id]);
    assert_eq!(w.editor.scene().objects.len(), 1);
    let after_ripple = w.editor.project.clone();
    w.click("Position");
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(
        w.editor.project, after_ripple,
        "a track label releases the frame selection"
    );
    // Keyboard tool switches and canvas/layer clicks give object editing focus.
    tap(&mut w, Key::S, Modifiers::NONE);
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert!(w.editor.scene().objects.is_empty());
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(w.editor.scene().objects.len(), 1);
    w.editor.selection = vec![id];
    tap(&mut w, Key::K, Modifiers::NONE);
    w.click("Layers");
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert!(w.editor.scene().objects.is_empty());
}

#[test]
fn frame_marquee_highlights_the_whole_range_over_drawings_and_deletes_it_in_one_edit() {
    let mut doc = Document {
        frames: 1000,
        ..Document::default()
    };
    doc.add_rectangle([0.; 2], [15.; 2], [255; 4]);
    crate::commands::add_layer(&mut doc);
    let original = AnimationProject::from_document(doc);
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(original.clone(), None);
    w.click("100%");
    w.frame(vec![Event::Text("5".into())]);
    tap(&mut w, Key::Enter, Modifiers::NONE);
    w.control("5%");
    let cell = w.editor.timeline.frame_width();
    assert_eq!(cell, 3.);
    let first = w.control("20");
    let upper = w.control("Layer 2").y;
    let lower = w.control("Layer 1").y;
    let from = egui::pos2(first.x, upper);
    let to = egui::pos2(first.x + 9. * cell, lower);
    w.click_at(from);
    w.control("Frame 20 selected");
    w.drag(from, to - from);
    w.control("20–29 · 10 frames");
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());

    // Both empty and occupied drawing cells need a visible tint above their fill.
    let painted = w.frame(vec![]);
    let center_x = from.x + cell * 4.;
    for y in [upper, lower] {
        let sample = egui::pos2(center_x, y);
        let visible_rects = painted.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if s.clip_rect.contains(sample) && r.rect.contains(sample) => {
                Some(r)
            }
            _ => None,
        });
        let rects: Vec<_> = visible_rects.collect();
        let hold = rects
            .iter()
            .rposition(|r| r.fill.a() == 255 && r.rect.height() < 30.)
            .unwrap();
        assert!(
            rects[hold + 1..].iter().any(|r| {
                (0 < r.fill.a() && r.fill.a() < 255) && (r.rect.width() - cell * 10.).abs() < 1.
            }),
            "expected all ten selected frames to be tinted above the hold at {sample:?}"
        );
    }
    // Reverse the selection, starting on the numbered ruler and crossing both layers.
    let back = egui::pos2(to.x, first.y);
    w.drag(back, from - back);
    w.control("20–29 · 10 frames");
    assert_eq!(w.editor.timeline.frame, 19);
    let mut expected = original.clone();
    expected.clips[0].delete_range(19, 28).unwrap();
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.project, expected);
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());
}

#[test]
fn deleting_selected_timeline_frames_ripples_the_clip_and_undo_restores_it_once() {
    let (mut w, _) = window();
    let mut project = w.editor.project.clone();
    project.clips[0].set_length(8).unwrap();
    project.clips[0].events.insert(6, vec!["landing".into()]);
    w.editor.replace_project(project.clone(), None);
    w.frame(vec![]);
    let first = w.control("1");
    let cell = w.editor.timeline.frame_width();
    let click = |w: &mut EditorWindow, frame: u32, y: f32, modifiers: Modifiers| {
        let pos = egui::pos2(first.x + frame as f32 * cell, y);
        click_with_modifiers(w, pos, modifiers);
    };
    // Start on a drawing row; extend backwards on the numbered ruler.
    click(&mut w, 4, first.y + 24., Modifiers::NONE);
    click(&mut w, 2, first.y, Modifiers::SHIFT);
    let mut expected = project.clone();
    expected.clips[0].delete_range(2, 4).unwrap();
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.project, expected);
    assert_eq!(w.editor.timeline.frame, 2);
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(w.editor.project, project);
    assert!(!w.editor.can_undo());
    // A whole-clip selection cannot remove its last frame.
    click(&mut w, 0, first.y, Modifiers::NONE);
    click(&mut w, 7, first.y, Modifiers::SHIFT);
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.project, project);
    // Escape releases the range without deleting artwork on the next Delete.
    tap(&mut w, Key::Escape, Modifiers::NONE);
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.project, project);
    assert!(!w.editor.can_undo());
}

#[test]
fn modifier_clicks_keep_disjoint_frame_ranges_and_delete_only_those_ticks() {
    let mut doc = Document {
        frames: 12,
        ..Document::default()
    };
    let artwork = doc.add_rectangle([3.; 2], [8.; 2], [255; 4]);
    let mut original = AnimationProject::from_document(doc);
    for tick in 0..12 {
        original.clips[0]
            .add_event(tick, &format!("pose-{tick}"))
            .unwrap();
    }
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.editor.replace_project(original.clone(), None);
    w.editor.selection = vec![artwork];
    let first = w.control("1");
    let cell = w.editor.timeline.frame_width();
    let pos = |frame: u32| first + egui::vec2((frame - 1) as f32 * cell, 0.);
    let shift = Modifiers::SHIFT;
    let ctrl = Modifiers::CTRL;

    for (frame, modifiers, summary) in [
        (2, Modifiers::NONE, "Frame 2 selected"),
        (5, shift, "2–5 · 4 frames"),
        (3, ctrl, "3 frames · 2 ranges"),
        (6, shift, "3–6 · 4 frames"),
        (10, ctrl, "5 frames · 2 ranges"),
        (12, ctrl | shift, "7 frames · 2 ranges"),
        (11, Modifiers::COMMAND, "6 frames · 3 ranges"),
    ] {
        click_with_modifiers(&mut w, pos(frame), modifiers);
        w.control(summary);
    }
    let drawing_y = w.control("Layer 1").y;
    let painted = w.frame(vec![]);
    // Sample the first selected band's paint in each row, then check which other
    // frames receive it. This also catches a highlight that bleeds into a gap.
    for y in [first.y, drawing_y] {
        let sample = egui::pos2(pos(3).x, y);
        let fill = painted
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if s.clip_rect.contains(sample)
                        && r.rect.contains(sample)
                        && (r.rect.left() - (pos(3).x - cell * 0.5)).abs() < 1.
                        && (r.rect.right() - (pos(6).x + cell * 0.5)).abs() < 1.
                        && r.fill.a() > 0 =>
                {
                    Some(r.fill)
                }
                _ => None,
            })
            .expect("visible selection band from frame 3 through 6");
        let highlighted: Vec<_> = (1..=12)
            .filter(|&frame| {
                let sample = egui::pos2(pos(frame).x, y);
                painted.shapes.iter().any(|s| {
                    matches!(&s.shape, egui::Shape::Rect(r)
                if r.fill == fill && s.clip_rect.contains(sample) && r.rect.contains(sample))
                })
            })
            .collect();
        assert_eq!(highlighted, [3, 4, 5, 6, 10, 12]);
    }
    assert!(
        !w.editor.can_undo(),
        "selecting frames does not edit the document"
    );
    let mut expected = original.clone();
    for (lo, hi) in [(11, 11), (9, 9), (2, 5)] {
        expected.clips[0].delete_range(lo, hi).unwrap();
    }
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(w.editor.project, expected);
    tap(&mut w, Key::Z, Modifiers::COMMAND);
    assert_eq!(w.editor.project, original);
    assert!(!w.editor.can_undo());

    w.drag(pos(8), pos(3) - pos(8));
    click_with_modifiers(&mut w, pos(6), shift);
    w.control("6–8 · 3 frames");
    click_with_modifiers(&mut w, pos(7), Modifiers::NONE);
    click_with_modifiers(&mut w, pos(7), ctrl);
    w.control("No frames selected");
    assert_eq!(w.editor.selection, [artwork]);
    tap(&mut w, Key::Delete, Modifiers::NONE);
    assert_eq!(
        w.editor.project, original,
        "toggling off the last frame retains timeline focus"
    );
}

#[test]
fn f7_refusals_are_visible_near_the_top_at_the_minimum_window_size() {
    for locked in [true, false] {
        let mut doc = Document::default();
        doc.add_rectangle([12.; 2], [9.; 2], [255; 4]);
        doc.layers[0].locked = locked;
        let mut project = AnimationProject::from_document(doc);
        let authored = project.clips[0].cels[&0].clone();
        project.clips[0].cels.insert(1, authored);
        let mut w = EditorWindow::new(egui::vec2(800., 500.));
        w.editor.replace_project(project.clone(), None);
        w.frame(vec![]);
        tap(&mut w, Key::F7, Modifiers::NONE);
        let refusal = if locked {
            "Choose a visible, unlocked drawing layer."
        } else {
            "Frame 2 already starts a drawing on this layer."
        };
        let message = w.control(refusal);
        assert!(
            message.y < 140.,
            "entire refusal should be visible in the issue banner: {message:?}"
        );
        assert_eq!(w.editor.project, project);
        assert!(!w.editor.can_undo());
        tap(&mut w, Key::Period, Modifiers::NONE);
        assert!(w.editor.timeline.error().is_none());
        assert_eq!(w.editor.timeline.frame, 1);
        if !locked {
            tap(&mut w, Key::F7, Modifiers::NONE);
            assert!(w.editor.timeline.error().is_none());
            assert_eq!(w.editor.timeline.frame, 2);
        }
    }
}
