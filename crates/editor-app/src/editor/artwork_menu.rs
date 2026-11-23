use super::{arrange, edit_commands::Command, Editor, Tool};
use crate::commands::{editable_selection, GroupAction};
use crate::inspector::appearance::{clip_actions, ClipAction};
use crate::{diagnostics::Action, keymap};
use eframe::egui;
use keymap::Action as Shortcut;
use std::collections::HashSet;

fn conversion_button(ui: &egui::Ui, action: Shortcut) -> egui::Button<'static> {
    egui::Button::new(action.name()).shortcut_text(keymap::label(ui.ctx(), action))
}

impl Editor {
    pub(super) fn object_menu(&mut self, ui: &mut egui::Ui) {
        let available = !self.timeline.is_playing()
            && !self.canvas.path_in_progress()
            && !self.canvas.rig.placing()
            && !self.canvas.text_active()
            && self.text_menu.field.is_none();
        ui.add_enabled_ui(available, |ui| {
            self.group_commands(ui);
            ui.separator();
            if let Some(horizontal) =
                crate::tool_keys::flip_menu(ui, self.scene(), &self.selection, available)
            {
                self.flip_selection(horizontal);
            }
            ui.menu_button("Stacking order", |ui| {
                let keys = keymap::active(ui.ctx());
                for (action, order, label) in crate::tool_keys::STACKING {
                    let button =
                        egui::Button::new(label).shortcut_text(keys.label(ui.ctx(), action));
                    if ui.add_enabled(!self.selection.is_empty(), button).clicked() {
                        self.stack_selection(order);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("Align and Distribute", |ui| {
                if let Some(action) =
                    crate::inspector::arrange_ui::menu(ui, self.scene(), &self.selection)
                {
                    ui.close_menu();
                    self.note(Action::Align);
                    self.finish_edit();
                    self.canvas.cancel_drag();
                    let frame = self.timeline.frame;
                    let doc = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
                    match arrange::apply(doc, &self.selection, frame, self.style.key_mode, action) {
                        Ok(true) => self.status.clear(),
                        Ok(false) => {
                            self.status = match action {
                                arrange::Action::Align(_, _) => "The selection is already aligned.",
                                _ => "The gaps are already equal.",
                            }
                            .into()
                        }
                        Err(error) => self.report_error("Could not arrange selection", error),
                    }
                    self.finish_edit();
                }
            });
            ui.separator();
            ui.menu_button("Clip and mask", |ui| self.clip_commands(ui));
            self.symbol_command(ui);
            if self.morph_action(ui, true) {
                ui.close_menu();
            }
        });
    }

    fn group_commands(&mut self, ui: &mut egui::Ui) {
        for action in [GroupAction::Group, GroupAction::Ungroup] {
            self.artwork_command(ui, Command::Grouping(action));
        }
    }

    fn artwork_command(&mut self, ui: &mut egui::Ui, command: Command) {
        let action = command.action();
        let button = egui::Button::new(action.name())
            .shortcut_text(keymap::active(ui.ctx()).labels(ui.ctx(), action));
        if ui
            .add_enabled(self.edit_enabled(ui.ctx(), command), button)
            .clicked()
        {
            ui.close_menu();
            self.run_edit_command(ui.ctx(), command);
        }
    }

    pub(super) fn conversion_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let action = [
            Shortcut::ConvertToPath,
            Shortcut::StrokeToPath,
            Shortcut::ConvertToSymbol,
        ]
        .into_iter()
        .find(|&action| keymap::pressed(ctx, action))
        .or_else(|| {
            // egui-winit emits Copy in place of a Ctrl+C key press, including
            // modified chords. Recover the conversion from the active modifiers.
            let keys = keymap::active(ctx);
            ctx.input(|input| {
                if !input.focused || !input.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
                    return None;
                }
                keys.resolve(egui::Key::C, input.modifiers)
                    .filter(|action| {
                        matches!(action, Shortcut::ConvertToPath | Shortcut::StrokeToPath)
                    })
            })
        });
        match action {
            Some(Shortcut::ConvertToPath) if self.can_convert_selected_paths() => {
                self.convert_selected_paths()
            }
            Some(Shortcut::StrokeToPath) if self.has_stroke_selection() => {
                self.convert_selected_strokes()
            }
            Some(Shortcut::ConvertToSymbol) if !self.selection.is_empty() => {
                self.convert_selected_symbol()
            }
            _ => {}
        }
        action.is_some()
    }

    pub(super) fn path_conversion_commands(&mut self, ui: &mut egui::Ui) {
        let enabled = !self.timeline.is_playing()
            && !self.canvas.path_in_progress()
            && !self.canvas.text_active()
            && !self.canvas.rig.placing();
        if self.convert_to_path_button(ui, enabled) {
            ui.close_menu();
        }
        if ui
            .add_enabled(
                enabled && self.has_stroke_selection(),
                conversion_button(ui, Shortcut::StrokeToPath),
            )
            .on_hover_text("Expand visible strokes into editable filled outlines")
            .clicked()
        {
            ui.close_menu();
            self.convert_selected_strokes();
        }
    }

    fn has_stroke_selection(&self) -> bool {
        let doc = self.scene();
        let frame = self.timeline.frame;
        !self.selection.is_empty()
            && doc.objects.iter().any(|o| {
                self.selection.contains(&o.id)
                    && document_core::arrange::is_object_editable(doc, o)
                    && o.visible_at(frame)
                    && document_core::path_edit::has_convertible_stroke(o, frame)
            })
    }

    fn convert_selected_strokes(&mut self) {
        self.finish_edit();
        self.canvas.cancel_drag();
        let frame = self.timeline.frame;
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
        match document_core::path_edit::stroke_to_paths(doc, &self.selection, frame) {
            Ok(ids) if !ids.is_empty() => {
                self.select_artwork(ids);
                self.set_tool(if self.selection.len() == 1 {
                    Tool::Node
                } else {
                    Tool::Select
                });
                self.sync_paint();
                self.note(Action::EditPath);
                self.finish_edit();
            }
            Ok(_) => {}
            Err(error) => self.report_error("Could not convert stroke to path", error),
        }
    }

    fn symbol_command(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                !self.selection.is_empty(),
                conversion_button(ui, Shortcut::ConvertToSymbol),
            )
            .clicked()
        {
            ui.close_menu();
            self.convert_selected_symbol();
        }
    }

    fn can_convert_selected_paths(&self) -> bool {
        let doc = self.scene();
        let selected: HashSet<_> = self.selection.iter().copied().collect();
        !selected.is_empty()
            && doc.objects.iter().any(|object| {
                selected.contains(&object.id)
                    && crate::path_editor::can_convert_to_path(object)
                    && document_core::arrange::is_object_editable(doc, object)
            })
    }

    pub(super) fn convert_to_path_button(&mut self, ui: &mut egui::Ui, enabled: bool) -> bool {
        if !ui
            .add_enabled(
                enabled && self.can_convert_selected_paths(),
                conversion_button(ui, Shortcut::ConvertToPath),
            )
            .on_hover_text(
                "Make rectangle, ellipse, or text outlines editable with the Nodes tool.",
            )
            .on_disabled_hover_text("Select editable rectangles, ellipses, or text.")
            .clicked()
        {
            return false;
        }
        self.convert_selected_paths();
        true
    }

    fn convert_selected_paths(&mut self) {
        self.finish_edit();
        self.note(Action::EditPath);
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
        let selected: HashSet<_> = self.selection.iter().copied().collect();
        let layers: HashSet<_> = doc
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked)
            .map(|l| l.id)
            .collect();
        for object in &mut doc.objects {
            if !object.locked && selected.contains(&object.id) && layers.contains(&object.layer) {
                crate::path_editor::convert_to_path(object);
            }
        }
        if selected.len() == 1 {
            self.set_tool(Tool::Node);
            self.sync_paint();
        } else {
            self.set_tool(Tool::Select);
        }
        self.finish_edit();
    }

    fn clip_commands(&mut self, ui: &mut egui::Ui) {
        if let Some(action) = clip_actions(ui, self.scene(), &self.selection, true) {
            ui.close_menu();
            self.apply_clip_action(action);
        }
    }

    pub(super) fn artwork_context_menu(&mut self) {
        let Some((surface, at)) = self.canvas.take_context_surface() else {
            return;
        };
        if surface.secondary_clicked() {
            if let Some(id) = at.and_then(|p| self.scene().hit_test(p, self.timeline.frame)) {
                if !self.selection.contains(&id) {
                    self.selection = editable_selection(self.scene(), &[id]);
                }
            }
        }
        surface.context_menu(|ui| {
            for command in [Command::Copy, Command::PasteStyle, Command::Duplicate] {
                self.artwork_command(ui, command);
            }
            ui.separator();
            self.group_commands(ui);
            self.path_conversion_commands(ui);
            self.symbol_command(ui);
            if self.morph_action(ui, true) {
                ui.close_menu();
            }
            ui.separator();
            self.clip_commands(ui);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.close_menu();
            }
        });
        self.artwork_menu_open = surface.context_menu_opened();
    }

    pub(super) fn apply_clip_action(&mut self, action: ClipAction) {
        self.finish_edit();
        self.canvas.cancel_drag();
        let frame = self.timeline.frame;
        let doc = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
        match action.apply(doc, &mut self.selection, frame) {
            Ok(()) => {
                self.timeline.leave_grid_selection();
                self.sync_paint();
                self.status.clear();
                self.finish_edit();
            }
            Err(error) => self.report_error("Could not change clipping", error),
        }
    }

    fn convert_selected_symbol(&mut self) {
        self.finish_edit();
        let names: std::collections::HashSet<_> = self
            .project
            .symbols
            .values()
            .map(|s| s.name.to_lowercase())
            .collect();
        let object_name = self
            .single()
            .and_then(|id| self.scene().objects.iter().find(|o| o.id == id))
            .map(|o| o.name.trim())
            .filter(|s| {
                !s.is_empty()
                    && s.chars().count() <= 80
                    && !s.chars().any(char::is_control)
                    && !names.contains(&s.to_lowercase())
            });
        let name = object_name.map(str::to_owned).unwrap_or_else(|| {
            (1..)
                .map(|i| format!("Symbol {i}"))
                .find(|s| !names.contains(&s.to_lowercase()))
                .unwrap()
        });
        match self.library.convert_selection(
            &mut self.project,
            self.timeline.clip,
            self.timeline.frame,
            &mut self.selection,
            &name,
        ) {
            Ok(message) => {
                self.timeline.leave_grid_selection();
                self.status = message;
                self.sync_paint();
                self.finish_edit();
            }
            Err(error) => self.report_error("Could not create symbol", error),
        }
    }
}
