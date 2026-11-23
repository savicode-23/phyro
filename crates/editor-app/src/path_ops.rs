use crate::keymap::{self, Action, Keymap};
use document_core::{BooleanOp, Document};
use eframe::egui::{self, Key, Modifiers, Ui};
#[cfg(windows)]
mod windows_layout;

fn enabled(op: BooleanOp, count: usize) -> bool {
    match op {
        BooleanOp::Division | BooleanOp::CutPath => count == 2,
        _ => count >= 2,
    }
}

fn operation(
    map: &Keymap,
    key: Key,
    physical: Option<Key>,
    modifiers: Modifiers,
) -> Option<BooleanOp> {
    let resolved = map.resolve(key, modifiers);
    // egui-winit falls back to the physical key for * and ^. On Windows
    // recover the symbol from the active keyboard layout, including dead ^.
    #[cfg(windows)]
    let resolved =
        if modifiers.command && physical == Some(key) && resolved.is_none_or(|a| map.is_default(a))
        {
            let translated = match (windows_layout::symbol(key, modifiers.shift), modifiers.alt) {
                (Some('*'), false) => Some(Action::Intersection),
                (Some('^'), false) => Some(Action::Exclusion),
                (Some('/'), true) => Some(Action::CutPath),
                _ => None,
            };
            translated.filter(|&a| map.is_default(a)).or(resolved)
        } else {
            resolved
        };
    #[cfg(not(windows))]
    let _ = physical;
    match resolved? {
        Action::Union => Some(BooleanOp::Union),
        Action::Difference => Some(BooleanOp::Difference),
        Action::Intersection => Some(BooleanOp::Intersection),
        Action::Exclusion => Some(BooleanOp::Exclusion),
        Action::Division => Some(BooleanOp::Division),
        Action::CutPath => Some(BooleanOp::CutPath),
        _ => None,
    }
}

#[derive(Default)]
pub struct Shortcuts {
    // A held key belongs to the edit until release, even if the new selection
    // has only one object. Otherwise egui's GUI zoom picks up auto-repeat.
    held: Vec<Key>,
}

impl Shortcuts {
    pub fn take(&mut self, ctx: &egui::Context, count: usize) -> Vec<BooleanOp> {
        let map = keymap::active(ctx);
        ctx.input_mut(|input| {
            let mut edits = Vec::new();
            if !input.focused {
                self.held.clear();
                return edits;
            }
            input.events.retain(|event| {
                if matches!(event, egui::Event::WindowFocused(false)) {
                    self.held.clear();
                    return true;
                }
                let egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } = event
                else {
                    return true;
                };
                let physical = physical_key.unwrap_or(*key);
                if let Some(index) = self.held.iter().position(|&k| k == physical) {
                    if !*pressed {
                        self.held.swap_remove(index);
                    }
                    return false;
                }
                if *pressed && !*repeat && count >= 2 {
                    if let Some(op) = operation(&map, *key, *physical_key, *modifiers)
                        .filter(|&op| enabled(op, count))
                    {
                        self.held.push(physical);
                        edits.push(op);
                        return false;
                    }
                }
                true
            });
            edits
        })
    }
}

pub fn choose(ui: &mut Ui, selection_count: usize, menu: bool) -> Option<BooleanOp> {
    let mut chosen = None;
    for (label, op, action, hint) in [
        (
            "Union",
            BooleanOp::Union,
            Action::Union,
            "Combine filled regions.",
        ),
        (
            "Difference",
            BooleanOp::Difference,
            Action::Difference,
            "Subtract the upper paths from the back path.",
        ),
        (
            "Intersection",
            BooleanOp::Intersection,
            Action::Intersection,
            "Keep regions inside every selected path.",
        ),
        (
            "Exclusion",
            BooleanOp::Exclusion,
            Action::Exclusion,
            "Keep regions inside an odd number of selected paths.",
        ),
        (
            "Division",
            BooleanOp::Division,
            Action::Division,
            "Split the back fill at the front path into separate objects.",
        ),
        (
            "Cut path",
            BooleanOp::CutPath,
            Action::CutPath,
            "Split the back outline at its intersections with the front path.",
        ),
    ] {
        let mut button = egui::Button::new(label);
        if menu {
            button = button.shortcut_text(keymap::label(ui.ctx(), action));
        }
        if ui
            .add_enabled(enabled(op, selection_count), button)
            .on_hover_text(hint)
            .clicked()
        {
            chosen = Some(op);
        }
    }
    chosen
}

pub fn show(ui: &mut Ui, doc: &mut Document, selected: &mut Vec<u64>, frame: u32) {
    ui.collapsing("Path operations", |ui| {
        let status = ui.make_persistent_id("boolean-status");
        if let Some(op) = ui
            .horizontal_wrapped(|ui| choose(ui, selected.len(), false))
            .inner
        {
            match doc.boolean(selected, frame, op) {
                Ok(ids) => {
                    *selected = ids;
                    ui.data_mut(|d| d.remove::<String>(status));
                }
                Err(error) => ui.data_mut(|d| d.insert_temp(status, error)),
            }
        }
        ui.weak("Uses the current frame and bakes transforms and effects into editable paths.");
        if let Some(error) = ui.data(|d| d.get_temp::<String>(status)) {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
    });
}
