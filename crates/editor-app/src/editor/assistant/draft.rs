//! The session retains the base, latest version and a replayable validated step log.
use super::{review::render, Editor, Record, Session};
use document_core::AnimationProject;
use eframe::egui;
use phyro_automation::{
    edit, ipc,
    live::{error, Reply},
};
use serde_json::{json, Value};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

const LIFETIME: Duration = Duration::from_secs(600);
type Project = Arc<AnimationProject>;

#[derive(PartialEq)]
pub(super) struct Step {
    pub label: String,
    pub operations: Value,
}
pub(super) struct Entry {
    input: Arc<Step>,
    ack: Value,
}
pub(super) enum Append {
    Answer(Value),
    Validate(Project),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Frame {
    tab: u64,
    revision: u64,
    proposal: String,
    step: usize,
    clip: u64,
    tick: u32,
    longest: u32,
}
struct Version<T> {
    key: Frame,
    project: Project,
    images: [T; 2],
}

pub(super) struct Draft {
    pub id: String,
    pub revision: u64,
    pub deadline: Instant,
    pub steps: Vec<Entry>,
    tab: u64,
    base: Project,
    latest: Project,
    bytes: usize,
    count: usize,
    sealed: bool,
    pinned: Option<usize>,
    running: Option<mpsc::Receiver<(Frame, Reply<Version<egui::ColorImage>>)>>,
    shown: Option<Version<egui::TextureHandle>>,
    failed: Option<(Frame, Value)>,
}

pub(super) enum Decision {
    Wait,
    Keep(Frame),
    Discard,
}

fn conflict(expected: usize) -> Value {
    let mut result = error("step_conflict", format!("Expected step {expected}."));
    result["expected_next_step"] = json!(expected);
    result
}

impl Session {
    pub(super) fn begin(
        &mut self,
        editor: &Editor,
        document: String,
        revision: u64,
        request_id: String,
    ) -> Reply {
        if document != self.handle {
            return Err(error("wrong_document", "Inspect the shared tab again."));
        }
        if request_id.is_empty() || request_id.len() > 128 {
            return Err(error(
                "invalid_request",
                "Supply a request ID of 1–128 bytes.",
            ));
        }
        if let Some(record) = self.records.values().find(|r| r.request_id == request_id) {
            if record.status.get("latest_step").is_some()
                && record.status["expected_revision"] == revision
            {
                return Ok(record.status.clone());
            }
            return Err(error(
                "request_conflict",
                "This request ID belongs to another proposal.",
            ));
        }
        if revision != editor.history.revision {
            return Err(error("stale", "Inspect the current revision."));
        }
        if self.pending.is_some() || self.draft.is_some() {
            return Err(error("busy", "A review is already open."));
        }
        if self.records.len() >= 256 {
            return Err(error(
                "session_full",
                "Pair again to start another session.",
            ));
        }
        let id = ipc::token().map_err(|e| error("proposal_failed", e))?;
        let base = Arc::new(editor.baseline.clone());
        let status = json!({"proposal_id":id,"state":"drawing","latest_step":0,"expected_revision":revision});
        self.records.insert(
            id.clone(),
            Record {
                request_id,
                fingerprint: 0,
                status: status.clone(),
            },
        );
        self.draft = Some(Draft {
            id,
            tab: self.id,
            revision,
            deadline: Instant::now() + LIFETIME,
            steps: vec![],
            latest: base.clone(),
            base,
            bytes: 0,
            count: 0,
            sealed: false,
            pinned: None,
            running: None,
            shown: None,
            failed: None,
        });
        Ok(status)
    }

    pub(super) fn closed_draft(&self, id: &str) -> Reply<Option<Value>> {
        let record = self
            .records
            .get(id)
            .filter(|r| r.status.get("latest_step").is_some())
            .ok_or_else(|| error("unknown_proposal", "Unknown draft in this session."))?;
        Ok((!self.draft.as_ref().is_some_and(|d| d.id == id)).then(|| record.status.clone()))
    }

    pub(super) fn stage(
        &self,
        id: &str,
        number: usize,
        step: &Step,
        bytes: usize,
    ) -> Reply<Append> {
        if let Some(status) = self.closed_draft(id)? {
            return Ok(Append::Answer(status));
        }
        let d = self.draft.as_ref().unwrap();
        let next = d.steps.len() + 1;
        if let Some(prior) = number.checked_sub(1).and_then(|i| d.steps.get(i)) {
            return if *prior.input == *step {
                Ok(Append::Answer(prior.ack.clone()))
            } else {
                Err(conflict(next))
            };
        }
        if number != next {
            return Err(conflict(next));
        }
        if d.sealed {
            return Ok(Append::Answer(self.records[id].status.clone()));
        }
        let ops = step
            .operations
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| error("invalid_batch", "Supply at least one operation."))?;
        if step.label.trim().is_empty() {
            return Err(error("invalid_label", "Name the visual change."));
        }
        if d.count + ops.len() > 128 || d.bytes + bytes > ipc::MAX_REQUEST {
            return Err(error(
                "draft_limit",
                "A draft holds up to 128 operations and one MiB of step payload.",
            ));
        }
        Ok(Append::Validate(d.latest.clone()))
    }

    pub(super) fn accept_step(
        &mut self,
        id: &str,
        number: usize,
        input: Arc<Step>,
        after: Project,
        results: Value,
        bytes: usize,
    ) -> Reply {
        let d = self.draft.as_mut().unwrap();
        if d.id != id || d.steps.len() + 1 != number || d.sealed {
            return Err(conflict(d.steps.len() + 1));
        }
        let ack = json!({"proposal_id":id,"state":"drawing","step":number,"operations":results});
        d.latest = after;
        d.count += input.operations.as_array().unwrap().len();
        d.bytes += bytes;
        d.steps.push(Entry {
            input,
            ack: ack.clone(),
        });
        d.deadline = Instant::now() + LIFETIME;
        self.records.get_mut(id).unwrap().status["latest_step"] = json!(number);
        Ok(ack)
    }

    pub(super) fn seal(&mut self, id: &str, last: usize) -> Reply {
        if let Some(status) = self.closed_draft(id)? {
            return Ok(status);
        }
        let d = self.draft.as_mut().unwrap();
        let status = &mut self.records.get_mut(id).unwrap().status;
        if !d.sealed {
            if last == 0 || last != d.steps.len() {
                return Err(conflict(d.steps.len() + 1));
            }
            d.sealed = true;
            d.deadline = Instant::now() + LIFETIME;
            status["state"] = json!("ready");
        }
        Ok(status.clone())
    }

    pub(super) fn keep(
        &mut self,
        editor: &mut Editor,
        ctx: &egui::Context,
        key: Frame,
    ) -> Reply<()> {
        self.refresh(editor);
        self.access(editor, ctx)?;
        let d = self
            .draft
            .as_ref()
            .filter(|d| {
                d.id == key.proposal
                    && key.tab == self.id
                    && key.revision == editor.history.revision
            })
            .ok_or_else(|| error("stale", "The displayed draft closed."))?;
        let shown = d
            .shown
            .as_ref()
            .filter(|s| s.key == key)
            .ok_or_else(|| error("stale", "Review the displayed step."))?;
        if shown.project != d.base {
            editor.commit_assistant_project(&shown.project);
        }
        let status = &mut self.records.get_mut(&d.id).unwrap().status;
        status["kept_step"] = json!(key.step);
        status["committed_revision"] = json!(editor.history.revision);
        status["operations"] = Value::Array(
            d.steps[..key.step]
                .iter()
                .enumerate()
                .flat_map(|(index, entry)| {
                    entry.ack["operations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned()
                        .map(move |mut result| {
                            result["step"] = json!(index + 1);
                            result
                        })
                })
                .collect(),
        );
        self.finish("applied", None);
        ctx.request_repaint();
        Ok(())
    }
}

impl Draft {
    fn frame(&self, step: usize, longest: u32) -> Frame {
        let op = &self.steps[step - 1].input.operations[0];
        Frame {
            tab: self.tab,
            revision: self.revision,
            proposal: self.id.clone(),
            step,
            longest,
            clip: op["clip_id"].as_u64().unwrap(),
            tick: op
                .get("frame")
                .unwrap_or(&op["exposure_start"])
                .as_u64()
                .unwrap() as u32,
        }
    }

    fn preview(&mut self, ctx: &egui::Context, target: Frame) {
        // Freeze the visible version across the press/release that chooses Keep.
        if ctx.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
            ctx.request_repaint();
            return;
        }
        if let Some(job) = &self.running {
            match job.try_recv() {
                Ok((key, result)) => {
                    self.running = None;
                    let advances = self.pinned.is_none()
                        && self.shown.as_ref().is_none_or(|v| v.key.step < key.step);
                    if advances || key == target {
                        match result {
                            Ok(version) => {
                                self.failed = None;
                                self.shown = Some(Version {
                                    images: version.images.map(|img| {
                                        ctx.load_texture(
                                            "draft preview",
                                            img,
                                            egui::TextureOptions::LINEAR,
                                        )
                                    }),
                                    project: version.project,
                                    key,
                                });
                            }
                            Err(e) => self.failed = Some((key, e)),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.running = None;
                    self.failed = Some((
                        target.clone(),
                        error("render_failed", "Preview worker stopped."),
                    ));
                }
            }
        }
        if self.shown.as_ref().is_some_and(|v| v.key == target)
            || self.failed.as_ref().is_some_and(|(k, _)| *k == target)
        {
            return;
        }
        let base = self.base.clone();
        let latest = (target.step == self.steps.len()).then(|| self.latest.clone());
        let replay: Vec<_> = if latest.is_none() {
            self.steps[..target.step]
                .iter()
                .map(|e| e.input.clone())
                .collect()
        } else {
            vec![]
        };
        let (tx, rx) = mpsc::sync_channel(1);
        self.running = Some(rx);
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let project = if let Some(p) = latest {
                    p
                } else {
                    let mut p = (*base).clone();
                    for s in replay {
                        edit::apply(&mut p, &s.operations).map_err(|e| json!(e))?;
                    }
                    Arc::new(p)
                };
                let image = |p: &AnimationProject| {
                    render(p, target.clip, target.tick, target.longest).map(|pix| {
                        egui::ColorImage::from_rgba_premultiplied(
                            [pix.width() as usize, pix.height() as usize],
                            pix.data(),
                        )
                    })
                };
                let images = [image(&base)?, image(&project)?];
                Ok(Version {
                    key: target.clone(),
                    project,
                    images,
                })
            })();
            let _ = tx.send((target, result));
            wake.request_repaint();
        });
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) -> Decision {
        ui.separator();
        let discard = ui
            .horizontal(|ui| {
                let clicked = ui.button("Discard draft").clicked();
                ui.heading(if self.sealed {
                    "Review finished drawing"
                } else {
                    "Live drawing"
                });
                clicked
            })
            .inner;
        if discard {
            return Decision::Discard;
        }
        let latest = self.steps.len();
        if latest > 0 {
            let selected = self.pinned.unwrap_or(latest);
            egui::ComboBox::from_label("Drawing step")
                .selected_text(format!(
                    "Step {selected} · {}",
                    self.steps[selected - 1].input.label
                ))
                .show_ui(ui, |ui| {
                    for (i, e) in self.steps.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.pinned,
                            Some(i + 1),
                            format!("Step {} · {}", i + 1, e.input.label),
                        );
                    }
                });
            if ui
                .selectable_label(self.pinned.is_none(), "Follow latest")
                .clicked()
            {
                self.pinned = None;
            }
            let column_width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.;
            let longest = (column_width.max(240.) * ctx.pixels_per_point())
                .ceil()
                .clamp(1., 1024.) as u32;
            self.preview(ctx, self.frame(self.pinned.unwrap_or(latest), longest));
            if let Some((_, err)) = &self.failed {
                ui.label(err["message"].as_str().unwrap_or("Preview failed."));
            }
            if let Some(v) = &self.shown {
                let step = v.key.step;
                let clip = &v.project.clips[v.project.clip_index(v.key.clip).unwrap()];
                ui.label(format!(
                    "Step {step} · {} · {} · frame {}",
                    self.steps[step - 1].input.label,
                    clip.name,
                    v.key.tick + 1
                ));
                ui.columns(2, |cols| {
                    for ((ui, img), name) in
                        cols.iter_mut().zip(&v.images).zip(["Original", "Proposed"])
                    {
                        ui.label(name);
                        let size = img.size_vec2();
                        ui.image((
                            img.id(),
                            size * (ui.available_width() / size.x).min(240. / size.y),
                        ));
                    }
                });
                let keep = ui
                    .horizontal(|ui| {
                        let click = ui
                            .add_enabled(
                                !busy && self.pinned.is_none_or(|n| n == step),
                                egui::Button::new(format!("Keep through step {step}")),
                            )
                            .clicked();
                        if latest > step {
                            ui.label(format!("{} later step(s) will be discarded", latest - step));
                        }
                        click
                    })
                    .inner;
                if keep {
                    return Decision::Keep(v.key.clone());
                }
            } else {
                ui.spinner();
            }
        } else {
            ui.label("Waiting for the first step.");
        }
        Decision::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[test]
    fn draft_limits_and_expiry_preserve_the_last_valid_acknowledgement() {
        let editor = Editor::new(&egui::Context::default());
        let mut session = Session {
            id: editor.document_id,
            handle: "shared".into(),
            name: "Untitled".into(),
            events: mpsc::channel().1,
            ended: oneshot::channel().1,
            _stop: oneshot::channel().0,
            records: Default::default(),
            pending: None,
            draft: None,
            decision: None,
        };
        let status = session
            .begin(
                &editor,
                "shared".into(),
                editor.history.revision,
                "lantern".into(),
            )
            .unwrap();
        let id = status["proposal_id"].as_str().unwrap();
        let step = Arc::new(Step {
            label: "Base".into(),
            operations: json!([{"op":"create_path"}]),
        });
        let Append::Validate(project) = session.stage(id, 1, &step, ipc::MAX_REQUEST - 1).unwrap()
        else {
            panic!("expected a fresh step")
        };
        let ack = session
            .accept_step(
                id,
                1,
                step.clone(),
                project,
                json!([]),
                ipc::MAX_REQUEST - 1,
            )
            .unwrap();
        assert_eq!(
            session.stage(id, 2, &step, 2).err().unwrap()["code"],
            "draft_limit"
        );
        let many = Step {
            label: "Parts".into(),
            operations: Value::Array(vec![json!({}); 128]),
        };
        assert_eq!(
            session.stage(id, 2, &many, 1).err().unwrap()["code"],
            "draft_limit"
        );
        let Append::Answer(retry) = session.stage(id, 1, &step, ipc::MAX_REQUEST - 1).unwrap()
        else {
            panic!("expected original acknowledgement")
        };
        assert_eq!(retry, ack);
        session.draft.as_mut().unwrap().deadline = Instant::now();
        session.refresh(&editor);
        let expired = session.seal(id, 1).unwrap();
        assert_eq!(expired["state"], "expired");
        assert_eq!(expired["latest_step"], 1);
        assert!(session.draft.is_none());
    }
}
