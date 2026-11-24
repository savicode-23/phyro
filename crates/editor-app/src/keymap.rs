//! Editor command shortcuts, including platform-neutral preferences and factory aliases.
use eframe::egui::{Context, Event, Id, Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

mod chord;
use chord::{Chord, ALT, NONE, PRIMARY, SHIFT, TAB_CONTROL};
#[cfg(test)]
mod tests;

// The first binding is shown in compact menus. Alternate keys remain active
// until the user assigns a replacement to the command.
macro_rules! catalog {
    ($($category:literal { $($id:ident: $name:literal => [$($key:ident($mods:expr)),+]),+ $(,)? })+) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum Action { $($($id),+),+ }

        impl Action {
            pub(crate) const ALL: &'static [Self] = &[$($(Self::$id),+),+];
            pub(crate) fn name(self) -> &'static str {
                match self { $($(Self::$id => $name),+),+ }
            }
            pub(crate) fn category(self) -> &'static str {
                match self { $($(Self::$id)|+ => $category),+ }
            }
            fn defaults(self) -> &'static [Chord] {
                match self { $($(Self::$id => const { &[$(Chord::new(Key::$key, $mods)),+] }),+),+ }
            }
        }
    };
}

catalog! {
    "File" {
        Open: "Open project" => [O(PRIMARY)],
        Save: "Save project" => [S(PRIMARY)],
        SaveAs: "Save project as" => [S(PRIMARY | SHIFT)],
        NextTab: "Next document tab" => [Tab(TAB_CONTROL)],
        PreviousTab: "Previous document tab" => [Tab(TAB_CONTROL | SHIFT)],
        CloseTab: "Close document tab" => [W(PRIMARY)],
    }
    "Edit" {
        Undo: "Undo" => [Z(PRIMARY)],
        Redo: "Redo" => [Z(PRIMARY | SHIFT), Y(PRIMARY)],
        Cut: "Cut" => [X(PRIMARY)],
        Copy: "Copy" => [C(PRIMARY)],
        Paste: "Paste" => [V(PRIMARY)],
        PasteStyle: "Paste style" => [V(PRIMARY | SHIFT)],
        Duplicate: "Duplicate" => [D(PRIMARY)],
        Delete: "Delete" => [Delete(NONE)],
        Group: "Group" => [G(PRIMARY)],
        Ungroup: "Ungroup" => [G(PRIMARY | SHIFT)],
        SelectAll: "Select all" => [A(PRIMARY)],
        Deselect: "Deselect" => [Escape(NONE)],
        Align: "Align and distribute" => [A(PRIMARY | SHIFT)],
    }
    "View" {
        Grayscale: "Toggle grayscale preview" => [G(SHIFT)],
    }
    "Tools" {
        SelectTool: "Select tool" => [S(NONE), F1(NONE)],
        NodeTool: "Nodes tool" => [N(NONE), F2(NONE)],
        RectangleTool: "Rectangle tool" => [R(NONE), F4(NONE)],
        EllipseTool: "Ellipse tool" => [E(NONE), F5(NONE)],
        PenTool: "Pen tool" => [B(NONE), F6(SHIFT)],
        PencilTool: "Freehand pencil" => [P(NONE), F6(NONE)],
        TextTool: "Text tool" => [T(NONE), F8(NONE)],
        EyedropperTool: "Color picker" => [D(NONE)],
        GradientTool: "Gradient tool" => [G(NONE)],
    }
    "Objects" {
        Front: "Raise to front" => [Home(NONE)],
        Raise: "Raise one step" => [PageUp(NONE)],
        Lower: "Lower one step" => [PageDown(NONE)],
        Back: "Lower to back" => [End(NONE)],
        ConvertToSymbol: "Convert to symbol" => [Y(PRIMARY | SHIFT)],
        LockObjects: "Lock selected artwork" => [L(NONE)],
        FlipH: "Flip horizontally" => [H(NONE)],
        FlipV: "Flip vertically" => [V(NONE)],
    }
    "Paths" {
        Union: "Union" => [Plus(PRIMARY)],
        Difference: "Difference" => [Minus(PRIMARY), Minus(PRIMARY | SHIFT)],
        Intersection: "Intersection" => [Num8(PRIMARY | SHIFT)],
        Exclusion: "Exclusion" => [Num6(PRIMARY | SHIFT)],
        Division: "Division" => [Slash(PRIMARY), Slash(PRIMARY | SHIFT)],
        CutPath: "Cut path" => [Slash(PRIMARY | ALT), Slash(PRIMARY | ALT | SHIFT)],
        ConvertToPath: "Convert to path" => [C(PRIMARY | SHIFT)],
        StrokeToPath: "Stroke to path" => [C(PRIMARY | ALT)],
        SimplifyPath: "Simplify" => [L(PRIMARY)],
        CombineNodes: "Combine selected nodes" => [J(SHIFT)],
        Corner: "Make nodes corner" => [C(SHIFT)],
        Smooth: "Make nodes smooth" => [S(SHIFT)],
        Symmetric: "Make nodes symmetric" => [Y(SHIFT)],
    }
    "Animation" {
        Play: "Play / pause" => [Space(NONE)],
        PreviousFrame: "Previous frame" => [Comma(NONE)],
        NextFrame: "Next frame" => [Period(NONE)],
        Keyframe: "Create keyframe" => [K(NONE)],
        BlankFrame: "Create blank frame" => [F7(NONE), K(SHIFT)],
        DuplicateFrames: "Copy layer drawing" => [D(SHIFT)],
        Onion: "Toggle onion skin" => [O(NONE)],
        OnionSettings: "Onion skin settings" => [O(SHIFT)],
        PreviousSkins: "Cycle previous onion skins" => [Comma(ALT)],
        NextSkins: "Cycle next onion skins" => [Period(ALT)],
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct Keymap {
    // None explicitly clears the binding. Missing entries use all factory aliases.
    overrides: BTreeMap<Action, Option<Chord>>,
}

impl Keymap {
    fn bindings(&self, action: Action) -> &[Chord] {
        match self.overrides.get(&action) {
            Some(custom) => custom.as_slice(),
            None => action.defaults(),
        }
    }

    pub(crate) fn is_default(&self, action: Action) -> bool {
        !self.overrides.contains_key(&action)
    }

    fn owner(&self, chord: Chord) -> Option<Action> {
        Action::ALL
            .iter()
            .copied()
            .find(|&action| self.bindings(action).contains(&chord))
    }

    pub(crate) fn resolve(&self, key: Key, mods: Modifiers) -> Option<Action> {
        self.owner(Chord::from_event(key, mods))
    }

    pub(crate) fn conflicting_action(
        &self,
        action: Action,
        key: Key,
        mods: Modifiers,
    ) -> Option<Action> {
        let chord = Chord::from_event(key, mods);
        chord.check_reserved(action).ok()?;
        self.owner(chord).filter(|&owner| owner != action)
    }

    pub(crate) fn label(&self, ctx: &Context, action: Action) -> String {
        self.bindings(action)
            .first()
            .map(|chord| ctx.format_shortcut(&chord.shortcut()))
            .unwrap_or_default()
    }

    pub(crate) fn labels(&self, ctx: &Context, action: Action) -> String {
        // Extra Shift on punctuation accommodates keyboard layouts. It does
        // not represent another shortcut the user needs to memorize.
        if matches!(
            action,
            Action::Difference | Action::Division | Action::CutPath
        ) {
            return self.label(ctx, action);
        }
        self.bindings(action)
            .iter()
            .map(|chord| ctx.format_shortcut(&chord.shortcut()))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    fn check_available(&self, action: Action, chord: Chord) -> Result<(), String> {
        chord.check_reserved(action)?;
        if let Some(owner) = self.owner(chord).filter(|&owner| owner != action) {
            return Err(format!("This shortcut is assigned to {}.", owner.name()));
        }
        Ok(())
    }

    pub(crate) fn set(&mut self, action: Action, key: Key, mods: Modifiers) -> Result<(), String> {
        let chord = Chord::from_event(key, mods);
        self.check_available(action, chord)?;
        if action.defaults() == [chord] {
            self.overrides.remove(&action);
        } else {
            self.overrides.insert(action, Some(chord));
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self, action: Action) {
        self.overrides.insert(action, None);
    }

    pub(crate) fn reset(&mut self, action: Action) -> Result<(), String> {
        for &chord in action.defaults() {
            self.check_available(action, chord)?;
        }
        self.overrides.remove(&action);
        Ok(())
    }

    pub(crate) fn merge_changes(&mut self, original: &Self, edited: &Self) -> Result<(), String> {
        let mut merged = self.clone();
        for &action in Action::ALL {
            let before = original.overrides.get(&action);
            let after = edited.overrides.get(&action);
            if before == after {
                continue;
            }
            let current = self.overrides.get(&action);
            if current != before && current != after {
                return Err(format!(
                    "Another window changed {}. Reopen Keyboard Shortcuts to review it.",
                    action.name()
                ));
            }
            if let Some(&binding) = after {
                merged.overrides.insert(action, binding);
            } else {
                merged.overrides.remove(&action);
            }
        }
        for &action in Action::ALL {
            for &chord in merged.bindings(action) {
                merged.check_available(action, chord)?;
            }
        }
        *self = merged;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A newer release can introduce an action or key this build doesn't
        // recognize. Keep unrelated preferences, including intentional clears.
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut map = Self::default();
        if let Some(entries) = value.as_object() {
            for (name, value) in entries {
                let Ok(action) = Action::deserialize(serde_json::Value::String(name.clone()))
                else {
                    continue;
                };
                let Ok(binding) = Option::<Chord>::deserialize(value) else {
                    continue;
                };
                if binding.is_none_or(|chord| chord.check_reserved(action).is_ok()) {
                    map.overrides.insert(action, binding);
                }
            }
        }
        // These defaults were added after users could already bind their keys.
        for action in [Action::SimplifyPath, Action::Grayscale] {
            if map.is_default(action)
                && map
                    .overrides
                    .values()
                    .flatten()
                    .any(|chord| action.defaults().contains(chord))
            {
                map.clear(action);
            }
        }
        // Resolve invalid hand-edited conflicts deterministically. A restored
        // default can reclaim another binding on the next pass. Valid swaps
        // survive because we check the complete saved set before restoring.
        loop {
            let before = map.overrides.len();
            let mut taken = std::collections::BTreeSet::new();
            for &action in Action::ALL {
                if map.is_default(action) {
                    taken.extend(action.defaults().iter().copied());
                }
            }
            map.overrides
                .retain(|_, binding| binding.is_none_or(|chord| taken.insert(chord)));
            if map.overrides.len() == before {
                break;
            }
        }
        Ok(map)
    }
}

pub(crate) fn active(ctx: &Context) -> Arc<Keymap> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Keymap>>(Id::new("phyro.keymap"))
            .clone()
    })
}

pub(crate) fn install(ctx: &Context, map: Arc<Keymap>) {
    ctx.data_mut(|data| data.insert_temp(Id::new("phyro.keymap"), map));
}

pub(crate) fn focused_text_field(ctx: &Context) -> Option<Id> {
    ctx.memory(|memory| memory.focused())
        .filter(|id| eframe::egui::TextEdit::load_state(ctx, *id).is_some())
}

pub(crate) fn widget_owns_shortcuts(ctx: &Context) -> bool {
    // egui reports wants_keyboard_input for every tab stop. A focused button
    // still needs activation and navigation keys, while drawing shortcuts can
    // keep working. Focused DragValues use a TextEdit with the same widget ID.
    if !ctx.wants_keyboard_input() {
        return false;
    }
    focused_text_field(ctx).is_some()
        || ctx.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        key: Key::Enter
                            | Key::Space
                            | Key::ArrowLeft
                            | Key::ArrowRight
                            | Key::ArrowUp
                            | Key::ArrowDown,
                        pressed: true,
                        ..
                    }
                ) || matches!(event, Event::Key { key: Key::Tab, modifiers, pressed: true, .. }
                    if !modifiers.ctrl && !modifiers.command)
            })
        })
}

pub(crate) fn label(ctx: &Context, action: Action) -> String {
    active(ctx).label(ctx, action)
}

pub(crate) fn hint(ctx: &Context, caption: &str, action: Action) -> String {
    let shortcut = label(ctx, action);
    if shortcut.is_empty() {
        caption.to_owned()
    } else {
        format!("{caption} ({shortcut})")
    }
}

pub(crate) fn pressed(ctx: &Context, action: Action) -> bool {
    let map = active(ctx);
    ctx.input(|input| input.focused && input.events.iter().any(|event| {
        matches!(event, Event::Key { key, modifiers, pressed: true, repeat: false, .. } if map.bindings(action).contains(&Chord::from_event(*key, *modifiers)))
    }))
}
