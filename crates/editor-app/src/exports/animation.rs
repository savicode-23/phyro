use super::{choose_folder, Completed, Exports};
use document_core::{Clip, ExportPreset};
use eframe::egui;
use project_io::{export_selection_animation, SelectionRange, SpriteOutput};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

pub(super) struct SelectionAnimation {
    clip: Clip,
    objects: Vec<u64>,
    reference: u32,
    // The timeline and these controls show one-based frame numbers.
    first: u32,
    last: u32,
    mode: SpriteOutput,
    preset: ExportPreset,
    measurement: super::resolution::Measurement<(u32, u32, SpriteOutput, ExportPreset), FrameSizes>,
}

type FrameSizes = ([u32; 2], Option<[u32; 2]>);

impl SelectionAnimation {
    pub(super) fn new(clip: &Clip, selected: &[u64], frame: u32, preset: ExportPreset) -> Self {
        Self {
            clip: clip.clone(),
            objects: selected.to_vec(),
            reference: frame,
            first: 1,
            last: clip.frames,
            mode: SpriteOutput::Sheet,
            preset,
            measurement: Default::default(),
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) -> bool {
        let count = self.objects.len();
        let noun = if count == 1 { "object" } else { "objects" };
        ui.heading(&self.clip.name);
        ui.label(format!(
            "Captured {count} selected {noun} on frame {} of {}.",
            self.reference + 1,
            self.clip.frames
        ));
        ui.label("A selection follows its layer’s hold. Across new drawings on that layer, use matching object and layer names. Missing artwork stays transparent.");
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let first = ui.label("First frame");
            if ui
                .add(egui::DragValue::new(&mut self.first).range(1..=self.clip.frames))
                .labelled_by(first.id)
                .changed()
            {
                self.last = self.last.max(self.first);
            }
            let last = ui.label("Last frame");
            if ui
                .add(egui::DragValue::new(&mut self.last).range(1..=self.clip.frames))
                .labelled_by(last.id)
                .changed()
            {
                self.first = self.first.min(self.last);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Format");
            ui.selectable_value(&mut self.mode, SpriteOutput::Sheet, "Sprite sheet");
            ui.selectable_value(&mut self.mode, SpriteOutput::Frames, "PNG sequence");
            ui.selectable_value(&mut self.mode, SpriteOutput::SheetAndFrames, "Both");
        });
        let doc = self.clip.drawing_at(self.reference);
        ui.label("Artboard at export scale");
        super::resolution::controls(ui, &mut self.preset, [doc.width, doc.height]);
        ui.horizontal_wrapped(|ui| {
            if matches!(
                self.mode,
                SpriteOutput::Sheet | SpriteOutput::SheetAndFrames
            ) {
                let columns = ui.label("Columns");
                ui.add(egui::DragValue::new(&mut self.preset.columns).range(1..=4096))
                    .labelled_by(columns.id);
            }
        });
        let preset = self.preset;
        let mode = self.mode;
        let range = SelectionRange {
            reference: self.reference,
            first: self.first - 1,
            last: self.last - 1,
        };
        self.measurement
            .update(ui.ctx(), (self.first, self.last, mode, preset), || {
                let clip = self.clip.clone();
                let objects = self.objects.clone();
                move |cancel| {
                    project_io::selection_animation_dimensions(
                        &clip,
                        &objects,
                        range,
                        &preset,
                        mode,
                        |_, _| !cancel.load(Ordering::Relaxed),
                    )
                }
            });
        match &self.measurement.result {
            Some(Ok(([w, h], sheet))) => {
                ui.label(format!("Cropped frame · {w} × {h} px"));
                if let Some([w, h]) = sheet {
                    ui.label(format!("Sprite sheet · {w} × {h} px"));
                }
                true
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                false
            }
            None => {
                ui.label("Measuring PNG dimensions…");
                false
            }
        }
    }
}

impl Exports {
    pub(super) fn selection_animation_window(
        &mut self,
        ctx: &egui::Context,
        active_clip: Option<&Clip>,
    ) -> Option<String> {
        let busy = self.busy();
        let settings = self.selection_animation.as_mut()?;
        if active_clip.is_none_or(|clip| clip.session_id() != settings.clip.session_id()) {
            self.selection_animation = None;
            return None;
        }
        let mut open = true;
        let mut export = false;
        egui::Window::new("Animated selection")
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .default_size([370., 310.])
            .min_size([250., 130.])
            .show(ctx, |ui| {
                let body = (ui.available_height() - 36.).max(70.);
                let ready = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(body)
                    .show(ui, |ui| settings.controls(ui))
                    .inner;
                ui.separator();
                if let Some(error) = &self.destination_error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                let action =
                    egui::Button::new(egui::RichText::new("Export to new folder…").strong())
                        .fill(ui.visuals().selection.bg_fill)
                        .min_size(egui::vec2(0., 24.));
                export = ui.add_enabled(!busy && ready, action).clicked();
            });
        if !open {
            self.selection_animation = None;
        } else if export {
            self.destination_error = None;
            match choose_folder("selection-animation") {
                Ok(Some(path)) => return self.start_selection_animation(path, ctx),
                Ok(None) => {}
                Err(error) => {
                    self.destination_error =
                        Some(format!("Could not choose an export folder. {error}"))
                }
            }
        }
        None
    }

    fn start_selection_animation(&mut self, path: PathBuf, ctx: &egui::Context) -> Option<String> {
        let settings = self.selection_animation.take()?;
        let range = SelectionRange {
            reference: settings.reference,
            first: settings.first - 1,
            last: settings.last - 1,
        };
        let count = range.last - range.first + 1;
        self.spawn(count * 2, ctx, move |progress| {
            export_selection_animation(
                &path,
                &settings.clip,
                &settings.objects,
                range,
                &settings.preset,
                settings.mode,
                |done, total| {
                    progress.total.store(total, Ordering::Relaxed);
                    progress.done.store(done, Ordering::Relaxed);
                    !progress.cancel.load(Ordering::Relaxed)
                },
            )?;
            Ok(Completed::SelectionAnimation(path, count))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::AnimationProject;
    use std::{fs, sync::Arc};

    #[test]
    fn sheet_exports_use_the_chosen_columns_with_or_without_individual_frames() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        let object = clip
            .drawing_at_mut(0)
            .add_rectangle([0.; 2], [4., 2.], [255; 4]);
        let temp = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        for (mode, folder) in [
            (SpriteOutput::Sheet, "sheet"),
            (SpriteOutput::SheetAndFrames, "both"),
        ] {
            let mut settings = SelectionAnimation::new(clip, &[object], 0, project.preset);
            settings.mode = mode;
            settings.last = 3;
            settings.preset.columns = 2;
            let mut exports = Exports {
                selection_animation: Some(settings),
                ..Default::default()
            };
            let path = temp.path().join(folder);
            assert!(exports
                .start_selection_animation(path.clone(), &ctx)
                .is_none());
            exports
                .active
                .take()
                .unwrap()
                .worker
                .join()
                .unwrap()
                .unwrap();
            let sheet = renderer::Pixmap::load_png(path.join("sheet.png")).unwrap();
            assert_eq!([sheet.width(), sheet.height()], [8, 4]);
            assert_eq!(
                path.join("frames/frame_0002.png").is_file(),
                mode == SpriteOutput::SheetAndFrames
            );
        }
    }

    #[test]
    fn animated_worker_keeps_the_captured_reference_and_inclusive_display_range() {
        let mut project = AnimationProject::default();
        project.preset.scale = 2;
        let clip = &mut project.clips[0];
        let doc = clip.drawing_at_mut(0);
        doc.add_rectangle([-2., 2.], [2., 2.], [255, 0, 0, 255]);
        doc.objects[0].name = "Eye".into();
        let mut missing = doc.clone();
        missing.objects[0].name = "Other".into();
        let mut later = missing.clone();
        let selected = later.add_rectangle([2., 2.], [2., 2.], [0, 210, 90, 255]);
        later.objects[1].name = "Eye".into();
        clip.cels.insert(2, Arc::new(missing));
        clip.cels.insert(3, Arc::new(later));

        let mut settings = SelectionAnimation::new(clip, &[selected], 3, project.preset);
        assert_eq!([settings.first, settings.last], [1, clip.frames]);
        settings.first = 2;
        settings.last = 4;
        settings.mode = SpriteOutput::Frames;
        let mut exports = Exports {
            selection_animation: Some(settings),
            ..Default::default()
        };
        // The dialog refers to the selection at opening, even if the live clip changes.
        clip.drawing_at_mut(3).objects[1].fill = [0, 0, 0, 0];
        clip.delete_range(2, clip.frames - 1).unwrap();
        project.preset.scale = 1;
        let ctx = egui::Context::default();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("animated");
        assert!(exports
            .start_selection_animation(path.clone(), &ctx)
            .is_none());
        let result = exports
            .active
            .take()
            .unwrap()
            .worker
            .join()
            .unwrap()
            .unwrap();
        assert!(matches!(result, Completed::SelectionAnimation(output, 3) if output == path));

        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("sheet.json")).unwrap()).unwrap();
        assert_eq!(metadata["frames"].as_array().unwrap().len(), 3);
        let frame =
            |i| renderer::Pixmap::load_png(path.join(format!("frames/frame_{i:04}.png"))).unwrap();
        let first = frame(0);
        assert_eq!([first.width(), first.height()], [12, 4]);
        assert_eq!(first.pixel(1, 1).unwrap().red(), 255);
        assert!(frame(1).pixels().iter().all(|p| p.alpha() == 0));
        assert_eq!(frame(2).pixel(9, 1).unwrap().green(), 210);
    }
}
