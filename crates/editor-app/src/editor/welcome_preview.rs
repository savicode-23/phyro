use eframe::egui::{self, Color32, ColorImage, FontId, Rect, TextureHandle, Vec2};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

const EDGE: u32 = 480;
const SELECTION_DELAY: Duration = Duration::from_millis(90);

struct Poster {
    image: ColorImage,
    caption: String,
}

struct Job {
    cancelled: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Poster, String>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

enum Display {
    Waiting,
    Ready(TextureHandle, String),
    Failed(String),
}

/// At most one decode is in flight. Rapid changes replace the pending path,
/// and entering the editor releases the texture and cancels remaining work.
#[derive(Default)]
pub(super) struct Preview {
    selected: Option<PathBuf>,
    due: Option<Instant>,
    job: Option<Job>,
    display: Option<Display>,
    opening: bool,
}

impl Preview {
    pub(super) fn clear_selection(&mut self, ctx: &egui::Context) {
        self.update(ctx, None, Instant::now());
    }

    pub(super) fn set_opening(&mut self, opening: bool) {
        self.opening = opening;
        if opening {
            if let Some(job) = &self.job {
                job.cancelled.store(true, Ordering::Relaxed);
                self.due = Some(Instant::now());
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, path: Option<&Path>, now: Instant) {
        if self.selected.as_deref() != path {
            if let Some(job) = &self.job {
                job.cancelled.store(true, Ordering::Relaxed);
            }
            self.selected = path.map(Path::to_owned);
            self.due = path.map(|_| now + SELECTION_DELAY);
            self.display = path.map(|_| Display::Waiting);
        }
        if let Some(job) = &self.job {
            match job.result.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                result => {
                    if !job.cancelled.load(Ordering::Relaxed) {
                        let result = result.unwrap_or_else(|_| Err("Preview task stopped.".into()));
                        self.display = Some(match result {
                            Ok(poster) => Display::Ready(
                                ctx.load_texture(
                                    "welcome-project-preview",
                                    poster.image,
                                    egui::TextureOptions::LINEAR,
                                ),
                                poster.caption,
                            ),
                            Err(error) => Display::Failed(error),
                        });
                    }
                    self.job = None;
                }
            }
        }
        if self.opening {
            return;
        }
        let (Some(due), Some(path)) = (self.due, &self.selected) else {
            return;
        };
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        self.due = None;
        let path = path.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let (sender, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        match std::thread::Builder::new()
            .name("recent project preview".into())
            .spawn(move || {
                if sender.send(render_poster(&path, &cancel)).is_ok() {
                    wake.request_repaint();
                }
            }) {
            Ok(_) => self.job = Some(Job { cancelled, result }),
            Err(error) => self.display = Some(Display::Failed(error.to_string())),
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, path: Option<&Path>, size: Vec2) {
        self.update(ui.ctx(), path, Instant::now());
        let (area, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(area.intersect(ui.clip_rect()));
        painter.rect_filled(area, 6., Color32::from_rgb(22, 30, 32));
        let muted = Color32::from_rgb(164, 184, 179);
        if let Some(Display::Ready(image, caption)) = &self.display {
            let mut artwork = area.shrink(8.);
            artwork.max.y -= 20.;
            if artwork.is_positive() {
                let original = image.size_vec2();
                let ratio = (artwork.width() / original.x).min(artwork.height() / original.y);
                let rect = Rect::from_center_size(artwork.center(), original * ratio);
                painter.rect_filled(rect, 0., Color32::from_rgb(177, 181, 183));
                painter.image(
                    image.id(),
                    rect,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    Color32::WHITE,
                );
            }
            let text = egui::WidgetText::from(egui::RichText::new(caption).size(12.).color(muted))
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    (area.width() - 16.).max(1.),
                    egui::TextStyle::Small,
                );
            painter.galley(
                area.left_bottom() + egui::vec2(8., -text.size().y - 5.),
                text,
                muted,
            );
            return;
        }
        let label = match &self.display {
            Some(Display::Waiting) => "Loading preview…",
            Some(Display::Failed(error)) => {
                response.on_hover_text(error);
                "Preview unavailable"
            }
            _ => "Select a recent project to preview",
        };
        painter.text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(13.),
            muted,
        );
    }
}

fn render_poster(path: &Path, cancelled: &AtomicBool) -> Result<Poster, String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Preview cancelled.".into());
    }
    let project = project_io::load_animation(path).map_err(|error| error.to_string())?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("Preview cancelled.".into());
    }
    let clip = project
        .clips
        .first()
        .ok_or("There are no clips to preview.")?;
    // Skip empty lead-in exposures without rasterizing every animation frame.
    let frame = clip
        .cels
        .iter()
        .find(|(_, doc)| !doc.objects.is_empty())
        .map_or(0, |(&frame, _)| frame);
    let doc = clip.drawing_at(frame);
    let scale = EDGE as f32 / doc.width.max(doc.height).max(1) as f32;
    let pixmap = renderer::render_scaled(doc, frame, scale)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("Preview cancelled.".into());
    }
    Ok(Poster {
        image: ColorImage::from_rgba_premultiplied(
            [pixmap.width() as usize, pixmap.height() as usize],
            pixmap.data(),
        ),
        caption: format!(
            "{} · Frame {} · {} × {}",
            clip.name,
            frame + 1,
            doc.width,
            doc.height
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::{AnimationProject, Document};

    #[test]
    fn poster_uses_the_first_drawn_exposure_and_fits_wide_artboards() {
        let mut project = AnimationProject::from_document(Document {
            width: 1000,
            height: 100,
            ..Default::default()
        });
        let clip = &mut project.clips[0];
        clip.name = "Run".into();
        clip.set_length(999).unwrap();
        let frame = clip.blank_frame(236).unwrap();
        clip.drawing_at_mut(frame)
            .add_rectangle([0., 0.], [500., 100.], [20, 160, 80, 255]);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Hero.vanim");
        project_io::save_animation(&file, &project, false).unwrap();
        let poster = render_poster(&file, &AtomicBool::new(false)).unwrap();
        assert_eq!(poster.caption, "Run · Frame 238 · 1000 × 100");
        assert_eq!(poster.image.size, [480, 48]);
        assert_eq!(poster.image[(50, 24)], Color32::from_rgb(20, 160, 80));
        assert_eq!(poster.image[(460, 24)], Color32::TRANSPARENT);
    }

    #[test]
    fn changing_selection_drops_outdated_results_and_coalesces_pending_work() {
        let ctx = egui::Context::default();
        let mut preview = Preview::default();
        let start = Instant::now();
        let first = Path::new("one.vanim");
        let second = Path::new("two.vanim");
        let last = Path::new("three.vanim");
        preview.update(&ctx, Some(first), start);
        // Hold the first completion while the selection changes twice.
        let (sender, result) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        preview.job = Some(Job {
            cancelled: cancelled.clone(),
            result,
        });
        preview.update(&ctx, Some(second), start);
        preview.update(&ctx, Some(last), start);
        assert!(cancelled.load(Ordering::Relaxed));
        sender.send(Err("Obsolete preview error".into())).unwrap();
        preview.update(&ctx, Some(last), start);
        assert!(matches!(preview.display, Some(Display::Waiting)));
        assert!(preview.job.is_none());
        assert_eq!(preview.selected.as_deref(), Some(last));
        assert_eq!(preview.due, Some(start + SELECTION_DELAY));
    }
}
