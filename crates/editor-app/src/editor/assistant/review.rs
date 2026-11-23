use document_core::{AnimationProject, Clip, VectorObject};
#[cfg(test)]
mod tests;
use eframe::egui;
use phyro_automation::{
    edit, inspect, ipc,
    live::{error, Reply},
};
use serde_json::{json, Value};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

pub(super) fn render(
    project: &AnimationProject,
    clip_id: u64,
    tick: u32,
    longest: u32,
) -> Reply<renderer::Pixmap> {
    let clip = inspect::clip_by_id(project, clip_id).map_err(|e| json!(e))?;
    render_clip(clip, tick, longest)
}

pub(super) fn render_clip(clip: &Clip, tick: u32, longest: u32) -> Reply<renderer::Pixmap> {
    if tick >= clip.frames {
        return Err(error(
            "frame_out_of_range",
            "Choose a tick inside the clip.",
        ));
    }
    let drawing = clip.drawing_at(tick);
    let scale = (longest as f32 / drawing.width.max(drawing.height) as f32).min(1.);
    renderer::render_scaled(drawing, tick, scale).map_err(|e| error("render_failed", e))
}

struct Hold {
    clip: u64,
    name: String,
    first: u32,
    last: u32,
}
type Frame = (u64, u32);
type Images = [egui::ColorImage; 2];

#[derive(Default)]
struct Preview {
    running: Option<mpsc::Receiver<(Frame, Reply<Images>)>>,
    shown: Option<(Frame, Reply<[egui::TextureHandle; 2]>)>,
}

pub(super) struct Review {
    pub id: String,
    pub revision: u64,
    pub results: Value,
    pub deadline: Instant,
    pub changed: bool,
    before: Arc<AnimationProject>,
    pub after: Arc<AnimationProject>,
    holds: Vec<Hold>,
    descriptions: Vec<String>,
    hold: usize,
    tick: u32,
    preview: Preview,
}

pub(super) enum Decision {
    Apply,
    Reject,
    Wait,
}

fn change(
    input: &Value,
    original: Option<&VectorObject>,
    revised: Option<&VectorObject>,
    tick: u32,
) -> String {
    match input["op"].as_str().unwrap_or_default() {
        "set_fill" => {
            let from = original
                .map(|o| {
                    if o.no_fill {
                        "none".into()
                    } else {
                        format!("{:?}", o.fill)
                    }
                })
                .unwrap_or_default();
            format!("Fill {from} → {} throughout the layer hold", input["fill"])
        }
        "set_position_key" => {
            let from = original
                .map(|o| format!("{:?}", o.position_at(tick)))
                .unwrap_or_default();
            let mut label = format!(
                "Position key at frame {} · {from} → {} px",
                tick + 1,
                input["position"]
            );
            if let Some((o, previous)) =
                revised.and_then(|o| o.keys.range(..tick).next_back().map(|(&key, _)| (o, key)))
            {
                let easing = document_core::Property::Position.easing_at(o, previous);
                label.push_str(&format!(
                    " · {} interpolation from frame {} to {}",
                    easing.label(),
                    previous + 1,
                    tick + 1
                ));
            }
            label
        }
        "align" => {
            let positions = original.zip(revised).map_or(String::new(), |(a, b)| {
                format!(
                    " · {:?} → {:?} px",
                    a.position_at(tick),
                    b.position_at(tick)
                )
            });
            format!(
                "Align {} at frame {}{positions}",
                input["direction"].as_str().unwrap(),
                tick + 1
            )
        }
        "boolean" => format!("{} at frame {}", input["operation"], tick + 1),
        "create_path" => format!(
            "Create path with {} nodes and fill {}",
            input["nodes"].as_array().map_or(0, Vec::len),
            input["fill"]
        ),
        "create_text" => format!("Create text {}", input["content"]),
        "set_text" => format!(
            "Set text to {}{}",
            input["content"],
            input
                .get("font_size")
                .map_or(String::new(), |s| format!(" at {s} px"))
        ),
        _ => unreachable!("validated operation"),
    }
}

fn object(clip: &Clip, tick: u32, id: u64) -> Option<&VectorObject> {
    clip.drawing_at(tick).objects.iter().find(|o| o.id == id)
}

impl Review {
    pub(super) fn prepare(
        before: Arc<AnimationProject>,
        revision: u64,
        operations: Value,
    ) -> Reply<Self> {
        let mut after = (*before).clone();
        let results = edit::apply(&mut after, &operations).map_err(|e| json!(e))?;
        let mut holds: Vec<Hold> = Vec::new();
        let mut descriptions = Vec::new();
        let mut preferred = None;
        for (input, result) in operations
            .as_array()
            .unwrap()
            .iter()
            .zip(results.as_array().unwrap())
        {
            let clip_id = result["clip_id"].as_u64().unwrap();
            let start = result["exposure_start"].as_u64().unwrap() as u32;
            let tick = input["frame"].as_u64().unwrap_or(u64::from(start)) as u32;
            let clip = &after.clips[after.clip_index(clip_id).unwrap()];
            let old = &before.clips[before.clip_index(clip_id).unwrap()];
            for id in result["affected_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
            {
                let original = object(old, tick, id);
                let revised = object(clip, tick, id);
                let obj = revised.or(original);
                let name = obj.map_or("Object", |o| o.name.as_str());
                let (first, duration) =
                    obj.map_or(clip.exposure(tick), |o| clip.layer_exposure(tick, o.layer));
                let layer_name = obj
                    .and_then(|o| {
                        clip.drawing_at(tick)
                            .layers
                            .iter()
                            .find(|l| l.id == o.layer)
                    })
                    .map_or("Layer", |l| l.name.as_str());
                descriptions.push(format!(
                    "{name} (#{id}) · {}\n{layer_name} · frames {}–{}",
                    change(input, original, revised, tick),
                    first + 1,
                    first + duration
                ));
                if !holds.iter().any(|h| {
                    h.clip == clip_id && h.first == first && h.last == first + duration - 1
                }) {
                    holds.push(Hold {
                        clip: clip_id,
                        name: clip.name.clone(),
                        first,
                        last: first + duration - 1,
                    });
                }
            }
            if input.get("frame").is_some() {
                preferred.get_or_insert((clip_id, tick));
            }
            if holds.is_empty() {
                holds.push(Hold {
                    clip: clip_id,
                    name: clip.name.clone(),
                    first: start,
                    last: start + result["exposure_duration"].as_u64().unwrap() as u32 - 1,
                });
            }
        }
        let (clip, tick) = preferred.unwrap_or((holds[0].clip, holds[0].first));
        let hold = holds
            .iter()
            .position(|h| h.clip == clip && (h.first..=h.last).contains(&tick))
            .unwrap_or(0);
        let changed = *before != after;
        Ok(Self {
            id: ipc::token().map_err(|e| error("proposal_failed", e))?,
            revision,
            deadline: Instant::now() + Duration::from_secs(600),
            results,
            before,
            after: Arc::new(after),
            holds,
            descriptions,
            hold,
            tick,
            changed,
            preview: Default::default(),
        })
    }

    fn preview(&mut self, ctx: &egui::Context, frame: Frame) {
        if let Some(job) = &self.preview.running {
            match job.try_recv() {
                Ok((rendered, result)) => {
                    self.preview.running = None;
                    if rendered == frame {
                        let textures = result.map(|imgs| {
                            imgs.map(|img| {
                                ctx.load_texture(
                                    "assistant frame",
                                    img,
                                    egui::TextureOptions::LINEAR,
                                )
                            })
                        });
                        self.preview.shown = Some((frame, textures));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.preview.running = None;
                    self.preview.shown = Some((
                        frame,
                        Err(error("render_failed", "Preview worker stopped.")),
                    ));
                }
            }
        }
        if self
            .preview
            .shown
            .as_ref()
            .is_some_and(|(key, _)| *key == frame)
        {
            return;
        }
        self.preview.shown = None;
        let (tx, rx) = mpsc::sync_channel(1);
        self.preview.running = Some(rx);
        let projects = [self.before.clone(), self.after.clone()];
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let [before, after] = projects.map(|p| {
                render(&p, frame.0, frame.1, 1024).map(|pix| {
                    egui::ColorImage::from_rgba_premultiplied(
                        [pix.width() as usize, pix.height() as usize],
                        pix.data(),
                    )
                })
            });
            let _ = tx.send((frame, before.and_then(|b| after.map(|a| [b, a]))));
            wake.request_repaint();
        });
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) -> Decision {
        ui.separator();
        if ui
            .horizontal(|ui| {
                ui.heading("Review proposed edits");
                ui.button("Reject").clicked()
            })
            .inner
        {
            return Decision::Reject;
        }
        egui::ScrollArea::vertical()
            .max_height(120.)
            .show(ui, |ui| {
                for line in &self.descriptions {
                    ui.label(line);
                }
            });
        let label = |h: &Hold| format!("{} · frames {}–{}", h.name, h.first + 1, h.last + 1);
        egui::ComboBox::from_label("Layer hold")
            .selected_text(label(&self.holds[self.hold]))
            .show_ui(ui, |ui| {
                for (index, h) in self.holds.iter().enumerate() {
                    if ui
                        .selectable_value(&mut self.hold, index, label(h))
                        .changed()
                    {
                        self.tick = h.first;
                    }
                }
            });
        let h = &self.holds[self.hold];
        let mut number = self.tick + 1;
        ui.add(egui::Slider::new(&mut number, h.first + 1..=h.last + 1).text("Preview frame"));
        self.tick = number - 1;
        let frame = (h.clip, self.tick);
        self.preview(ctx, frame);
        let mut ready = false;
        match &self.preview.shown {
            Some((key, Ok(images))) if *key == frame => {
                ready = true;
                ui.columns(2, |columns| {
                    for ((ui, texture), label) in
                        columns.iter_mut().zip(images).zip(["Original", "Proposed"])
                    {
                        ui.label(label);
                        let size = texture.size_vec2();
                        let scale = (ui.available_width() / size.x).min(240. / size.y);
                        ui.image((texture.id(), size * scale));
                    }
                });
            }
            Some((_, Err(e))) => {
                ui.label(e["message"].as_str().unwrap_or("Preview failed."));
            }
            _ => {
                ui.spinner();
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(ready && !busy, egui::Button::new("Apply"))
                .clicked()
            {
                Decision::Apply
            } else {
                Decision::Wait
            }
        })
        .inner
    }
}
