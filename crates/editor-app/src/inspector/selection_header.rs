use document_core::Document;
use eframe::egui::{self, Color32, TextureHandle};
use std::sync::{mpsc, Arc, Mutex, Weak};

struct Request {
    drawing: Arc<Document>,
    index: usize,
    id: u64,
    frame: u32,
    edge: u32,
    context: egui::Context,
}

struct Preview {
    drawing: Weak<Document>,
    id: u64,
    frame: Option<u32>,
    edge: u32,
    image: Option<egui::ColorImage>,
}

struct Worker {
    requests: mpsc::SyncSender<Request>,
    results: mpsc::Receiver<Preview>,
}

impl Worker {
    fn new() -> Option<Self> {
        let (requests, jobs) = mpsc::sync_channel::<Request>(1);
        let (done, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("selection-preview".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let object = job
                        .drawing
                        .objects
                        .get(job.index)
                        .filter(|o| o.id == job.id);
                    let image = object
                        .and_then(|o| renderer::object_thumbnail(o, job.frame, job.edge))
                        .map(|p| {
                            egui::ColorImage::from_rgba_premultiplied(
                                [job.edge as usize; 2],
                                p.data(),
                            )
                        });
                    let preview = Preview {
                        drawing: Arc::downgrade(&job.drawing),
                        id: job.id,
                        frame: object.filter(|o| !o.frame_independent()).map(|_| job.frame),
                        edge: job.edge,
                        image,
                    };
                    if done.send(preview).is_err() {
                        break;
                    }
                    job.context.request_repaint();
                }
            })
            .ok()?;
        Some(Self { requests, results })
    }
}

#[derive(Default)]
struct Cache {
    worker: Option<Worker>,
    busy: bool,
    shown: Option<(Preview, Option<TextureHandle>)>,
}

impl Cache {
    fn texture(
        &mut self,
        ui: &egui::Ui,
        drawing: &Arc<Document>,
        index: usize,
        frame: u32,
        edge: u32,
    ) -> Option<egui::TextureId> {
        if self.worker.is_none() {
            self.worker = Worker::new();
        }
        let worker = self.worker.as_ref()?;
        if self.busy {
            match worker.results.try_recv() {
                Ok(mut ready) => {
                    let texture = ready.image.take().map(|im| {
                        ui.ctx()
                            .load_texture("Selected artwork", im, egui::TextureOptions::LINEAR)
                    });
                    self.shown = Some((ready, texture));
                    self.busy = false;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.busy = false;
                    return None;
                }
            }
        }
        let id = drawing.objects[index].id;
        if let Some((ready, texture)) = &self.shown {
            if ready.drawing.as_ptr() == Arc::as_ptr(drawing)
                && ready.id == id
                && ready.edge == edge
                && ready.frame.is_none_or(|tick| tick == frame)
            {
                return texture.as_ref().map(TextureHandle::id);
            }
        }
        if !self.busy {
            self.busy = worker
                .requests
                .try_send(Request {
                    drawing: drawing.clone(),
                    index,
                    id,
                    frame,
                    edge,
                    context: ui.ctx().clone(),
                })
                .is_ok();
        }
        // The prior pose can remain visible while a new animation frame is rasterized.
        self.shown
            .as_ref()
            .filter(|(p, _)| p.id == id && p.drawing.as_ptr() == Arc::as_ptr(drawing))
            .and_then(|(_, texture)| texture.as_ref().map(TextureHandle::id))
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    snapshot: Option<&Arc<Document>>,
    selected: &[u64],
    frame: u32,
) {
    if selected.is_empty() {
        return;
    }
    if let [id] = selected {
        let Some(index) = doc.objects.iter().position(|o| o.id == *id) else {
            return;
        };
        let object = &doc.objects[index];
        ui.horizontal(|ui| {
            let side = if ui.available_width() < 205. {
                38.
            } else {
                46.
            };
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 3., ui.visuals().extreme_bg_color);
            if let Some(drawing) =
                snapshot.filter(|d| d.objects.get(index).is_some_and(|o| o.id == *id))
            {
                let key = ui.id().with("artwork-thumbnail");
                let cache = ui.data_mut(|data| {
                    data.get_temp_mut_or_default::<Arc<Mutex<Cache>>>(key)
                        .clone()
                });
                let edge = (side * ui.ctx().pixels_per_point()).round().clamp(8., 128.) as u32;
                let texture = cache
                    .lock()
                    .ok()
                    .and_then(|mut c| c.texture(ui, drawing, index, frame, edge));
                if let Some(texture) = texture {
                    ui.painter().image(
                        texture,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                }
            }
            ui.vertical(|ui| {
                ui.add_space(3.);
                let name = object.display_name();
                ui.add(egui::Label::new(egui::RichText::new(&name).strong().size(15.)).truncate())
                    .on_hover_text(name);
                let kind = object.kind_name();
                let layer = doc.layers.iter().find(|l| l.id == object.layer);
                let detail = layer.map_or_else(|| kind.into(), |l| format!("{kind} · {}", l.name));
                ui.add(egui::Label::new(egui::RichText::new(&detail).small().weak()).truncate())
                    .on_hover_text(detail);
            });
        });
    } else {
        ui.strong(format!("{} objects selected", selected.len()));
    }
    ui.separator();
}
