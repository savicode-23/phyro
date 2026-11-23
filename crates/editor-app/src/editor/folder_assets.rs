use super::{commands, Editor, Tool};
use crate::canvas::{PlacementPage, PlacementResolution};
use crate::library::AssetSource;
use document_core::{AnimationProject, Shape};
use eframe::egui;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const PLACEMENT_INDICATOR_DELAY: Duration = Duration::from_millis(120);

#[derive(Default)]
pub(super) struct Placements {
    pending: Option<Pending>,
}

impl Placements {
    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn loading_point(
        &self,
        ctx: &egui::Context,
        clip: usize,
        frame: u32,
        layer: u64,
    ) -> Option<[f32; 2]> {
        let job = self.pending.as_ref()?;
        let target = &job.target;
        ((target.clip, target.frame, target.layer) == (clip, frame, layer)
            && ctx.input(|input| input.time >= job.show_indicator_at))
        .then_some(target.point)
    }
}

struct Placed {
    project: AnimationProject,
    id: u64,
    page: Option<PlacementPage>,
}

type ResultMessage = Result<Placed, String>;

struct Pending {
    label: String,
    target: Target,
    receiver: mpsc::Receiver<ResultMessage>,
    cancel: Arc<AtomicBool>,
    show_indicator_at: f64,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct Target {
    // Cels share their immutable artwork with the editor's last completed edit.
    before: Arc<AnimationProject>,
    clip: usize,
    frame: u32,
    layer: u64,
    point: [f32; 2],
    resolution: Option<PlacementResolution>,
}

impl Editor {
    pub(super) fn place_asset(
        &mut self,
        ctx: &egui::Context,
        source: AssetSource,
        label: String,
        point: [f32; 2],
    ) {
        if self.folder_assets.is_pending() {
            self.status = "An asset is already loading.".into();
            return;
        }
        if !commands::is_editable(self.scene(), self.style.layer) {
            self.report_warning(
                "Could not place artwork",
                "Choose a visible, unlocked layer to place artwork.",
            );
            return;
        }
        if self.canvas.path_in_progress() || self.canvas.text_active() || self.library.is_editing()
        {
            self.report_warning(
                "Could not place artwork",
                "Finish the current edit before placing artwork.",
            );
            return;
        }
        self.timeline.stop();
        self.finish_edit();
        let target = Target {
            before: Arc::new(self.baseline.clone()),
            clip: self.timeline.clip,
            frame: self.timeline.frame,
            layer: self.style.layer,
            point,
            resolution: self.canvas.placement_resolution(ctx),
        };
        let snapshot = target.clone();
        let project_path = self.path.clone();
        let wake = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("library-placement".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    prepare_placement(&source, snapshot, project_path.as_deref(), &stop)
                }))
                .unwrap_or_else(|_| Err("The asset loading task stopped unexpectedly.".into()));
                if sender.send(result).is_ok() {
                    wake.request_repaint();
                }
            }) {
            Ok(_) => {
                self.status = format!("Placing {label}…");
                self.folder_assets.pending = Some(Pending {
                    label,
                    target,
                    receiver,
                    cancel,
                    show_indicator_at: ctx.input(|input| input.time)
                        + PLACEMENT_INDICATOR_DELAY.as_secs_f64(),
                });
                ctx.request_repaint_after(PLACEMENT_INDICATOR_DELAY);
            }
            Err(error) => self.report_error("Could not start asset loading", error.to_string()),
        }
    }

    pub(super) fn poll_folder_placement(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.folder_assets.pending else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The asset loading task stopped.".into()),
        };
        let job = self.folder_assets.pending.take().unwrap();
        match result {
            Ok(Placed { project, id, page }) => {
                let target = &job.target;
                if self.timeline.clip != target.clip
                    || self.timeline.frame != target.frame
                    || self.style.layer != target.layer
                    || self.project != *target.before
                {
                    self.report_warning(
                        "Could not place artwork",
                        "The drawing changed while loading. Drag the asset again to place it here.",
                    );
                    return;
                }
                self.project = project;
                self.selection = vec![id];
                self.set_tool(Tool::Select);
                // The worker already hydrated and validated this exact snapshot.
                // Publish its transaction without walking the dependency graph again.
                self.history
                    .record(&target.before, &self.project, target.clip, target.frame);
                self.pending_edit = false;
                self.capture_baseline();
                if let Some(page) = page {
                    self.canvas.install_placement_page(ctx, page);
                }
                self.status = format!("Placed {}.", job.label);
            }
            Err(error) => {
                self.report_error("Could not place artwork", format!("{}\n{error}", job.label))
            }
        }
        ctx.request_repaint();
    }
}

fn prepare_placement(
    source: &AssetSource,
    target: Target,
    current_path: Option<&Path>,
    cancel: &AtomicBool,
) -> ResultMessage {
    let mut before = target.before.as_ref().clone();
    let clip = target.clip;
    let frame = target.frame;
    let id = match source {
        AssetSource::ProjectClip { id, session } => {
            let source = before
                .clip_index(*id)
                .filter(|&i| before.clips[i].session_id() == *session)
                .ok_or("The source clip is no longer in this project.")?;
            place_project_clip(&mut before, source, &target)?
        }
        AssetSource::File { path, clip_id } => {
            let id = place_file(&mut before, path, *clip_id, &target, current_path, cancel)?;
            check_cancel(cancel)?;
            before.clips[clip].sync_layer_edits(&target.before.clips[clip], frame)?;
            before.hydrate_composition()?;
            id
        }
    };
    check_cancel(cancel)?;
    // Check native archive and embedded-image budgets on the worker too, so the
    // next ordinary save can keep everything shown in this placement.
    project_io::validate_image_import(&before).map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    let page = target.resolution.and_then(|resolution| {
        let (_, drawing) = before.clips[clip].cels.range(..=frame).next_back()?;
        PlacementPage::render(drawing, frame, id, resolution)
    });
    Ok(Placed {
        project: before,
        id,
        page,
    })
}

fn place_project_clip(
    project: &mut AnimationProject,
    source: usize,
    target: &Target,
) -> Result<u64, String> {
    let child = &project.clips[source];
    let drawing = child.drawing_at(0);
    let position = [
        target.point[0] - drawing.width as f32 * 0.5,
        target.point[1] - drawing.height as f32 * 0.5,
    ];
    project.place_clip(child.id, target.clip, target.frame, target.layer, position)
}

fn place_file(
    before: &mut AnimationProject,
    path: &Path,
    clip_id: Option<u64>,
    target: &Target,
    current_path: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let (clip, frame, layer, point) = (target.clip, target.frame, target.layer, target.point);
    let name: String = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    let name = if name.trim().is_empty() {
        "Artwork"
    } else {
        name.trim()
    };
    let center = |s: [f32; 2]| [point[0] - s[0] * 0.5, point[1] - s[1] * 0.5];
    let id = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "vanim" => {
            if current_path.is_some_and(|open| {
                open == path
                    || open
                        .canonicalize()
                        .ok()
                        .zip(path.canonicalize().ok())
                        .is_some_and(|(a, b)| a == b)
            }) {
                let child = clip_id.unwrap_or(before.clips[0].id);
                let index = before
                    .clip_index(child)
                    .ok_or("The source clip no longer exists.")?;
                place_project_clip(before, index, target)?
            } else {
                let src = project_io::load_animation(path).map_err(|e| e.to_string())?;
                check_cancel(cancel)?;
                before.place_external_clip(
                    &src,
                    clip_id.unwrap_or(src.clips[0].id),
                    name,
                    (clip, frame, layer),
                    point,
                )?
            }
        }
        "svg" => {
            let mut drawing = project_io::import_svg(path).map_err(|e| e.to_string())?;
            check_cancel(cancel)?;
            drawing.frames = 1;
            let size = [drawing.width as f32, drawing.height as f32];
            let doc = before.clips[clip].drawing_at_mut(frame);
            let id = doc.add_shape(center(size), size, [0; 4], Shape::Rectangle, layer);
            let object = doc.objects.last_mut().unwrap();
            object.embedded_artwork = Some(Arc::new(drawing).into());
            object.name = name.into();
            id
        }
        "png" | "jpeg" | "jpg" => {
            let image = project_io::import_reference_image(path).map_err(|e| e.to_string())?;
            check_cancel(cancel)?;
            let size = [image.width() as f32, image.height() as f32];
            let doc = before.clips[clip].drawing_at_mut(frame);
            let image = doc
                .objects
                .iter()
                .filter_map(|o| o.image.as_ref())
                .find(|old| old.as_ref() == image.as_ref())
                .cloned()
                .unwrap_or(image);
            let id = doc.add_shape(center(size), size, [0; 4], Shape::Rectangle, layer);
            let object = doc.objects.last_mut().unwrap();
            object.image = Some(image);
            object.name = name.into();
            id
        }
        _ => return Err("Choose a Phyro project, SVG, PNG or JPEG.".into()),
    };
    Ok(id)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled.".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
