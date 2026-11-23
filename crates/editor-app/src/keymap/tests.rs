use super::*;
use eframe::egui::Modifiers as M;

#[test]
fn every_logical_key_round_trips_using_its_stable_name() {
    for &key in Key::ALL {
        for mods in [M::NONE, M::COMMAND | M::ALT | M::SHIFT] {
            let chord = Chord::from_event(key, mods);
            assert_eq!(
                serde_json::from_value::<Chord>(serde_json::to_value(chord).unwrap()).unwrap(),
                chord
            );
        }
    }
}

#[test]
fn every_factory_alias_has_exactly_one_owner_and_can_be_restored() {
    let mut map = Keymap::default();
    for &action in Action::ALL {
        assert!(!action.name().is_empty() && !action.category().is_empty());
        for &chord in action.defaults() {
            assert_eq!(map.owner(chord), Some(action));
            assert!(chord.check_reserved(action).is_ok());
        }
        map.clear(action);
        for &chord in action.defaults() {
            assert_eq!(map.owner(chord), None);
        }
        map.reset(action).unwrap();
        assert!(map.is_default(action));
    }
}

#[test]
fn custom_shortcuts_replace_aliases_and_reset_checks_every_alias() {
    let mut map = Keymap::default();
    assert_eq!(map.resolve(Key::F6, M::SHIFT), Some(Action::PenTool));
    assert!(map
        .set(Action::Copy, Key::F6, M::SHIFT)
        .unwrap_err()
        .contains("Pen tool"));
    map.set(Action::PenTool, Key::F10, M::NONE).unwrap();
    assert_eq!(map.resolve(Key::B, M::NONE), None);
    assert_eq!(map.resolve(Key::F6, M::SHIFT), None);
    map.set(Action::Copy, Key::F6, M::SHIFT).unwrap();
    let before = map.clone();
    assert!(map.reset(Action::PenTool).is_err());
    assert_eq!(map, before);
    map.reset(Action::Copy).unwrap();
    map.reset(Action::PenTool).unwrap();
    assert_eq!(map, Keymap::default());
}

#[test]
fn logical_punctuation_and_platform_primary_share_bindings() {
    let mut map = Keymap::default();
    for primary in [M::COMMAND, M::COMMAND | M::CTRL, M::COMMAND | M::MAC_CMD] {
        for (key, mods) in [
            (Key::Plus, primary),
            (Key::Plus, primary | M::SHIFT),
            (Key::Equals, primary | M::SHIFT),
        ] {
            assert_eq!(map.resolve(key, mods), Some(Action::Union));
        }
        assert_eq!(
            map.resolve(Key::Slash, primary | M::ALT | M::SHIFT),
            Some(Action::CutPath)
        );
    }
    assert_eq!(map.resolve(Key::Z, M::CTRL), None);
    map.set(Action::CutPath, Key::Equals, M::ALT | M::SHIFT)
        .unwrap();
    assert_eq!(map.resolve(Key::Plus, M::ALT), Some(Action::CutPath));
    assert!(map
        .set(Action::PenTool, Key::Plus, M::ALT | M::SHIFT)
        .is_err());
}

#[test]
fn reserved_bindings_are_rejected_even_if_the_original_action_was_cleared() {
    let mut map = Keymap::default();
    map.clear(Action::Paste);
    map.clear(Action::Union);
    for (key, mods) in [
        (Key::Enter, M::NONE),
        (Key::Tab, M::SHIFT),
        (Key::Escape, M::NONE),
        (Key::V, M::COMMAND),
        (Key::X, M::COMMAND | M::ALT),
        (Key::Copy, M::NONE),
        (Key::Num0, M::COMMAND),
        (Key::Num0, M::COMMAND | M::ALT),
        (Key::Plus, M::COMMAND),
        (Key::Minus, M::MAC_CMD | M::COMMAND | M::CTRL),
        (Key::Equals, M::COMMAND),
    ] {
        assert!(map.set(Action::PenTool, key, mods).is_err());
    }
    #[cfg(windows)]
    for (key, mods) in [
        (Key::F4, M::ALT),
        (Key::Delete, M::SHIFT),
        (Key::Insert, M::SHIFT),
        (Key::Insert, M::CTRL | M::COMMAND),
    ] {
        assert!(map.set(Action::PenTool, key, mods).is_err());
    }
    #[cfg(target_os = "macos")]
    for (key, mods) in [
        (Key::Q, M::COMMAND),
        (Key::H, M::COMMAND),
        (Key::H, M::COMMAND | M::ALT),
    ] {
        assert!(map.set(Action::PenTool, key, mods).is_err());
    }
    map.reset(Action::Paste).unwrap();
    map.reset(Action::Union).unwrap();
}

#[test]
fn preferences_preserve_swaps_clears_and_valid_entries_alongside_unknown_entries() {
    let raw = serde_json::json!({
        "open": "Primary+S", "save": "Primary+O", "pen_tool": null, "rectangle_tool": "Alt+F10",
        "future_action": "Q", "front": "FutureKey", "raise": "Primary+C", "lower": "Y"
    });
    let map: Keymap = serde_json::from_value(raw).unwrap();
    assert_eq!(map.resolve(Key::O, M::COMMAND), Some(Action::Save));
    assert_eq!(map.resolve(Key::S, M::COMMAND), Some(Action::Open));
    assert_eq!(map.resolve(Key::B, M::NONE), None);
    assert_eq!(map.resolve(Key::F10, M::ALT), Some(Action::RectangleTool));
    assert!(map.is_default(Action::Front));
    assert!(map.is_default(Action::Raise));
    assert_eq!(map.resolve(Key::Y, M::NONE), Some(Action::Lower));
    assert_eq!(
        serde_json::from_str::<Keymap>(&serde_json::to_string(&map).unwrap()).unwrap(),
        map
    );
    assert_eq!(serde_json::to_string(&Keymap::default()).unwrap(), "{}");
}

#[test]
fn invalid_saved_conflicts_restore_defaults_without_shadowing_other_commands() {
    let map: Keymap = serde_json::from_value(serde_json::json!({
        "open": "F1", "save": "Primary+O", "align": "F9", "front": "F9",
        "pen_tool": "Enter", "duplicate": "Alt+F10"
    }))
    .unwrap();
    assert_eq!(map.resolve(Key::F1, M::NONE), Some(Action::SelectTool));
    assert!(map.is_default(Action::Open) && map.is_default(Action::Save));
    assert_eq!(map.resolve(Key::F9, M::NONE), Some(Action::Align));
    assert!(map.is_default(Action::Front) && map.is_default(Action::PenTool));
    assert_eq!(map.resolve(Key::F10, M::ALT), Some(Action::Duplicate));
}

#[test]
fn merging_window_preferences_preserves_unrelated_edits_and_is_atomic() {
    let original = Keymap::default();
    let mut live = original.clone();
    live.set(Action::Copy, Key::F9, M::NONE).unwrap();
    let mut edited = original.clone();
    edited.set(Action::Undo, Key::F10, M::NONE).unwrap();
    live.merge_changes(&original, &edited).unwrap();
    assert_eq!(live.resolve(Key::F9, M::NONE), Some(Action::Copy));
    assert_eq!(live.resolve(Key::F10, M::NONE), Some(Action::Undo));
    let before = live.clone();
    edited.set(Action::Redo, Key::F9, M::NONE).unwrap();
    assert!(live.merge_changes(&original, &edited).is_err());
    assert_eq!(live, before);
    edited.set(Action::Copy, Key::F11, M::NONE).unwrap();
    assert!(live.merge_changes(&original, &edited).is_err());
    assert_eq!(live, before);
}
