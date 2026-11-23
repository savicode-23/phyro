use super::*;
use egui::{Event, Key, Modifiers, OutputCommand, ViewportCommand};

fn window() -> (EditorWindow, Document) {
    let mut drawing = Document::default();
    let first = drawing.add_rectangle([10.; 2], [20.; 2], [128; 4]);
    let second = drawing.add_rectangle([40.; 2], [20.; 2], [255; 4]);
    commands::group(&mut drawing, &[first, second]);
    drawing.add_rectangle([80.; 2], [20.; 2], [66; 4]);
    drawing.objects[2].layer = commands::add_layer(&mut drawing);
    drawing.layers[1].locked = true;
    let mut w = EditorWindow::new(egui::vec2(1100., 740.));
    w.ctx.style_mut(|s| s.animation_time = 0.);
    w.editor
        .replace_project(AnimationProject::from_document(drawing.clone()), None);
    w.editor.set_tool(Tool::Select);
    w.editor.selection = vec![first];
    w.frame(vec![]);
    (w, drawing)
}

fn key(k: Key, modifiers: Modifiers) -> Vec<Event> {
    [true, false]
        .map(|pressed| Event::Key {
            key: k,
            physical_key: Some(k),
            modifiers,
            pressed,
            repeat: false,
        })
        .to_vec()
}

fn choose(w: &mut EditorWindow, label: &str) -> egui::FullOutput {
    w.click("Edit");
    let pos = w.control(label);
    w.frame(vec![Event::PointerMoved(pos)]);
    let event = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    w.frame(vec![event(true)]);
    w.frame(vec![event(false)])
}

#[track_caller]
fn copied(output: egui::FullOutput) -> String {
    output
        .platform_output
        .commands
        .into_iter()
        .find_map(|command| match command {
            OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .expect("copied to the system clipboard")
}

#[test]
fn selecting_pasting_and_ungrouping_artwork_take_delete_back_from_the_timeline() {
    let (mut w, original) = window();
    let copy = copied(choose(&mut w, "Copy"));
    let length = w.editor.project.clips[0].frames;

    w.click("2");
    w.frame(key(Key::A, Modifiers::COMMAND));
    w.frame(key(Key::Delete, Modifiers::NONE));
    assert_eq!(w.editor.project.clips[0].frames, length);
    assert_eq!(w.editor.scene().objects, original.objects[2..]);
    w.frame(key(Key::Z, Modifiers::COMMAND));

    w.click("2");
    w.frame(vec![Event::Paste(copy)]);
    assert!(w.editor.scene().objects.len() > original.objects.len());
    w.frame(key(Key::Delete, Modifiers::NONE));
    assert_eq!(w.editor.project.clips[0].frames, length);
    assert_eq!(w.editor.scene().objects, original.objects);

    w.frame(key(Key::A, Modifiers::COMMAND));
    w.click("2");
    w.frame(key(Key::G, Modifiers::COMMAND | Modifiers::SHIFT));
    assert!(w.editor.scene().objects[0].group.is_none());
    w.frame(key(Key::Delete, Modifiers::NONE));
    assert_eq!(w.editor.project.clips[0].frames, length);
    assert_eq!(w.editor.scene().objects, original.objects[2..]);
}

#[test]
fn menu_and_native_shortcuts_share_group_clipboard_and_single_step_history() {
    let (mut w, original) = window();
    let editable: Vec<_> = original.objects[..2].iter().map(|o| o.id).collect();
    choose(&mut w, "Select All");
    assert_eq!(w.editor.selection, editable);
    choose(&mut w, "Deselect");
    assert!(w.editor.selection.is_empty());
    assert!(!w.editor.can_undo());
    w.frame(key(Key::A, Modifiers::COMMAND));
    assert_eq!(w.editor.selection, editable);
    let copy = copied(choose(&mut w, "Copy"));
    assert_eq!(Clipboard::decode(&copy).unwrap().encode().unwrap(), copy);

    // Backends may send a raw shortcut alongside their normalized event.
    w.editor.selection = vec![editable[0]];
    let mut cut = key(Key::X, Modifiers::COMMAND);
    cut.push(Event::Cut);
    assert_eq!(copied(w.frame(cut)), copy);
    assert_eq!(w.editor.scene().objects, original.objects[2..]);
    choose(&mut w, "Undo");
    assert_eq!(w.editor.scene(), &original);
    assert!(!w.editor.can_undo());
    choose(&mut w, "Redo");
    assert_eq!(w.editor.scene().objects, original.objects[2..]);

    let request = choose(&mut w, "Paste");
    assert!(request
        .viewport_output
        .values()
        .flat_map(|v| &v.commands)
        .any(|c| matches!(c, ViewportCommand::RequestPaste)));
    w.frame(vec![Event::Paste("unrelated clipboard text".into())]);
    assert_eq!(w.editor.scene().objects, original.objects[2..]);
    w.frame(vec![Event::Paste(copy)]);
    let pasted = w.editor.scene().clone();
    assert_eq!(pasted.objects.len(), 3);
    assert_eq!(pasted.objects[1].group, pasted.objects[2].group);
    assert!(pasted.objects[1].group.is_some());
    assert_eq!(pasted.objects[1].position, original.objects[0].position);
    assert_eq!(pasted.objects[2].position, original.objects[1].position);
    assert_ne!(w.editor.selection, editable);
    choose(&mut w, "Duplicate");
    assert_eq!(w.editor.scene().objects.len(), 5);
    choose(&mut w, "Delete");
    assert_eq!(w.editor.scene().objects, pasted.objects);
    w.frame(key(Key::Z, Modifiers::COMMAND));
    assert_eq!(w.editor.scene().objects.len(), 5);
    w.frame(key(Key::Y, Modifiers::COMMAND));
    assert_eq!(w.editor.scene().objects, pasted.objects);
    w.frame(key(Key::A, Modifiers::COMMAND));
    w.frame(key(Key::Escape, Modifiers::NONE));
    assert!(w.editor.selection.is_empty());
}

#[test]
fn browser_images_paste_once_with_or_without_text_and_leave_style_and_fields_alone() {
    let (mut w, _) = window();
    let native_copy = copied(choose(&mut w, "Copy"));
    w.editor.image_clipboard.read = || {
        let image =
            document_core::RasterImage::from_rgba(2, 1, vec![250, 88, 11, 127, 4, 8, 12, 0])?;
        Ok(Some(std::sync::Arc::new(image)))
    };
    let images = |w: &EditorWindow| {
        w.editor
            .scene()
            .objects
            .iter()
            .filter(|o| o.image.is_some())
            .count()
    };
    let release = |modifiers| key(Key::V, modifiers).pop().unwrap();
    // The desktop backend swallows the key press on an image-only clipboard.
    w.frame(vec![release(Modifiers::COMMAND)]);
    assert_eq!(images(&w), 1);
    let pasted = w.editor.scene().objects.last().unwrap();
    assert_eq!(
        pasted.image.as_ref().unwrap().rgba(),
        &[125, 44, 5, 127, 0, 0, 0, 0]
    );
    assert_eq!(w.editor.selection, [pasted.id]);
    assert!(!pasted.reference_only);
    // A browser can also publish HTML or a source URL alongside its bitmap.
    w.frame(vec![Event::Paste(
        "https://example.invalid/picture.png".into(),
    )]);
    w.frame(vec![release(Modifiers::COMMAND)]);
    assert_eq!(images(&w), 2);
    choose(&mut w, "Paste");
    assert_eq!(images(&w), 3);
    // A valid Phyro clipboard payload has priority over fallback pixels.
    w.frame(vec![Event::Paste(native_copy), release(Modifiers::COMMAND)]);
    assert_eq!(images(&w), 3);
    let mut style = vec![Event::Paste("browser text".into())];
    style.push(release(Modifiers::COMMAND | Modifiers::SHIFT));
    w.frame(style);
    assert_eq!(images(&w), 3);
    w.click("12 fps");
    w.frame(vec![]);
    w.frame(vec![Event::Paste("30".into()), release(Modifiers::COMMAND)]);
    assert_eq!(images(&w), 3);
}

#[test]
fn menu_and_shortcuts_group_and_ungroup_in_place_with_one_undo() {
    let (mut w, grouped) = window();
    let members: Vec<_> = grouped.objects[..2].iter().map(|o| o.id).collect();
    let mut separate = grouped.clone();
    for o in &mut separate.objects[..2] {
        o.group = None;
    }
    // Selecting even one member of a group acts on the entire group.
    w.click("Object");
    w.click("Ungroup");
    assert_eq!(w.editor.scene(), &separate);
    assert_eq!(w.editor.selection, members);
    w.frame(key(Key::Z, Modifiers::COMMAND));
    assert_eq!(w.editor.scene(), &grouped);
    assert!(!w.editor.can_undo());
    w.editor.selection = vec![members[1]];
    w.frame(key(Key::G, Modifiers::COMMAND | Modifiers::SHIFT));
    assert_eq!(w.editor.scene(), &separate);
    assert_eq!(w.editor.selection, members);
    w.click("Object");
    w.click("Group");
    assert_eq!(w.editor.scene(), &grouped);
    w.frame(key(Key::Z, Modifiers::COMMAND));
    assert_eq!(w.editor.scene(), &separate);
    w.editor.selection = members.clone();
    w.frame(key(Key::G, Modifiers::COMMAND));
    assert_eq!(w.editor.scene(), &grouped);
    assert_eq!(w.editor.selection, members);
}

#[test]
fn reassigned_clipboard_and_edit_keys_use_the_active_map_and_leave_typing_alone() {
    use crate::keymap::{self, Action, Keymap};
    let (mut w, original) = window();
    let mut map = Keymap::default();
    for (action, key) in [
        (Action::Copy, Key::F9),
        (Action::Paste, Key::F10),
        (Action::Duplicate, Key::J),
    ] {
        map.set(action, key, Modifiers::NONE).unwrap();
    }
    keymap::install(&w.ctx, std::sync::Arc::new(map));
    let old_copy = w.frame(vec![Event::Copy]);
    assert!(!old_copy
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, OutputCommand::CopyText(_))));
    w.frame(key(Key::D, Modifiers::COMMAND));
    assert_eq!(w.editor.scene(), &original);
    let payload = copied(w.frame(key(Key::F9, Modifiers::NONE)));
    w.frame(vec![Event::Paste(payload.clone())]);
    assert_eq!(w.editor.scene(), &original);
    let request = w.frame(key(Key::F10, Modifiers::NONE));
    assert!(request
        .viewport_output
        .values()
        .flat_map(|v| &v.commands)
        .any(|cmd| matches!(cmd, ViewportCommand::RequestPaste)));
    w.frame(vec![Event::Paste(payload)]);
    assert_eq!(w.editor.scene().objects.len(), 5);
    w.frame(key(Key::J, Modifiers::NONE));
    assert_eq!(w.editor.scene().objects.len(), 7);
    w.click("12 fps");
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    w.frame(key(Key::J, Modifiers::NONE));
    w.frame(vec![Event::Copy]);
    assert_eq!(w.editor.scene().objects.len(), 7);
}

#[test]
fn edit_menu_cut_and_raw_cut_restore_once_and_text_entry_keeps_native_cut() {
    let (mut w, drawing) = window();
    let menu = copied(choose(&mut w, "Cut"));
    assert_eq!(w.editor.scene().objects.len(), 1);
    w.frame(key(Key::Z, Modifiers::COMMAND));
    assert_eq!(w.editor.scene(), &drawing);
    assert!(!w.editor.can_undo());
    w.editor.selection = vec![drawing.objects[1].id];
    assert_eq!(copied(w.frame(key(Key::X, Modifiers::COMMAND))), menu);
    assert_eq!(w.editor.scene().objects.len(), 1);
    w.frame(key(Key::Z, Modifiers::COMMAND));
    w.frame(key(Key::A, Modifiers::COMMAND));
    let selected = w.editor.selection.clone();
    w.click("12 fps");
    w.frame(vec![]);
    assert!(w.ctx.wants_keyboard_input());
    w.frame(vec![Event::Cut]);
    assert_eq!(w.editor.scene(), &drawing);
    assert_eq!(w.editor.selection, selected);
    assert_eq!(w.editor.clipboard.encode().unwrap(), menu);
    assert!(!w.editor.can_undo());
}

#[test]
fn escape_closes_a_menu_or_cancels_the_pen_before_clearing_the_selection() {
    let (mut w, _) = window();
    let selected = w.editor.selection.clone();
    w.click("Edit");
    w.control("Deselect");
    w.frame(key(Key::Escape, Modifiers::NONE));
    assert_eq!(w.editor.selection, selected, "Escape closed the Edit menu");
    w.frame(key(Key::B, Modifiers::NONE));
    let center = w.board().center();
    w.click_at(center);
    assert!(w.editor.canvas.pen_in_progress());
    w.click("Edit");
    w.frame(key(Key::Escape, Modifiers::NONE));
    assert!(
        w.editor.canvas.pen_in_progress(),
        "closing the menu keeps the active stroke"
    );
    w.frame(key(Key::Escape, Modifiers::NONE));
    assert!(!w.editor.canvas.pen_in_progress());
    w.frame(key(Key::A, Modifiers::COMMAND));
    assert!(!w.editor.selection.is_empty());
    w.frame(key(Key::Escape, Modifiers::NONE));
    assert!(w.editor.selection.is_empty());
}

#[test]
fn a_cut_over_the_system_clipboard_limit_pastes_only_from_its_own_marker() {
    let mut source = Document {
        frames: 1,
        ..Default::default()
    };
    source.add_rectangle([0.; 2], [8.; 2], [200; 4]);
    // These shared drawings fit comfortably in memory but are repeated in the portable JSON.
    source.guides_x = vec![12.345678; 64 * 1024];
    let artwork = std::sync::Arc::new(source);
    let mut doc = Document::default();
    for _ in 0..32 {
        doc.add_rectangle([8.; 2], [8.; 2], [255; 4]);
        doc.objects.last_mut().unwrap().embedded_artwork = Some(artwork.clone().into());
    }
    let ctx = egui::Context::default();
    let mut editor = Editor::new(&ctx);
    editor.replace_project(AnimationProject::from_document(doc), None);
    editor.selection = editor.scene().objects.iter().map(|o| o.id).collect();
    let dispatch = |editor: &mut Editor, event| {
        ctx.run(
            egui::RawInput {
                events: vec![event],
                ..Default::default()
            },
            |ctx| editor.shortcuts(ctx),
        )
    };
    let token = copied(dispatch(&mut editor, Event::Cut));
    assert!(token.starts_with("Phyro/session-objects/1/"));
    assert!(editor.scene().objects.is_empty());
    dispatch(&mut editor, Event::Paste("unrelated external text".into()));
    assert!(editor.scene().objects.is_empty());
    dispatch(&mut editor, Event::Paste(token.clone()));
    assert_eq!(editor.scene().objects.len(), 32);
    assert_eq!(editor.scene().objects[0].static_artwork(), Some(&artwork));
    editor.restore(false);
    editor.restore(false);
    assert_eq!(editor.scene().objects.len(), 32);
    assert!(!editor.can_undo());

    let (mut other, original) = window();
    other.frame(vec![Event::Paste(token)]);
    assert_eq!(other.editor.scene(), &original);
}

#[test]
fn lettering_and_numeric_fields_receive_edit_menu_actions_while_the_project_waits_for_commit() {
    let (mut w, original) = window();
    w.click("Text");
    let center = w.board().center();
    w.click_at(center);
    w.frame(vec![Event::Text("Lamp".into())]);
    choose(&mut w, "Select All");
    assert_eq!(copied(choose(&mut w, "Copy")), "Lamp");
    assert_eq!(copied(choose(&mut w, "Cut")), "Lamp");
    assert_eq!(w.editor.scene(), &original);
    let request = choose(&mut w, "Paste");
    assert!(request
        .viewport_output
        .values()
        .flat_map(|v| &v.commands)
        .any(|c| matches!(c, ViewportCommand::RequestPaste)));
    w.frame(vec![Event::Paste("Light".into())]);
    choose(&mut w, "Select All");
    // TextEdit coalesces rapid typing and cursor movement until the buffer settles.
    let settled = w.ctx.input(|i| i.time) + 1.5;
    w.frame_with_input(egui::RawInput {
        time: Some(settled),
        ..Default::default()
    });
    choose(&mut w, "Delete");
    choose(&mut w, "Undo");
    choose(&mut w, "Select All");
    assert_eq!(copied(choose(&mut w, "Copy")), "Light");
    choose(&mut w, "Redo");
    w.frame(vec![Event::Text("Glow".into())]);
    choose(&mut w, "Select All");
    choose(&mut w, "Deselect");
    w.frame(vec![Event::Text("!".into())]);
    let size = w.field_on_row("Size", " px");
    w.click_at(size);
    choose(&mut w, "Select All");
    let number = copied(choose(&mut w, "Copy"));
    assert!(
        number.parse::<f32>().is_ok(),
        "format size field copied {number:?}"
    );
    choose(&mut w, "Paste");
    w.frame(vec![Event::Paste("36".into())]);
    w.frame(key(Key::Enter, Modifiers::NONE));
    assert!(w.editor.canvas.text_active());
    w.frame(Vec::new());
    w.frame(key(Key::Enter, Modifiers::COMMAND));
    assert!(
        !w.editor.canvas.text_active(),
        "the lettering session committed"
    );
    let text = w
        .editor
        .scene()
        .objects
        .iter()
        .find_map(|o| o.text.as_ref())
        .unwrap();
    assert_eq!((text.content.as_str(), text.size), ("Glow!", 36.));
    w.editor.restore(false);
    assert_eq!(w.editor.scene(), &original);
    assert!(!w.editor.can_undo());
}

#[test]
fn native_style_paste_resolves_once_and_ordinary_paste_replaces_an_unanswered_request() {
    use crate::editor::edit_commands::{clipboard_input, request_paste, ClipboardInput};
    let ctx = egui::Context::default();
    let paint = Modifiers::COMMAND | Modifiers::SHIFT;
    let receive = |events, modifiers| {
        let mut actions = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                actions = clipboard_input(ctx, false);
                for a in &actions {
                    if let ClipboardInput::RequestPaste { style } = a {
                        request_paste(ctx, *style);
                    }
                }
            },
        );
        actions
    };
    let mut combined = key(Key::V, paint);
    combined.push(Event::Paste("both".into()));
    assert!(matches!(
        receive(combined, Modifiers::NONE).as_slice(),
        [ClipboardInput::Paste { style: true, .. }]
    ));
    let mut released = key(Key::V, paint);
    released.remove(0);
    released.insert(0, Event::Paste("native".into()));
    assert!(matches!(
        receive(released, Modifiers::NONE).as_slice(),
        [ClipboardInput::Paste { style: true, .. }]
    ));
    assert!(matches!(
        receive(key(Key::V, paint), paint).as_slice(),
        [ClipboardInput::RequestPaste { style: true }]
    ));
    assert!(receive(Vec::new(), Modifiers::COMMAND).is_empty());
    assert!(matches!(
        receive(vec![Event::Paste("reply".into())], Modifiers::COMMAND).as_slice(),
        [ClipboardInput::Paste { style: true, .. }]
    ));
    receive(key(Key::V, paint), paint);
    receive(key(Key::V, Modifiers::COMMAND), Modifiers::COMMAND);
    assert!(matches!(
        receive(vec![Event::Paste("plain".into())], Modifiers::NONE).as_slice(),
        [ClipboardInput::Paste { style: false, .. }]
    ));
    receive(key(Key::V, paint), paint);
    receive(vec![Event::Text("other input".into())], Modifiers::NONE);
    assert!(receive(vec![Event::Paste("canceled reply".into())], Modifiers::NONE).is_empty());
    assert!(matches!(
        receive(vec![Event::Paste("new paste".into())], Modifiers::NONE).as_slice(),
        [ClipboardInput::Paste { style: false, .. }]
    ));
}
