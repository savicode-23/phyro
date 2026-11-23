use std::path::{Path, PathBuf};

use document_core::AnimationProject;
use eframe::egui;

use crate::recovery::{self, Claim};

struct OfferedDraft {
    modified: String,
    claim: Claim,
}

impl OfferedDraft {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        self.claim
            .source()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "Untitled".into())
    }
}

#[derive(Default)]
pub(super) struct RecoveryDialog {
    open: bool,
    root: Option<PathBuf>,
    drafts: Vec<OfferedDraft>,
    selected: usize,
    confirm_discard: bool,
    scan_failed: bool,
    error: Option<String>,
}

impl RecoveryDialog {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn scan(&mut self, root: Option<&Path>, automatic: bool) {
        *self = Self::default();
        self.root = root.map(Path::to_owned);
        let Some(root) = root else {
            self.open = !automatic;
            self.error = Some("Recovery storage is unavailable on this device.".into());
            return;
        };
        match recovery::discover(root) {
            Ok(drafts) => {
                for draft in drafts {
                    match draft.claim() {
                        Ok(claim) => self.drafts.push(OfferedDraft {
                            modified: crate::diagnostics::utc(draft.modified),
                            claim,
                        }),
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::NotFound | std::io::ErrorKind::WouldBlock
                            ) => {}
                        Err(error) => {
                            self.error = Some(format!("Could not access a recovery copy. {error}"));
                            self.scan_failed = true;
                        }
                    }
                }
            }
            Err(error) => {
                self.error = Some(format!("Could not list recovery copies. {error}"));
                self.scan_failed = true;
            }
        }
        self.open = !automatic || !self.drafts.is_empty();
    }

    pub(super) fn selected_source(&self) -> Option<&Path> {
        self.drafts.get(self.selected)?.claim.source()
    }

    /// Transfer ownership only after the editor accepts the loaded project.
    /// Other windows can offer the remaining drafts as soon as this one closes.
    pub(super) fn take_recovered(&mut self) -> Option<Claim> {
        if self.selected >= self.drafts.len() {
            return None;
        }
        let draft = self.drafts.remove(self.selected);
        *self = Self::default();
        Some(draft.claim)
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.label("Choose a draft to open as an unsaved copy.");
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("recoverable-drafts-list")
            .auto_shrink([false, true])
            .max_height((ui.ctx().screen_rect().height() - 240.).clamp(65., 300.))
            .show(ui, |ui| {
                for (index, draft) in self.drafts.iter().enumerate() {
                    ui.push_id(index, |ui| {
                        let row = ui.add(
                            egui::Button::new(draft.name())
                                .shortcut_text(&draft.modified)
                                .selected(index == self.selected)
                                .min_size(egui::vec2(
                                    ui.available_width(),
                                    ui.spacing().interact_size.y,
                                ))
                                .truncate(),
                        );
                        if row.clicked() {
                            self.selected = index;
                            if !self.scan_failed {
                                self.error = None;
                            }
                        }
                        if let Some(source) = draft.claim.source() {
                            row.on_hover_text(source.to_string_lossy());
                        }
                    });
                }
            });
        ui.separator();
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) -> Option<AnimationProject> {
        if !self.open {
            return None;
        }
        let mut load = false;
        let mut discard = false;
        let mut later = false;
        let mut retry = false;
        let modal = egui::Modal::new("recover-drafts".into()).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.).clamp(200., 520.));
            ui.heading("Recover drafts");
            if self.confirm_discard {
                let draft = &self.drafts[self.selected];
                ui.label("Permanently discard this recovery copy?");
                ui.add(egui::Label::new(draft.name().as_ref()).truncate());
                ui.horizontal(|ui| {
                    if ui.button("Keep draft").clicked() {
                        self.confirm_discard = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        discard = ui.button("Discard draft").clicked();
                    });
                });
            } else {
                if !self.drafts.is_empty() {
                    self.list(ui);
                } else if self.error.is_none() {
                    ui.label("No recoverable drafts are available.");
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.horizontal(|ui| {
                    if !self.drafts.is_empty() {
                        load = ui.button("Recover a copy").clicked();
                        if ui.button("Discard…").clicked() {
                            self.confirm_discard = true;
                        }
                    }
                    if self.scan_failed {
                        retry = ui.button("Retry").clicked();
                    }
                    later = if self.drafts.is_empty() {
                        ui.button("Close").clicked()
                    } else {
                        ui.button("Later")
                            .on_hover_text("Reopen these copies from File → Recover drafts.")
                            .clicked()
                    };
                });
            }
        });
        if modal.should_close() {
            if self.confirm_discard {
                self.confirm_discard = false;
            } else {
                later = true;
            }
        }
        if later {
            *self = Self::default();
            ctx.request_repaint();
        } else if retry {
            self.scan(self.root.clone().as_deref(), false);
        } else if discard {
            self.confirm_discard = false;
            match self.drafts[self.selected].claim.discard() {
                Ok(()) => {
                    self.drafts.remove(self.selected);
                    self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
                    self.error = None;
                    if self.drafts.is_empty() {
                        *self = Self::default();
                        ctx.request_repaint();
                    }
                }
                Err(error) => self.error = Some(format!("Could not discard this draft. {error}")),
            }
        } else if load {
            match self.drafts[self.selected].claim.load() {
                Ok(project) => return Some(project),
                Err(error) => self.error = Some(format!("Could not open this draft. {error}")),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
