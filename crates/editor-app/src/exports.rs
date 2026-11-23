mod animation;
mod animation_files;
mod preview;
mod resolution;
mod sheet_settings;
mod still;

use animation::SelectionAnimation;
use animation_files::LastExport;
use document_core::AnimationProject;
use eframe::egui;
use preview::Preview;
pub(crate) use project_io::AnimationExportTarget as AnimationExport;
use project_io::AnimationSetOutput;
use project_io::StillFormat;
use rfd::FileDialog;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Exports {
    active: Option<Job>,
    previous: Vec<LastExport>,
    generation: u64,
    still_export: Option<still::Settings>,
    selection_animation: Option<SelectionAnimation>,
    animation_export: Option<AnimationExport>,
    animation_format: AnimationSetOutput,
    changed_animation_options: bool,
    preset_name: String,
    preview: Preview,
    destination_error: Option<String>,
}
impl Default for Exports {
    fn default() -> Self {
        Self {
            active: None,
            previous: Vec::new(),
            generation: 0,
            still_export: None,
            selection_animation: None,
            animation_export: None,
            animation_format: AnimationSetOutput::SharedSheet,
            changed_animation_options: false,
            preset_name: String::new(),
            preview: Preview::default(),
            destination_error: None,
        }
    }
}
struct Job {
    worker: JoinHandle<io::Result<Completed>>,
    progress: Arc<Progress>,
    generation: u64,
}
#[derive(Default)]
struct Progress {
    cancel: AtomicBool,
    done: AtomicU32,
    total: AtomicU32,
}
enum Completed {
    Frame(PathBuf),
    Animation(String, LastExport),
    Objects(PathBuf, usize, StillFormat),
    SelectionAnimation(PathBuf, u32),
}

impl Exports {
    pub fn is_open(&self) -> bool {
        self.still_export.is_some()
            || self.selection_animation.is_some()
            || self.animation_export.is_some()
    }

    pub(crate) fn take_animation_option_edit(&mut self) -> bool {
        std::mem::take(&mut self.changed_animation_options)
    }

    pub(crate) fn artwork_changed(&mut self) {
        self.changed_animation_options = false;
        self.preview.clear();
    }

    pub fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub fn reset_destination(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.previous.clear();
        self.changed_animation_options = false;
        self.still_export = None;
        self.selection_animation = None;
        self.animation_export = None;
        self.preview.clear();
        self.preset_name.clear();
        self.destination_error = None;
    }
    pub fn menu(
        &mut self,
        ui: &mut egui::Ui,
        p: &AnimationProject,
        index: usize,
        frame: u32,
        selected: &[u64],
    ) -> Option<String> {
        let clip = p.clips.get(index)?;
        let mut message = None;
        ui.add_enabled_ui(!self.busy(), |ui| {
            message = self.still_menu(ui, clip, frame, selected, p.preset);
            ui.separator();
            ui.weak("Animation");
            if ui.button("Export clip…").clicked() {
                ui.close_menu();
                message = self.open_animation_export(AnimationExport::Clip(clip.id));
            }
            if ui.button("Export animation set…").clicked() {
                ui.close_menu();
                message = self.open_animation_export(AnimationExport::Set(clip.set_id));
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new("Animated selection…"),
                )
                .clicked()
            {
                ui.close_menu();
                self.animation_export = None;
                self.preview.clear();
                self.still_export = None;
                self.destination_error = None;
                self.selection_animation =
                    Some(SelectionAnimation::new(clip, selected, frame, p.preset));
            }
        });
        message
    }
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        p: &mut AnimationProject,
        index: usize,
    ) -> Option<String> {
        if self.still_export.is_some() {
            return self.still_window(ctx);
        }
        if let Some(target) = self.animation_export {
            return self.animation_export_dialog(ctx, p, target);
        }
        self.selection_animation_window(ctx, p.clips.get(index))
    }
    fn spawn(
        &mut self,
        total: u32,
        ctx: &egui::Context,
        worker: impl FnOnce(&Progress) -> io::Result<Completed> + Send + 'static,
    ) -> Option<String> {
        if self.busy() {
            return Some("An export is already running.".into());
        }
        let progress = Arc::new(Progress {
            total: AtomicU32::new(total),
            ..Default::default()
        });
        let signal = progress.clone();
        let ctx = ctx.clone();
        match thread::Builder::new()
            .name("sprite-export".into())
            .spawn(move || {
                let result = worker(&signal);
                ctx.request_repaint();
                result
            }) {
            Ok(worker) => {
                self.active = Some(Job {
                    worker,
                    progress,
                    generation: self.generation,
                });
                None
            }
            Err(e) => Some(format!("Could not start export: {e}")),
        }
    }
    pub fn cancel(&mut self) {
        if let Some(job) = &self.active {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn status(&mut self, ui: &mut egui::Ui) -> Option<Result<String, String>> {
        let job = self.active.as_ref()?;
        if job.worker.is_finished() {
            let job = self.active.take()?;
            let message = match job.worker.join() {
                Ok(Ok(Completed::Frame(path))) => format!("Exported {}", path.display()),
                Ok(Ok(Completed::Objects(path, count, format))) => {
                    let noun = if count == 1 { "file" } else { "files" };
                    format!(
                        "Exported {count} {} {noun} to {}",
                        still::label(format),
                        path.display()
                    )
                }
                Ok(Ok(Completed::Animation(name, entry))) => {
                    let message =
                        format!("Exported {name} to {}", entry.receipt.directory().display());
                    if job.generation == self.generation {
                        self.previous.retain(|old| old.target() != entry.target());
                        self.previous.push(entry);
                    }
                    message
                }
                Ok(Ok(Completed::SelectionAnimation(path, count))) => {
                    let noun = if count == 1 { "frame" } else { "frames" };
                    format!("Exported {count} {noun} to {}", path.display())
                }
                Ok(Err(e))
                    if e.kind() == io::ErrorKind::Interrupted
                        && job.progress.cancel.load(Ordering::Relaxed) =>
                {
                    "Export cancelled.".into()
                }
                Ok(Err(e)) => return Some(Err(e.to_string())),
                Err(_) => return Some(Err("The export worker stopped unexpectedly.".into())),
            };
            return Some(Ok(message));
        }
        ui.ctx().request_repaint_after(Duration::from_millis(80));
        ui.horizontal(|ui| {
            let total = job.progress.total.load(Ordering::Relaxed);
            ui.spinner();
            ui.label(format!(
                "Exporting {}/{}",
                job.progress.done.load(Ordering::Relaxed),
                total
            ));
            if total > 1 && ui.small_button("Cancel").clicked() {
                job.progress.cancel.store(true, Ordering::Relaxed);
            }
        });
        None
    }
}

fn choose_folder(stem: &str) -> io::Result<Option<PathBuf>> {
    let Some(parent) = crate::diagnostics::native_dialog(|| {
        FileDialog::new()
            .set_title("Choose the parent for a new export folder")
            .pick_folder()
    }) else {
        return Ok(None);
    };
    let mut number = 1;
    loop {
        let path = parent.join(if number == 1 {
            stem.to_owned()
        } else {
            format!("{stem}-{number}")
        });
        match fs::symlink_metadata(&path) {
            Ok(_) => number += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Some(path)),
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn selection_worker_exports_the_displayed_exposure_and_scale_while_edits_continue() {
        let mut project = AnimationProject::default();
        project.preset.scale = 2;
        let clip = &mut project.clips[0];
        let first = clip.drawing_at_mut(0);
        let a = first.add_rectangle([4., 0.], [4., 4.], [255, 0, 0, 255]);
        let mut second = first.clone();
        let green = [0, 170, 30, 255];
        second.objects[0].fill = green;
        second.objects[0]
            .keys
            .extend([(2, [-14., -8.]), (4, [-10., -8.])]);
        let b = second.add_rectangle([-6., -8.], [2., 4.], [30, 40, 210, 255]);
        crate::commands::group(&mut second, &[a, b]);
        let c = second.add_rectangle([-2., -8.], [2., 2.], [255, 200, 0, 255]);
        second.add_rectangle([-12., -8.], [12., 4.], [200, 200, 200, 255]);
        clip.cels.insert(2, Arc::new(second));

        let ctx = egui::Context::default();
        let target = tempfile::tempdir().unwrap();
        let mut exports = Exports::default();
        let project_at_click = project.clone();
        let combined = target.path().join("selection.png");
        let capture = still::Image::new(&project.clips[0], 3, still::Scope::Selection, &[a, c], 2);
        assert!(exports
            .write_still(capture, combined.clone(), StillFormat::Png, false, &ctx)
            .is_none());
        project.clips[0].drawing_at_mut(3).objects[0].fill = [90; 4];
        project.preset.scale = 3;
        finish(&mut exports, &ctx);
        let image = renderer::Pixmap::load_png(combined).unwrap();
        assert_eq!([image.width(), image.height()], [24, 8]);
        assert_eq!(image.pixel(2, 2).unwrap().green(), green[1]);
        assert_eq!(image.pixel(9, 2).unwrap().alpha(), 0);
        assert_eq!(image.pixel(13, 2).unwrap().blue(), 210);
        assert_eq!(image.pixel(21, 2).unwrap().red(), 255);

        let folder = target.path().join("objects");
        let capture = still::Image::new(
            &project_at_click.clips[0],
            3,
            still::Scope::Batch,
            &[a, c],
            project_at_click.preset.scale,
        );
        assert!(exports
            .write_still(capture, folder.clone(), StillFormat::Png, false, &ctx)
            .is_none());
        finish(&mut exports, &ctx);
        let mut sizes: Vec<_> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| {
                let image = renderer::Pixmap::load_png(entry.unwrap().path()).unwrap();
                [image.width(), image.height()]
            })
            .collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [[4, 4], [16, 8]]);
        assert!(!exports.can_reexport(&project_at_click, 0));
    }

    #[test]
    fn svg_batch_exports_a_group_and_a_loose_object_from_the_captured_drawing() {
        let mut project = AnimationProject::default();
        project.preset.scale = 3;
        let clip = &mut project.clips[0];
        let first = clip.drawing_at_mut(0);
        let a = first.add_rectangle([-20., 0.], [4., 2.], [255, 0, 0, 255]);
        let mut pose = first.clone();
        pose.objects[0].fill = [0, 153, 75, 255];
        let b = pose.add_rectangle([-14., 0.], [4., 2.], [30, 40, 210, 255]);
        crate::commands::group(&mut pose, &[a, b]);
        let group = pose.objects[0].group.unwrap();
        pose.rename_group(1, group, "Wings").unwrap();
        let c = pose.add_rectangle([16., 2.], [3., 2.], [255, 202, 0, 255]);
        pose.objects[2].name = "Spark".into();
        pose.add_rectangle([16., 2.], [3., 2.], [0, 0, 0, 255]);
        clip.cels.insert(2, Arc::new(pose));

        let ctx = egui::Context::default();
        let mut exports = Exports::default();
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("objects-svg");
        let capture = still::Image::new(
            &project.clips[0],
            3,
            still::Scope::Batch,
            &[a, c],
            project.preset.scale,
        );
        assert!(exports
            .write_still(capture, folder.clone(), StillFormat::Svg, false, &ctx)
            .is_none());
        project.clips[0].drawing_at_mut(3).objects[0].fill = [220, 1, 1, 255];
        let message = finish(&mut exports, &ctx);
        assert_eq!(
            message,
            format!("Exported 2 SVG files to {}", folder.display())
        );
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 2);

        let wings = project_io::import_svg(&folder.join("Wings.svg")).unwrap();
        assert_eq!(wings.objects.len(), 2);
        assert_eq!(wings.objects[0].fill, [0, 153, 75, 255]);
        assert_eq!(wings.objects[1].fill, [30, 40, 210, 255]);
        let spark = project_io::import_svg(&folder.join("Spark.svg")).unwrap();
        assert_eq!(spark.objects.len(), 1);
        assert_eq!(spark.objects[0].fill, [255, 202, 0, 255]);
        assert_eq!([spark.width, spark.height], [3, 2]);
    }

    fn finish(exports: &mut Exports, ctx: &egui::Context) -> String {
        finish_result(exports, ctx).expect("successful export")
    }

    fn finish_result(exports: &mut Exports, ctx: &egui::Context) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut message = None;
        while exports.busy() {
            assert!(Instant::now() < deadline, "export worker timed out");
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(status) = exports.status(ui) {
                        message = Some(status);
                    }
                });
            });
            thread::yield_now();
        }
        message.expect("export result")
    }

    #[test]
    fn export_failure_is_distinct_from_cancellation() {
        let ctx = egui::Context::default();
        let mut exports = Exports::default();
        exports.spawn(2, &ctx, |_| Err(io::Error::other("Disk is full")));
        assert_eq!(
            finish_result(&mut exports, &ctx),
            Err("Disk is full".into())
        );

        exports.spawn(2, &ctx, |_| Err(io::ErrorKind::Interrupted.into()));
        exports.cancel();
        assert_eq!(
            finish_result(&mut exports, &ctx),
            Ok("Export cancelled.".into())
        );
    }

    #[test]
    fn packed_reexport_uses_latest_drawing_and_keeps_godot_sidecar() {
        let mut project = AnimationProject::default();
        let doc = project.clips[0].drawing_at_mut(0);
        doc.add_rectangle([3., 4.], [4., 5.], [20, 80, 150, 255]);
        let ctx = egui::Context::default();
        let mut exports = Exports::default();
        let target = tempfile::tempdir().unwrap();
        let path = target.path().join("packed");
        project.preset.packed.enabled = true;
        exports.start_animation_export(
            &project,
            AnimationExport::Clip(project.clips[0].id),
            path.clone(),
            &ctx,
        );
        finish(&mut exports, &ctx);
        let manifest = path.join("animation.phyro");
        let before: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        let sidecar = path.join("animation.phyro.import");
        fs::write(&sidecar, b"Godot-owned settings").unwrap();
        project.clips[0].drawing_at_mut(0).objects[0].position = [7., 2.];
        project.rename_clip(0, "Renamed").unwrap();
        assert!(exports.can_reexport(&project, 0));
        exports.reexport(&project, 0, &ctx);
        finish(&mut exports, &ctx);
        let after: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
        assert_eq!(before["animations"][0]["frames"][0]["trim_x"], 3);
        assert_eq!(after["animations"][0]["frames"][0]["trim_x"], 7);
        assert_eq!(after["animations"][0]["name"], "Renamed");
        assert_eq!(fs::read(sidecar).unwrap(), b"Godot-owned settings");
    }

    #[test]
    fn reexport_from_a_member_reuses_the_sets_saved_resolution_and_inclusion() {
        let doc = document_core::Document {
            width: 20,
            height: 12,
            frames: 2,
            ..Default::default()
        };
        let mut project = AnimationProject::from_document(doc);
        let id = project.clips[0].set_id;
        project.rename_animation_set(id, "Hero").unwrap();
        project.rename_clip(0, "Walk").unwrap();
        let run = project.add_clip_in_set(id, "Run").unwrap();
        let enemy = project.add_animation_set("Enemy").unwrap();
        let other = project.add_clip_in_set(enemy, "Walk").unwrap();
        project.preset.columns = 2;
        project.preset.set_raster_ratio(1, 2);
        let ctx = egui::Context::default();
        let mut exports = Exports {
            animation_format: AnimationSetOutput::SeparateSheets,
            ..Default::default()
        };
        let target = tempfile::tempdir().unwrap();
        let path = target.path().join("hero");
        exports.start_animation_export(&project, AnimationExport::Set(id), path.clone(), &ctx);
        finish(&mut exports, &ctx);
        assert!(path.join("hero_run.png").is_file());
        assert!(exports.can_reexport(&project, run));
        assert!(!exports.can_reexport(&project, other));

        project.preset.set_raster_ratio(2, 1);
        project.clips[run].include_in_set_export = false;
        std::fs::write(path.join("animation.phyro.import"), b"Godot import options").unwrap();
        exports.reexport(&project, run, &ctx);
        finish(&mut exports, &ctx);
        assert!(!path.join("hero_run.png").exists());
        assert_eq!(
            std::fs::read(path.join("animation.phyro.import")).unwrap(),
            b"Godot import options"
        );
        let image = renderer::Pixmap::load_png(path.join("hero_walk.png")).unwrap();
        assert_eq!([image.width(), image.height()], [20, 6]);
    }

    #[test]
    fn export_authorization_follows_the_original_clip_across_rename_and_undo() {
        let mut project = AnimationProject::default();
        project.add_clip("Other").unwrap();
        let snapshot = project.clone();
        let ctx = egui::Context::default();
        let mut exports = Exports::default();
        let target = tempfile::tempdir().unwrap();
        let path = target.path().join("output");
        exports.start_animation_export(
            &project,
            AnimationExport::Clip(project.clips[0].id),
            path.clone(),
            &ctx,
        );
        // Replace the clip while the worker is still in flight.
        project.remove_clip(0).unwrap();
        let replacement = project.add_clip("Animation").unwrap();
        finish(&mut exports, &ctx);
        assert!(!exports.can_reexport(&project, replacement));
        assert!(!exports.can_reexport(&project, 0));
        assert!(exports.can_reexport(&snapshot, 0));
        let mut restored = snapshot;
        restored.rename_clip(0, "Renamed").unwrap();
        restored.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([1., 1.], [5., 4.], [255; 4]);
        assert!(exports.can_reexport(&restored, 0));
        restored.duplicate_clip(0, "Animation").unwrap();
        assert!(!exports.can_reexport(&restored, 1));
        exports.reexport(&restored, 0, &ctx);
        exports.reset_destination();
        finish(&mut exports, &ctx);
        assert!(!exports.can_reexport(&restored, 0));
        assert!(path.join("sheet.json").is_file());
    }
}
