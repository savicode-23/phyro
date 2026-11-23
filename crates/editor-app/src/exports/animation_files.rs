use super::{choose_folder, AnimationExport, Completed, Exports};
use document_core::{AnimationProject, Clip, ExportPreset};
use eframe::egui;
use project_io::{
    animation_clip_example_filename, animation_set_example_filename, animation_set_file_stem,
    export_animation_files, AnimationSetOutput, ExportReceipt,
};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn name(target: AnimationExport, project: &AnimationProject) -> Option<&str> {
    match target {
        AnimationExport::Set(id) => Some(&project.animation_set(id)?.name),
        AnimationExport::Clip(id) => Some(&project.clips[project.clip_index(id)?].name),
    }
}

#[derive(Clone, Copy)]
struct AnimationSettings {
    target: AnimationExport,
    format: AnimationSetOutput,
    preset: ExportPreset,
}

#[derive(Clone)]
pub(super) struct LastExport {
    sessions: Vec<u64>,
    settings: AnimationSettings,
    pub(super) receipt: ExportReceipt,
}

impl LastExport {
    pub(super) fn target(&self) -> AnimationExport {
        self.settings.target
    }

    fn matches(&self, project: &AnimationProject, active: &Clip) -> bool {
        match self.settings.target {
            AnimationExport::Clip(id) => {
                active.id == id && self.sessions.contains(&active.session_id())
            }
            // Persistent IDs can be reused on a new undo branch. Only recognize a
            // set if it still contains artwork from the original export session.
            AnimationExport::Set(id) => {
                active.set_id == id
                    && project
                        .clips
                        .iter()
                        .any(|c| c.set_id == id && self.sessions.contains(&c.session_id()))
            }
        }
    }
}

impl Exports {
    fn previous_export(&self, project: &AnimationProject, index: usize) -> Option<&LastExport> {
        let active = project.clips.get(index)?;
        self.previous
            .iter()
            .rev()
            .find(|entry| entry.matches(project, active))
    }

    pub fn can_reexport(&self, project: &AnimationProject, index: usize) -> bool {
        !self.busy() && self.previous_export(project, index).is_some()
    }

    pub fn reexport(
        &mut self,
        project: &AnimationProject,
        index: usize,
        ctx: &egui::Context,
    ) -> Option<String> {
        let entry = self.previous_export(project, index)?.clone();
        self.write_animation(
            project,
            entry.settings,
            entry.receipt.directory().to_owned(),
            Some(entry.receipt),
            None,
            ctx,
        )
    }

    pub(crate) fn open_animation_export(&mut self, target: AnimationExport) -> Option<String> {
        if self.busy() {
            return Some("An export is already running.".into());
        }
        self.still_export = None;
        self.selection_animation = None;
        self.preview.clear();
        self.destination_error = None;
        self.animation_export = Some(target);
        None
    }

    pub(super) fn animation_export_dialog(
        &mut self,
        ctx: &egui::Context,
        project: &mut AnimationProject,
        target: AnimationExport,
    ) -> Option<String> {
        let Some(title) = name(target, project).map(|name| format!("Export {name}")) else {
            self.preview.clear();
            self.animation_export = None;
            return None;
        };
        let before_preset = project.preset;
        let modal = egui::Modal::new("animation-file-export".into()).show(ctx, |ui| {
            let available = ctx.screen_rect();
            let width = (available.width() - 46.).clamp(185., 640.);
            ui.set_width(width);
            ui.add(egui::Label::new(egui::RichText::new(&title).heading()).truncate())
                .on_hover_text(&title);
            ui.horizontal_wrapped(|ui| {
                for (format, label) in [
                    (AnimationSetOutput::PngSequences, "PNG sequence per clip"),
                    (AnimationSetOutput::SeparateSheets, "Sprite sheet per clip"),
                    (AnimationSetOutput::SharedSheet, "Shared sprite sheet"),
                ] {
                    ui.radio_value(&mut self.animation_format, format, label);
                }
            });
            ui.separator();
            let sheet = self.animation_format != AnimationSetOutput::PngSequences;
            // Keep mouse targets fixed while the asynchronous fit check finishes.
            let desired_height = match (sheet, target) {
                (false, AnimationExport::Clip(_)) => 82.,
                (false, AnimationExport::Set(_)) => 220.,
                (true, _) => 300.,
            };
            let height = (available.height() - 155.).clamp(65., desired_height);
            let mut example = None;
            let mut valid = false;
            egui::ScrollArea::vertical()
                .id_salt("animation-export-content")
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if width >= 510. {
                        ui.columns(2, |cols| {
                            if let AnimationExport::Set(id) = target {
                                self.changed_animation_options |=
                                    clip_checklist(&mut cols[0], project, id);
                                self.animation_settings(&mut cols[1], project, target, sheet);
                                cols[0].add_space(4.);
                                (example, valid) = self.fit_status(&mut cols[0], project, target);
                            } else {
                                self.animation_settings(&mut cols[0], project, target, sheet);
                                (example, valid) = self.fit_status(&mut cols[1], project, target);
                            }
                        });
                    } else {
                        if let AnimationExport::Set(id) = target {
                            self.changed_animation_options |= clip_checklist(ui, project, id);
                            ui.separator();
                        }
                        self.animation_settings(ui, project, target, sheet);
                        (example, valid) = self.fit_status(ui, project, target);
                    }
                });
            ui.separator();
            if let Some(path) = example {
                ui.horizontal(|ui| {
                    ui.weak("Filename");
                    ui.add(egui::Label::new(egui::RichText::new(&path).monospace()).truncate())
                        .on_hover_text(&path);
                });
            } else {
                ui.label("Select at least one clip to export.");
            }
            if let Some(error) = &self.destination_error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                let export = ui
                    .add_enabled(
                        valid && !self.busy(),
                        egui::Button::new("Export to new folder…")
                            .fill(ui.visuals().selection.bg_fill),
                    )
                    .clicked();
                (export, ui.button("Close").clicked())
            })
            .inner
        });
        self.changed_animation_options |= before_preset != project.preset;
        let (export, close) = modal.inner;
        if export {
            self.destination_error = None;
            match choose_folder(&animation_set_file_stem(name(target, project)?)) {
                Ok(Some(path)) => {
                    self.animation_export = None;
                    let message = self.start_animation_export(project, target, path, ctx);
                    self.preview.clear();
                    return message;
                }
                Ok(None) => {}
                Err(error) => {
                    self.destination_error =
                        Some(format!("Could not choose an export folder. {error}"))
                }
            }
        } else if close || modal.should_close() {
            self.animation_export = None;
            self.preview.clear();
        }
        None
    }

    fn fit_status(
        &mut self,
        ui: &mut egui::Ui,
        project: &AnimationProject,
        target: AnimationExport,
    ) -> (Option<String>, bool) {
        let example = match target {
            AnimationExport::Set(id) => {
                animation_set_example_filename(project, id, self.animation_format)
            }
            AnimationExport::Clip(id) => {
                animation_clip_example_filename(project, id, self.animation_format)
            }
        };
        let valid = if example.is_none() {
            self.preview.clear_plan();
            false
        } else if self.animation_format == AnimationSetOutput::PngSequences {
            self.preview.clear_plan();
            let drawing = project.clips[0].drawing_at(0);
            match project.preset.raster_size([drawing.width, drawing.height]) {
                Ok(_) => true,
                Err(e) => {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                    false
                }
            }
        } else {
            self.preview
                .show(ui, project, target, self.animation_format)
        };
        (example, valid)
    }

    fn animation_settings(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut AnimationProject,
        target: AnimationExport,
        sheet: bool,
    ) {
        // All clips in a project share the same artboard.
        let drawing = project.clips[0].drawing_at(0);
        let native = [drawing.width, drawing.height];
        ui.strong("Frame resolution");
        super::resolution::controls(ui, &mut project.preset, native);
        if sheet {
            ui.add_space(5.);
            let clips = project
                .clips
                .iter()
                .filter(|c| target.includes(c))
                .map(|c| c.frames);
            let frames = if self.animation_format == AnimationSetOutput::SharedSheet {
                clips.sum()
            } else {
                clips.max().unwrap_or(1)
            };
            self.sheet_settings(ui, project, frames);
        }
    }

    pub(super) fn start_animation_export(
        &mut self,
        project: &AnimationProject,
        target: AnimationExport,
        path: PathBuf,
        ctx: &egui::Context,
    ) -> Option<String> {
        let settings = AnimationSettings {
            target,
            format: self.animation_format,
            preset: project.preset,
        };
        let plan = self.preview.prepared(project, target, settings.format);
        self.write_animation(project, settings, path, None, plan, ctx)
    }

    fn write_animation(
        &mut self,
        project: &AnimationProject,
        settings: AnimationSettings,
        path: PathBuf,
        previous: Option<ExportReceipt>,
        plan: Option<Arc<project_io::AnimationSheetPreviewPlan>>,
        ctx: &egui::Context,
    ) -> Option<String> {
        let AnimationSettings {
            target,
            format,
            preset,
        } = settings;
        let name = name(target, project)?.to_owned();
        let frames: u32 = project
            .clips
            .iter()
            .filter(|clip| target.includes(clip))
            .map(|clip| clip.frames)
            .sum();
        let sessions = project
            .clips
            .iter()
            .filter(|clip| match target {
                AnimationExport::Set(id) => clip.set_id == id,
                AnimationExport::Clip(id) => clip.id == id,
            })
            .map(Clip::session_id)
            .collect();
        let total = if format == AnimationSetOutput::PngSequences {
            frames
        } else {
            frames * 2
        };
        enum Source {
            Prepared(Arc<project_io::AnimationSheetPreviewPlan>),
            Snapshot(AnimationProject),
        }
        let source = match plan {
            Some(plan) => Source::Prepared(plan),
            None => Source::Snapshot(project.clone()),
        };
        self.spawn(total, ctx, move |progress| {
            let report = |done, total| {
                progress.total.store(total, Ordering::Relaxed);
                progress.done.store(done, Ordering::Relaxed);
                !progress.cancel.load(Ordering::Relaxed)
            };
            let receipt = match source {
                Source::Prepared(plan) => plan.write(&path, previous.as_ref(), report),
                Source::Snapshot(project) => export_animation_files(
                    &path,
                    &project,
                    target,
                    &preset,
                    format,
                    previous.as_ref(),
                    report,
                ),
            }?;
            Ok(Completed::Animation(
                name,
                LastExport {
                    sessions,
                    settings,
                    receipt,
                },
            ))
        })
    }
}

fn clip_checklist(ui: &mut egui::Ui, project: &mut AnimationProject, set_id: u64) -> bool {
    let mut changed = false;
    let mut members: Vec<_> = project
        .clips
        .iter_mut()
        .filter(|c| c.set_id == set_id)
        .collect();
    let some_included = members.iter().any(|clip| clip.include_in_set_export);
    let all_included = members.iter().all(|clip| clip.include_in_set_export);
    ui.horizontal(|ui| {
        ui.strong("Export as named animations");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(some_included, egui::Button::new("None").small())
                .clicked()
            {
                changed = true;
                for clip in &mut members {
                    clip.include_in_set_export = false;
                }
            }
            if ui
                .add_enabled(!all_included, egui::Button::new("All").small())
                .clicked()
            {
                changed = true;
                for clip in &mut members {
                    clip.include_in_set_export = true;
                }
            }
        });
    });
    let row = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt(("set-export-clips", set_id))
        .max_height((ui.ctx().screen_rect().height() - 270.).clamp(row, 124.))
        .show_rows(ui, row, members.len(), |ui, visible| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            for clip in &mut members[visible] {
                ui.push_id(clip.id, |ui| {
                    changed |= ui
                        .checkbox(&mut clip.include_in_set_export, &clip.name)
                        .on_hover_text(&clip.name)
                        .changed();
                });
            }
        });
    ui.weak("Unchecked clips still play inside their parent.");
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::Document;

    #[test]
    fn file_worker_snapshots_the_chosen_format_and_exports_an_explicit_unchecked_clip() {
        let doc = Document {
            width: 8,
            height: 8,
            frames: 2,
            ..Default::default()
        };
        let mut project = AnimationProject::from_document(doc);
        let set = project.clips[0].set_id;
        project.rename_animation_set(set, "Hero").unwrap();
        project.rename_clip(0, "Walk").unwrap();
        let index = project.add_clip_in_set(set, "Boot").unwrap();
        project.clips[index].include_in_set_export = false;
        let boot = project.clips[index].id;
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut exports = Exports::default();
        for (target, format, filename, action) in [
            (
                AnimationExport::Set(set),
                AnimationSetOutput::PngSequences,
                "hero_walk_2.png",
                "Walk",
            ),
            (
                AnimationExport::Clip(boot),
                AnimationSetOutput::SeparateSheets,
                "hero_boot.png",
                "Boot",
            ),
            (
                AnimationExport::Set(set),
                AnimationSetOutput::SharedSheet,
                "hero.png",
                "Walk",
            ),
        ] {
            let path = root.path().join(format!("{format:?}"));
            exports.animation_format = format;
            assert!(exports
                .start_animation_export(&project, target, path.clone(), &ctx)
                .is_none());
            project.clips[0].name = "Changed while rendering".into();
            exports
                .active
                .take()
                .unwrap()
                .worker
                .join()
                .unwrap()
                .unwrap();
            assert!(path.join(filename).is_file());
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path.join("sheet.json")).unwrap()).unwrap();
            let actions = manifest["animations"].as_array().unwrap();
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0]["name"], action);
            project.clips[0].name = "Walk".into();
        }
    }
}
