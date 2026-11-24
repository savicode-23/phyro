use super::{Editor, Key, Tool};
use crate::canvas::GroupEdit;
use crate::theme::{MINT, PETROL};
use document_core::arrange::{self, Group, SelectionScope};
use eframe::egui;

impl Editor {
    pub(super) fn selection_scope(&self) -> SelectionScope {
        self.canvas.selection_scope(&self.style, &self.selection)
    }

    pub(super) fn can_enter_group(&self, group: Group) -> bool {
        self.style.group_edit.is_none()
            && arrange::is_editable(self.scene(), group.layer)
            && self
                .scene()
                .objects
                .iter()
                .any(|o| group.contains(o) && !o.locked)
    }

    pub(super) fn enter_group(&mut self, group: Group) {
        if !self.can_enter_group(group) {
            return;
        }
        self.finish_edit();
        self.set_tool(Tool::Select);
        self.select_artwork(Vec::new());
        let clip = &self.project.clips[self.timeline.clip];
        self.style.group_edit = Some(GroupEdit {
            group,
            clip_session: clip.session_id(),
            exposure_start: clip.layer_exposure(self.timeline.frame, group.layer).0,
        });
        self.style.layer = group.layer;
    }

    pub(super) fn leave_group(&mut self) {
        self.style.group_edit = None;
        self.canvas.cancel_drag();
        self.select_artwork(Vec::new());
        self.sync_paint();
    }

    pub(super) fn refresh_group_edit(&mut self) {
        let Some(edit) = self.style.group_edit else {
            return;
        };
        let clip = &self.project.clips[self.timeline.clip];
        if self.style.layer != edit.group.layer
            || edit.clip_session != clip.session_id()
            || edit.exposure_start != clip.layer_exposure(self.timeline.frame, edit.group.layer).0
            || !arrange::is_editable(self.scene(), edit.group.layer)
            || edit.group.member(self.scene()).is_none()
        {
            self.leave_group();
        }
    }

    pub(super) fn group_edit_bar(&mut self, ui: &mut egui::Ui) {
        let Some(edit) = self.style.group_edit else {
            return;
        };
        let Some((layer, member)) = self
            .scene()
            .layers
            .iter()
            .find(|l| l.id == edit.group.layer)
            .zip(edit.group.member(self.scene()))
        else {
            return;
        };
        let name = member
            .group_name
            .as_deref()
            .map_or_else(|| format!("Group {}", edit.group.id), str::to_owned);
        let can_leave = !self.canvas.has_unfinished_edit();
        let leave = egui::Frame::new()
            .fill(PETROL)
            .stroke(egui::Stroke::new(1_f32, MINT.gamma_multiply(0.55)))
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let exit = ui
                            .add_enabled(can_leave, egui::Button::new("Exit group"))
                            .on_hover_text("Return to the layer (Esc)")
                            .clicked();
                        let breadcrumb = ui
                            .with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new("Editing group").color(MINT).strong());
                                ui.separator();
                                let parent = ui
                                    .scope(|ui| {
                                        ui.set_max_width((ui.available_width() * 0.45).min(160.));
                                        ui.add_enabled(
                                            can_leave,
                                            egui::Button::new(&layer.name).frame(false).truncate(),
                                        )
                                        .on_hover_text(format!("Return to {}", layer.name))
                                        .clicked()
                                    })
                                    .inner;
                                ui.label("›");
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&name).strong())
                                        .truncate(),
                                )
                                .on_hover_text(&name);
                                parent
                            })
                            .inner;
                        exit || breadcrumb
                    },
                )
                .inner
            })
            .inner;
        if leave {
            self.leave_group();
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn group_exit_shortcut(&mut self, ctx: &egui::Context) -> bool {
        if self.style.group_edit.is_some()
            && !self.canvas.has_unfinished_edit()
            && !self.artwork_menu_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            self.leave_group();
            return true;
        }
        false
    }
}
