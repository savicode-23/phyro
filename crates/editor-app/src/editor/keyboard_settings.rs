use super::Editor;
use crate::keymap::{Action, Keymap};
use eframe::egui::{self, Event, Key, Modifiers};

#[derive(Default)]
pub(super) struct KeyboardSettings {
    pub(super) current: Keymap,
    dialog: Option<Dialog>,
}

struct Dialog {
    draft: Keymap,
    query: String,
    selected: Option<Action>,
    listening: bool,
    clipboard_modifiers: Option<Modifiers>,
    conflict: Option<Action>,
    message: String,
    focus_search: bool,
}

impl KeyboardSettings {
    pub(super) fn open(&mut self) {
        self.dialog = Some(Dialog {
            draft: self.current.clone(),
            query: String::new(),
            selected: None,
            listening: false,
            clipboard_modifiers: None,
            conflict: None,
            message: String::new(),
            focus_search: true,
        });
    }

    pub(super) fn is_open(&self) -> bool {
        self.dialog.is_some()
    }
}

impl Dialog {
    fn select(&mut self, action: Action, listen: bool) {
        self.selected = Some(action);
        self.listening = listen;
        self.clipboard_modifiers = None;
        self.conflict = None;
        self.message.clear();
    }

    fn capture(&mut self, ctx: &egui::Context) {
        let Some(action) = self.selected.filter(|_| self.listening) else {
            return;
        };
        let mut pressed = None;
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                match event {
                    Event::Key {
                        key,
                        pressed: down,
                        repeat: false,
                        modifiers,
                        ..
                    } => {
                        // egui-winit translates native clipboard presses into semantic
                        // events. The key release still distinguishes X/C/V and Insert/Delete.
                        let clipboard_release = matches!(
                            key,
                            Key::X
                                | Key::C
                                | Key::V
                                | Key::Insert
                                | Key::Delete
                                | Key::Cut
                                | Key::Copy
                                | Key::Paste
                        ) && (self.clipboard_modifiers.is_some()
                            || modifiers.command
                            || modifiers.ctrl
                            || modifiers.shift);
                        if *down || clipboard_release {
                            let mods = if !down {
                                self.clipboard_modifiers
                                    .filter(|pressed| !pressed.is_none())
                                    .unwrap_or(*modifiers)
                            } else {
                                *modifiers
                            };
                            pressed.get_or_insert((*key, mods));
                        }
                        false
                    }
                    Event::Copy | Event::Cut | Event::Paste(_) => {
                        self.clipboard_modifiers = Some(input.modifiers);
                        false
                    }
                    Event::Key { .. } | Event::Text(_) | Event::Ime(_) => false,
                    _ => true,
                }
            });
        });
        let Some((key, mods)) = pressed else { return };
        self.select(action, false);
        if key == Key::Escape {
            return;
        }
        if let Err(reason) = self.draft.set(action, key, mods) {
            self.conflict = self.draft.conflicting_action(action, key, mods);
            self.message = reason;
        }
    }

    fn rows(&mut self, ui: &mut egui::Ui, height: f32) {
        let query = self.query.trim().to_lowercase();
        let mut category = None;
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for &action in Action::ALL {
                    let keys = self.draft.labels(ui.ctx(), action);
                    if !query.is_empty()
                        && ![action.name(), action.category(), keys.as_str()]
                            .iter()
                            .any(|text| text.to_lowercase().contains(&query))
                    {
                        continue;
                    }
                    if category != Some(action.category()) {
                        category = Some(action.category());
                        ui.strong(action.category());
                    }
                    ui.push_id(action.name(), |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.selected == Some(action), action.name())
                                .clicked()
                            {
                                self.select(action, false);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let label = if keys.is_empty() { "Unassigned" } else { &keys };
                                    if ui
                                        .button(label)
                                        .on_hover_text("Record a shortcut")
                                        .clicked()
                                    {
                                        self.select(action, true);
                                    }
                                },
                            );
                        });
                    });
                }
                if category.is_none() {
                    ui.weak("No matching commands");
                }
            });
    }

    fn edit_selection(&mut self, ui: &mut egui::Ui) {
        let Some(action) = self.selected else {
            ui.weak("Select a command or click its shortcut to record a key.");
            return;
        };
        if self.listening {
            ui.label(format!(
                "Press a shortcut for {}. Esc cancels.",
                action.name()
            ));
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.strong(action.name());
            if ui.button("Change shortcut").clicked() {
                self.select(action, true);
            }
            if ui
                .add_enabled(
                    !self.draft.label(ui.ctx(), action).is_empty(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.draft.clear(action);
                self.select(action, false);
            }
            if ui
                .add_enabled(!self.draft.is_default(action), egui::Button::new("Reset"))
                .clicked()
            {
                self.select(action, false);
                if let Err(reason) = self.draft.reset(action) {
                    self.message = reason;
                }
            }
        });
    }
}

impl Editor {
    pub(super) fn show_keyboard_settings(&mut self, ctx: &egui::Context) {
        if self
            .keyboard_settings
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.focus_search)
        {
            self.refresh_keyboard_shortcuts(ctx);
            if let Some(dialog) = &mut self.keyboard_settings.dialog {
                dialog.draft.clone_from(&self.keyboard_settings.current);
            }
        }
        let Some(dialog) = &mut self.keyboard_settings.dialog else {
            return;
        };
        dialog.capture(ctx);
        let mut apply = false;
        let mut cancel = false;
        let modal = egui::Modal::new("keyboard-shortcuts".into()).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.).clamp(240., 550.));
            ui.heading("Keyboard shortcuts");
            let search = ui.add(
                egui::TextEdit::singleline(&mut dialog.query)
                    .hint_text("Search commands or shortcuts")
                    .desired_width(f32::INFINITY),
            );
            if dialog.focus_search {
                search.request_focus();
                dialog.focus_search = false;
            }
            if search.changed() || search.clicked() {
                dialog.selected = None;
                dialog.listening = false;
                dialog.conflict = None;
                dialog.message.clear();
            }
            let height = (ctx.screen_rect().height() - 242.).clamp(65., 340.);
            ui.separator();
            dialog.rows(ui, height);
            ui.separator();
            // Keep the commands and footer at fixed positions as recording and
            // conflict messages appear. The modal is centered in the window.
            let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            ui.scope(|ui| {
                ui.set_min_height(row_height * 2.);
                dialog.edit_selection(ui);
                if !dialog.message.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(ui.visuals().warn_fg_color, &dialog.message);
                        if let Some(owner) = dialog.conflict {
                            if ui.button("Edit conflicting command").clicked() {
                                dialog.query = owner.name().to_owned();
                                dialog.select(owner, false);
                            }
                        }
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        dialog.draft != Keymap::default(),
                        egui::Button::new("Reset all"),
                    )
                    .clicked()
                {
                    dialog.draft = Keymap::default();
                    dialog.listening = false;
                    dialog.conflict = None;
                    dialog.message.clear();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    apply = ui
                        .add_enabled(
                            !dialog.listening && dialog.conflict.is_none(),
                            egui::Button::new("Apply"),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        });
        if apply {
            let changed = dialog.draft.clone();
            match self.apply_keyboard_shortcuts(ctx, changed) {
                Ok(()) => self.keyboard_settings.dialog = None,
                Err(error) => {
                    if let Some(dialog) = &mut self.keyboard_settings.dialog {
                        dialog.message = format!("Could not save shortcuts. {error}");
                    }
                }
            }
        } else if cancel || modal.should_close() {
            self.keyboard_settings.dialog = None;
        }
        if !self.keyboard_settings.is_open() {
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
            ctx.request_repaint();
        }
    }
}
