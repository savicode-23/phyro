mod draft;
mod folders;
use document_core::{AnimationProject, SymbolDefinition};
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) enum AssetSource {
    File { path: PathBuf, clip_id: Option<u64> },
    ProjectClip { id: u64, session: u64 },
}

pub(crate) struct AssetDrop {
    pub source: AssetSource,
    pub label: String,
    pub thumbnail: Option<egui::TextureHandle>,
}

#[derive(Default)]
pub struct Library {
    selected: Option<u64>,
    name: String,
    notice: Option<String>,
    pending_removal: Option<u64>,
    draft: Option<draft::Draft>,
    folders: folders::FolderPanel,
    show_folders: bool,
}

impl Library {
    pub(crate) fn reset_project(&mut self) {
        let folders = std::mem::take(&mut self.folders);
        *self = Self {
            folders,
            show_folders: self.show_folders,
            ..Self::default()
        };
    }

    pub(crate) fn folders_active(&self) -> bool {
        self.show_folders
    }

    pub(crate) fn set_folders_active(&mut self, active: bool) {
        self.show_folders = active;
    }

    pub(crate) fn directory(&self) -> Option<&Path> {
        self.folders.directory()
    }

    pub(crate) fn set_directory(&mut self, path: PathBuf) {
        self.folders.navigate(path);
    }

    pub(crate) fn begin_frame(&mut self, busy: bool) {
        self.folders.begin_frame(busy);
    }

    pub(crate) fn end_frame(&mut self, ctx: &egui::Context, resume_after_input: bool) {
        self.folders.end_frame(ctx, resume_after_input);
    }

    pub(crate) fn take_open_asset(&mut self) -> Option<PathBuf> {
        self.folders.open_source.take()
    }

    #[cfg(test)]
    pub(crate) fn set_cache_directory(&mut self, path: PathBuf) {
        self.folders.cache_directory = Some(path);
    }

    #[cfg(test)]
    pub(crate) fn indexed_assets(&self) -> usize {
        self.folders.entry_count()
    }

    #[cfg(test)]
    pub(crate) fn resident_previews(&self) -> usize {
        self.folders.resident_count()
    }

    #[cfg(test)]
    pub(crate) fn playing_preview_frame(&self) -> Option<u32> {
        self.folders.playing_frame()
    }

    pub fn is_editing(&self) -> bool {
        self.draft.is_some() || self.pending_removal.is_some()
    }
    pub fn edit_copy_button(
        &mut self,
        ui: &mut egui::Ui,
        project: &AnimationProject,
        clip: usize,
        frame: u32,
        selection: &[u64],
    ) {
        if let Some(copy) = draft::Draft::edit_copy_button(ui, project, clip, frame, selection) {
            self.draft = Some(copy);
        }
    }
    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        p: &mut AnimationProject,
        clip: usize,
        frame: u32,
        layer: u64,
        selection: &mut Vec<u64>,
    ) -> Option<String> {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_folders, false, "Symbols");
            ui.selectable_value(&mut self.show_folders, true, "Folders");
        });
        ui.separator();
        if self.show_folders {
            self.folders.show(ui);
            return None;
        }
        let mut status = None;
        if let Some(result) = self.symbols(ui, p, clip, frame, layer, selection) {
            match result {
                Ok(message) => {
                    self.notice = None;
                    status = Some(message);
                }
                Err(error) => self.notice = Some(error),
            }
        }
        if let Some(message) = &self.notice {
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
        status
    }
    pub fn dialogs(
        &mut self,
        ctx: &egui::Context,
        p: &mut AnimationProject,
        checkerboard: bool,
        pencil_smoothing: &mut u8,
        pencil_pressure: &mut bool,
    ) -> Option<Result<String, String>> {
        if let Some(id) = self.pending_removal {
            let Some(definition) = p.symbols.get(&id) else {
                self.pending_removal = None;
                return None;
            };
            let name = &definition.name;
            let mut remove = false;
            let modal = egui::Modal::new(egui::Id::new("remove-symbol")).show(ctx, |ui| {
                ui.set_max_width(340.);
                ui.heading(format!("Remove {name} from the library?"));
                ui.label("Each placement keeps its artwork and animation and can be edited independently.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.pending_removal = None;
                    }
                    remove = ui.button("Remove and keep artwork").clicked();
                });
            });
            if remove {
                self.pending_removal = None;
                let name = name.clone();
                return Some(match p.remove_symbol_keeping_artwork(id) {
                    Ok(()) => {
                        self.selected = None;
                        Ok(format!("Removed {name} and kept its placed artwork."))
                    }
                    Err(error) => Err(error),
                });
            }
            if modal.should_close() {
                self.pending_removal = None;
            }
            return None;
        }
        let result =
            self.draft
                .as_mut()?
                .show(ctx, p, checkerboard, pencil_smoothing, pencil_pressure)?;
        self.draft = None;
        result.map(Ok)
    }
    pub(crate) fn convert_selection(
        &mut self,
        project: &mut AnimationProject,
        clip: usize,
        frame: u32,
        selection: &mut Vec<u64>,
        name: &str,
    ) -> Result<String, String> {
        let (id, object) = project.convert_to_symbol(clip, frame, selection, name)?;
        self.selected = Some(id);
        self.name = name.to_owned();
        self.notice = None;
        *selection = vec![object];
        Ok(format!("Created {name} and linked the selected artwork."))
    }

    fn symbols(
        &mut self,
        ui: &mut egui::Ui,
        p: &mut AnimationProject,
        clip: usize,
        frame: u32,
        layer: u64,
        selection: &mut Vec<u64>,
    ) -> Option<Result<String, String>> {
        let mut result = None;
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .desired_width(ui.available_width())
                    .hint_text("Symbol name")
                    .char_limit(80),
            );
        });
        let name = self.name.trim().to_owned();
        ui.horizontal_wrapped(|ui| {
            let enabled = !name.is_empty() && !selection.is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new("Convert selection"))
                .on_hover_text("Replace adjacent artwork on one layer with a linked instance.")
                .clicked()
            {
                result = Some(self.convert_selection(p, clip, frame, selection, &name));
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Save a copy"))
                .on_hover_text("Capture the selection at this frame and keep the original objects.")
                .clicked()
            {
                result = Some(
                    SymbolDefinition::capture(
                        &name,
                        p.clips[clip].drawing_at(frame),
                        frame,
                        selection,
                    )
                    .and_then(|(def, _)| p.add_symbol(def))
                    .map(|id| {
                        self.selected = Some(id);
                        format!("Added {name} to the library.")
                    }),
                );
            }
        });
        ui.separator();
        if p.symbols.is_empty() {
            self.pending_removal = None;
            ui.label("Select artwork on the canvas and give your first symbol a name.");
            return result;
        }
        egui::ScrollArea::vertical()
            .id_salt("definitions")
            .max_height(140.)
            .show(ui, |ui| {
                for (&id, def) in &p.symbols {
                    if ui
                        .selectable_label(self.selected == Some(id), &def.name)
                        .clicked()
                    {
                        self.selected = Some(id);
                        self.name = def.name.clone();
                        self.notice = None;
                    }
                }
            });
        let Some(id) = self.selected.filter(|id| p.symbols.contains_key(id)) else {
            self.pending_removal = None;
            return result;
        };
        let def = &p.symbols[&id];
        let label = def.name.clone();
        let (w, h) = (def.artwork.width, def.artwork.height);
        ui.label(format!("{w} × {h} px"));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Place instance").clicked() {
                let doc = p.clips[clip].drawing_at(frame);
                let origin = [
                    (doc.width as f32 - w as f32) * 0.5,
                    (doc.height as f32 - h as f32) * 0.5,
                ];
                result = Some(
                    p.place_symbol(id, clip, frame, layer, origin)
                        .map(|object| {
                            *selection = vec![object];
                            format!("Placed {label} at the center of the artboard.")
                        }),
                );
            }
            if ui.button("Edit artwork…").clicked() {
                self.draft = Some(draft::Draft::symbol(id, p));
            }
            ui.menu_button("Manage", |ui| {
                if ui
                    .add_enabled(
                        !name.is_empty() && name != label,
                        egui::Button::new("Rename using name field"),
                    )
                    .clicked()
                {
                    result = Some(
                        p.rename_symbol(id, &name)
                            .map(|()| format!("Renamed {label} to {name}.")),
                    );
                    ui.close_menu();
                }
            });
            if ui
                .button("Remove symbol…")
                .on_hover_text("Remove the definition and keep its artwork in the project.")
                .clicked()
            {
                // Counting references can visit every drawing. Do it on request.
                let uses = p.symbol_usage(id);
                if uses == 0 {
                    result = Some(p.remove_symbol(id).map(|()| {
                        self.selected = None;
                        format!("Removed {label} from the library.")
                    }));
                } else {
                    self.pending_removal = Some(id);
                }
            }
        });
        result
    }
}
