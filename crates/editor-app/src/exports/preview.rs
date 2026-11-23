use super::AnimationExport;
use document_core::{AnimationProject, ExportPreset};
use eframe::egui::{self, Color32, ColorImage, TextureHandle};
use project_io::{AnimationSetOutput, AnimationSheetPreviewPlan};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

enum PreviewResult {
    Plan(Arc<AnimationSheetPreviewPlan>),
    Thumbnail(ColorImage),
}

struct Job {
    worker: JoinHandle<Result<PreviewResult, String>>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    artwork_revision: u64,
    target: AnimationExport,
    preset: ExportPreset,
    format: AnimationSetOutput,
    // Only the export dialog can change membership while its artwork snapshot is open.
    clips: Vec<u64>,
}

impl Key {
    fn matches(
        &self,
        revision: u64,
        project: &AnimationProject,
        target: AnimationExport,
        format: AnimationSetOutput,
    ) -> bool {
        self.artwork_revision == revision
            && self.target == target
            && self.preset == project.preset
            && self.format == format
            && self.clips.iter().copied().eq(project
                .clips
                .iter()
                .filter(|c| target.includes(c))
                .map(|c| c.id))
    }
}

#[derive(Default)]
pub(super) struct Preview {
    revision: u64,
    artwork: Option<Arc<AnimationProject>>,
    key: Option<Key>,
    plan: Option<Arc<AnimationSheetPreviewPlan>>,
    sheet: usize,
    texture: Option<TextureHandle>,
    error: Option<String>,
    job: Option<Job>,
}

impl Preview {
    pub(super) fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.artwork = None;
        self.clear_plan();
    }

    pub(super) fn clear_plan(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        if self
            .job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            self.job = None;
        }
        self.key = None;
        self.plan = None;
        self.sheet = 0;
        self.texture = None;
        self.error = None;
    }

    pub(super) fn prepared(
        &self,
        project: &AnimationProject,
        target: AnimationExport,
        format: AnimationSetOutput,
    ) -> Option<Arc<AnimationSheetPreviewPlan>> {
        self.key
            .as_ref()
            .filter(|key| key.matches(self.revision, project, target, format))?;
        self.plan.clone()
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(Arc<AtomicBool>) -> Result<PreviewResult, String> + Send + 'static,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        let ctx = ctx.clone();
        match thread::Builder::new()
            .name("sheet-preview".into())
            .spawn(move || {
                let result = work(Arc::clone(&stop));
                ctx.request_repaint();
                if stop.load(Ordering::Relaxed) {
                    Err("Preview cancelled.".into())
                } else {
                    result
                }
            }) {
            Ok(worker) => self.job = Some(Job { worker, cancel }),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self
            .job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.job.take().unwrap();
            let result = job.worker.join();
            if job.cancel.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(Ok(PreviewResult::Plan(plan))) => self.plan = Some(plan),
                Ok(Ok(PreviewResult::Thumbnail(image))) => {
                    self.texture = Some(ctx.load_texture(
                        "Sprite sheet preview",
                        image,
                        egui::TextureOptions::NEAREST,
                    ));
                }
                Ok(Err(e)) => self.error = Some(e),
                Err(_) => self.error = Some("Could not generate the sheet preview.".into()),
            }
        }
    }

    /// Preflight every selected sheet on a worker. Rasterize a preview only when opened.
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        project: &AnimationProject,
        target: AnimationExport,
        format: AnimationSetOutput,
    ) -> bool {
        if self
            .key
            .as_ref()
            .is_none_or(|key| !key.matches(self.revision, project, target, format))
        {
            self.clear_plan();
            self.poll(ui.ctx());
            if self.job.is_none() {
                let snapshot = self
                    .artwork
                    .get_or_insert_with(|| Arc::new(project.clone()))
                    .clone();
                let key = Key {
                    artwork_revision: self.revision,
                    target,
                    preset: project.preset,
                    format,
                    clips: project
                        .clips
                        .iter()
                        .filter(|c| target.includes(c))
                        .map(|c| c.id)
                        .collect(),
                };
                self.key = Some(key.clone());
                self.start(ui.ctx(), move |stop| {
                    AnimationSheetPreviewPlan::prepare_snapshot(
                        snapshot,
                        target,
                        &key.preset,
                        format,
                        &key.clips,
                        |_, _| !stop.load(Ordering::Relaxed),
                    )
                    .map(|plan| PreviewResult::Plan(Arc::new(plan)))
                    .map_err(|e| e.to_string())
                });
            }
        }
        self.poll(ui.ctx());
        if let Some(message) = &self.error {
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
        let Some(plan) = self.plan.clone() else {
            if self.error.is_none() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Measuring sheet…");
                });
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
            return false;
        };
        let sheets = plan.sheets();
        let mut index = self.sheet;
        if sheets.len() > 1 {
            egui::ComboBox::from_id_salt("preview-sheet")
                .selected_text(&sheets[index].0)
                .show_ui(ui, |ui| {
                    for (n, (name, _)) in sheets.iter().enumerate() {
                        ui.selectable_value(&mut index, n, name);
                    }
                });
        }
        if index != self.sheet {
            self.sheet = index;
            self.texture = None;
            self.error = None;
            if let Some(job) = &self.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        let [w, h] = sheets[index].1;
        ui.label(format!("Sheet size   {w} × {h} px"));
        ui.collapsing("Preview artwork", |ui| {
            if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let zoom = (ui.available_width() / size.x).min(145. / size.y).min(2.);
                ui.image((texture.id(), size * zoom));
            } else if self.job.is_none() && self.error.is_none() {
                self.start(ui.ctx(), move |stop| {
                    let image = plan
                        .render(index, |_, _| !stop.load(Ordering::Relaxed))
                        .map_err(|e| e.to_string())?;
                    Ok(PreviewResult::Thumbnail(thumbnail(&image)))
                });
            }
            if self.job.is_some() {
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        });
        true
    }
}

fn thumbnail(image: &renderer::Pixmap) -> ColorImage {
    let ratio = (512. / f64::from(image.width().max(image.height()))).min(1.);
    let w = (f64::from(image.width()) * ratio).round().max(1.) as usize;
    let h = (f64::from(image.height()) * ratio).round().max(1.) as usize;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let ix = x * (image.width() as usize) / w;
            let iy = y * (image.height() as usize) / h;
            let pixel = image.pixels()[iy * image.width() as usize + ix];
            let gray: u16 = if (x / 8 + y / 8) % 2 == 0 { 72 } else { 88 };
            let backdrop = (gray * (255 - u16::from(pixel.alpha())) / 255) as u8;
            pixels.push(Color32::from_rgb(
                pixel.red().saturating_add(backdrop),
                pixel.green().saturating_add(backdrop),
                pixel.blue().saturating_add(backdrop),
            ));
        }
    }
    ColorImage {
        size: [w, h],
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_and_set_membership_reuse_only_the_artwork_snapshot() {
        let mut project = AnimationProject::from_document(document_core::Document {
            width: 16,
            height: 8,
            frames: 1,
            ..Default::default()
        });
        project.preset.columns = 1;
        let set = project.clips[0].set_id;
        let second = project.add_clip_in_set(set, "Jump").unwrap();
        let ctx = egui::Context::default();
        let target = AnimationExport::Set(set);
        let output = AnimationSetOutput::SeparateSheets;
        let mut preview = Preview::default();
        let ready = |preview: &mut Preview, p: &AnimationProject| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let _ = ctx.run(Default::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        preview.show(ui, p, target, output);
                    });
                });
                if let Some(plan) = preview.prepared(p, target, output) {
                    break plan;
                }
                assert!(std::time::Instant::now() < deadline);
                thread::yield_now();
            }
        };
        assert_eq!(ready(&mut preview, &project).sheets().len(), 2);
        let source = preview.artwork.clone().unwrap();
        project.clips[second].include_in_set_export = false;
        project.preset.set_raster_ratio(1, 2);
        assert!(preview.prepared(&project, target, output).is_none());
        let next = ready(&mut preview, &project);
        assert_eq!(next.sheets().len(), 1);
        assert_eq!(next.sheets()[0].1, [8, 4]);
        assert!(Arc::ptr_eq(&source, preview.artwork.as_ref().unwrap()));
        project.clips[0]
            .drawing_at_mut(0)
            .add_rectangle([0., 0.], [16., 8.], [255; 4]);
        preview.clear();
        assert!(preview.prepared(&project, target, output).is_none());
        let refreshed = ready(&mut preview, &project)
            .render(0, |_, _| true)
            .unwrap();
        assert_eq!(refreshed.pixel(2, 2).unwrap().alpha(), 255);
        assert!(!Arc::ptr_eq(&source, preview.artwork.as_ref().unwrap()));
    }
}
