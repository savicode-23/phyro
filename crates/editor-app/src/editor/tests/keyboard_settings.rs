use super::*;
use crate::keymap::{self, Action};
use egui::{Event, Key, Modifiers};

fn key(w: &mut EditorWindow, key: Key, modifiers: Modifiers, pressed: bool) {
    w.frame_with_input(egui::RawInput {
        modifiers,
        events: vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    });
}

fn search(w: &mut EditorWindow, query: &str) {
    w.ctx.style_mut(|style| style.animation_time = 0.);
    w.frame(vec![]);
    w.click_edit("Keyboard shortcuts…");
    w.frame(vec![]);
    w.frame(vec![Event::Text(query.into())]);
}

#[test]
fn shortcuts_dialog_searches_captures_conflicts_and_applies_only_on_request() {
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    let original = w.editor.project.clone();
    search(&mut w, "Pen tool");
    w.click("Pen tool");
    w.click("Change shortcut");
    key(&mut w, Key::N, Modifiers::NONE, true);
    key(&mut w, Key::N, Modifiers::NONE, false);
    w.control("This shortcut is assigned to Nodes tool.");
    w.click("Edit conflicting command");
    w.click("Clear");
    w.control("Unassigned");
    w.click("Reset");
    w.click("Change shortcut");
    key(&mut w, Key::F12, Modifiers::NONE, true);
    key(&mut w, Key::F12, Modifiers::NONE, false);
    w.control("F12");
    assert_eq!(
        keymap::active(&w.ctx).resolve(Key::F12, Modifiers::NONE),
        None
    );
    let tool = w.editor.tool;
    w.click("Apply");
    assert_eq!(
        keymap::active(&w.ctx).resolve(Key::F12, Modifiers::NONE),
        Some(Action::NodeTool)
    );
    assert!(w.editor.tool == tool);
    // Paint the workspace again after the modal closes.
    w.frame(vec![]);
    key(&mut w, Key::F12, Modifiers::NONE, true);
    key(&mut w, Key::F12, Modifiers::NONE, false);
    assert!(w.editor.tool == Tool::Node);
    search(&mut w, "F12");
    w.control("Nodes tool");
    w.click("Reset all");
    w.click("Cancel");
    assert_eq!(
        keymap::active(&w.ctx).resolve(Key::F12, Modifiers::NONE),
        Some(Action::NodeTool)
    );
    search(&mut w, "F12");
    w.click("Reset all");
    w.click("Apply");
    assert_eq!(*keymap::active(&w.ctx), Default::default());
    assert_eq!(w.editor.project, original);
}

#[test]
fn shortcut_capture_consumes_zoom_and_native_clipboard_events() {
    let mut w = EditorWindow::new(egui::vec2(800., 500.));
    search(&mut w, "Paste style");
    w.click("Paste style");
    w.click("Clear");
    w.click("Change shortcut");
    let mods = Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT;
    w.frame_with_input(egui::RawInput {
        modifiers: mods,
        events: vec![Event::Paste("clipboard content".into())],
        ..Default::default()
    });
    // The right Shift key is released before V.
    key(&mut w, Key::V, Modifiers::CTRL | Modifiers::COMMAND, false);
    w.control(&keymap::active(&w.ctx).label(&w.ctx, Action::PasteStyle));
    w.click("Change shortcut");
    let zoom = w.ctx.zoom_factor();
    key(
        &mut w,
        Key::Minus,
        Modifiers::CTRL | Modifiers::COMMAND,
        true,
    );
    key(
        &mut w,
        Key::Minus,
        Modifiers::CTRL | Modifiers::COMMAND,
        false,
    );
    w.control("The interface zoom controls use this shortcut. Choose another key.");
    assert_eq!(w.ctx.zoom_factor(), zoom);
    w.click("Cancel");
    assert_eq!(*keymap::active(&w.ctx), Default::default());
}
