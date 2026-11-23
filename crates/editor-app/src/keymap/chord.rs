use super::{Action, Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

pub(super) const NONE: u8 = 0;
pub(super) const PRIMARY: u8 = 1;
pub(super) const SHIFT: u8 = 2;
pub(super) const ALT: u8 = 4;
const CONTROL: u8 = 8;
pub(super) const TAB_CONTROL: u8 = if cfg!(target_os = "macos") {
    CONTROL
} else {
    PRIMARY
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct Chord {
    key: Key,
    mods: u8,
}

impl Chord {
    pub(super) const fn new(key: Key, mods: u8) -> Self {
        Self { key, mods }
    }

    fn normalized(key: Key, mods: u8) -> Self {
        // Different input backends report a typed '+' as Plus, Shift+Plus,
        // or Shift+Equals. All three must resolve and conflict identically.
        if key == Key::Plus || (key == Key::Equals && mods & SHIFT != 0) {
            Self::new(Key::Plus, mods & !SHIFT)
        } else {
            Self::new(key, mods)
        }
    }

    pub(super) fn from_event(key: Key, modifiers: Modifiers) -> Self {
        let mut mods = NONE;
        if modifiers.command || modifiers.mac_cmd {
            mods |= PRIMARY;
        }
        if modifiers.shift {
            mods |= SHIFT;
        }
        if modifiers.alt {
            mods |= ALT;
        }
        // egui sets both ctrl and command for Control on Windows/Linux. A
        // distinct physical Control is usable on macOS, including Ctrl+Cmd.
        if modifiers.ctrl && (!modifiers.command || modifiers.mac_cmd) {
            mods |= CONTROL;
        }
        Self::normalized(key, mods)
    }

    pub(super) fn shortcut(self) -> KeyboardShortcut {
        KeyboardShortcut::new(
            Modifiers {
                command: self.mods & PRIMARY != 0,
                shift: self.mods & SHIFT != 0,
                alt: self.mods & ALT != 0,
                ctrl: self.mods & CONTROL != 0,
                mac_cmd: false,
            },
            self.key,
        )
    }

    pub(super) fn check_reserved(self, action: Action) -> Result<(), String> {
        match self.key {
            Key::Escape if self.mods != NONE || action != Action::Deselect => {
                return Err("Escape is reserved for canceling an edit.".into());
            }
            Key::Tab if self.mods & !SHIFT == TAB_CONTROL => {}
            Key::Tab | Key::Enter => {
                return Err(
                    "Tab and Enter are reserved for navigating and confirming edits.".into(),
                );
            }
            _ => {}
        }

        let native = matches!(self.key, Key::Copy | Key::Cut | Key::Paste)
            || (self.mods & PRIMARY != 0 && matches!(self.key, Key::C | Key::X | Key::V))
            || (cfg!(windows)
                && ((self.key == Key::Delete && self.mods & SHIFT != 0)
                    || (self.key == Key::Insert && self.mods & (PRIMARY | CONTROL | SHIFT) != 0)));
        if native && !action.defaults().contains(&self) {
            return Err("The native clipboard uses this shortcut. Choose another key.".into());
        }

        // winit forwards Alt+F4 to the Windows window manager. Its standard
        // macOS application menu handles Quit, Hide and Hide Others directly.
        let window_shortcut = (cfg!(windows) && self.key == Key::F4 && self.mods & !SHIFT == ALT)
            || (cfg!(target_os = "macos")
                && ((self.key == Key::Q && self.mods == PRIMARY)
                    || (self.key == Key::H
                        && (self.mods == PRIMARY || self.mods == (PRIMARY | ALT)))));
        if window_shortcut {
            return Err(
                "The operating system uses this window shortcut. Choose another key.".into(),
            );
        }

        // egui uses matches_logically for interface zoom. With Primary down
        // it accepts additional Alt, Shift, and physical Control modifiers.
        let gui_zoom = self.mods & PRIMARY != 0
            && matches!(self.key, Key::Num0 | Key::Equals | Key::Plus | Key::Minus);
        if gui_zoom && !action.defaults().contains(&self) {
            return Err(
                "The interface zoom controls use this shortcut. Choose another key.".into(),
            );
        }
        Ok(())
    }
}

// Use stable key names and the platform-neutral Primary modifier in the
// preference file. Display strings can vary with OS and accessibility options.
const MODIFIER_NAMES: &[(u8, &str)] = &[
    (PRIMARY, "Primary"),
    (CONTROL, "Control"),
    (ALT, "Alt"),
    (SHIFT, "Shift"),
];

impl Serialize for Chord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut name = String::new();
        for &(flag, modifier) in MODIFIER_NAMES {
            if self.mods & flag != 0 {
                name.push_str(modifier);
                name.push('+');
            }
        }
        name.push_str(self.key.name());
        serializer.serialize_str(&name)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let mut parts = text.rsplit('+');
        let key = parts
            .next()
            .and_then(Key::from_name)
            .ok_or_else(|| serde::de::Error::custom("Unknown shortcut key"))?;
        let mut mods = NONE;
        for part in parts {
            let &(flag, _) = MODIFIER_NAMES
                .iter()
                .find(|(_, name)| *name == part)
                .ok_or_else(|| serde::de::Error::custom("Unknown shortcut modifier"))?;
            if mods & flag != 0 {
                return Err(serde::de::Error::custom("Repeated shortcut modifier"));
            }
            mods |= flag;
        }
        Ok(Self::normalized(key, mods))
    }
}
