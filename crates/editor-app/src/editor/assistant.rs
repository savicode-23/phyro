//! The editor owns consent, the active document and the sole publish path.
mod draft;
mod review;
mod worker;

use super::Editor;
use eframe::egui;
use phyro_automation::live::{error, Reply};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

pub(super) fn document_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
pub(super) struct Assistant {
    pub(super) open: bool,
    code: String,
    session: Option<Session>,
}

type Event = Box<dyn FnOnce(&Editor, &mut Session, Reply<()>) + Send>;
struct Session {
    id: u64,
    handle: String,
    name: String,
    events: mpsc::Receiver<Event>,
    ended: oneshot::Receiver<std::io::Result<()>>,
    // Dropping this sender cancels even a blocked socket read.
    _stop: oneshot::Sender<()>,
    records: HashMap<String, Record>,
    pending: Option<review::Review>,
    draft: Option<draft::Draft>,
    decision: Option<&'static str>,
}

struct Record {
    request_id: String,
    fingerprint: u64,
    status: Value,
}

impl Assistant {
    pub(super) fn sharing(&self) -> bool {
        self.session.is_some()
    }
}

impl Session {
    fn preview_available(&self) -> bool {
        self.pending.is_some() || self.draft.as_ref().is_some_and(|d| !d.steps.is_empty())
    }

    fn start(editor: &Editor, ctx: &egui::Context, code: String) -> Self {
        let (events, inbox) = mpsc::sync_channel(4);
        let (stop, cancel) = oneshot::channel();
        let (done, ended) = oneshot::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = worker::run(code, events, &wake, cancel);
            let _ = done.send(result);
            wake.request_repaint();
        });
        Self {
            id: editor.document_id,
            handle: format!("{}-{}", std::process::id(), editor.document_id),
            name: editor.name(),
            events: inbox,
            ended,
            _stop: stop,
            records: HashMap::new(),
            pending: None,
            draft: None,
            decision: None,
        }
    }

    fn finish(&mut self, state: &str, revision: Option<u64>) {
        if let Some(draft) = self.draft.take() {
            self.records.get_mut(&draft.id).unwrap().status["state"] = json!(state);
            self.decision = Some(match state {
                "applied" => "Kept the displayed drawing.",
                "expired" => "The live draft expired.",
                "stale" => "Your edit closed the live draft.",
                _ => "Draft discarded.",
            });
        }
        if let Some(review) = self.pending.take() {
            self.decision = Some(match state {
                "applied" => "Applied the reviewed changes.",
                "rejected" => "Proposal rejected.",
                "expired" => "This review expired. Ask your assistant for a new proposal.",
                "stale" => "Your document changed. Ask your assistant to inspect it again.",
                _ => unreachable!("review outcome"),
            });
            let record = &mut self.records.get_mut(&review.id).unwrap().status;
            record["state"] = json!(state);
            if let Some(rev) = revision {
                record["committed_revision"] = json!(rev);
                record["operations"] = review.results;
            }
        }
    }

    fn refresh(&mut self, editor: &Editor) {
        let active = self
            .pending
            .as_ref()
            .map(|r| (r.deadline, r.revision))
            .or_else(|| self.draft.as_ref().map(|d| (d.deadline, d.revision)));
        if let Some((deadline, revision)) = active {
            if Instant::now() >= deadline {
                self.finish("expired", None)
            } else if self.id == editor.document_id && revision != editor.history.revision {
                self.finish("stale", None)
            }
        }
    }

    fn access(&self, editor: &Editor, ctx: &egui::Context) -> Reply<()> {
        if self.id != editor.document_id {
            return Err(error("paused", "Activate the shared tab in Phyro."));
        }
        if editor.assistant_busy(ctx) {
            return Err(error(
                "busy",
                "Finish the current gesture or text edit and retry.",
            ));
        }
        Ok(())
    }

    fn apply(&mut self, editor: &mut Editor, ctx: &egui::Context) -> Reply<()> {
        self.refresh(editor);
        self.access(editor, ctx)?;
        let review = self
            .pending
            .as_ref()
            .ok_or_else(|| error("stale", "Inspect the document again."))?;
        // History must use the actual view at commit, since the artist can scrub during review.
        if review.changed {
            editor.commit_assistant_project(&review.after);
        }
        self.finish("applied", Some(editor.history.revision));
        ctx.request_repaint();
        Ok(())
    }
}

impl Editor {
    fn commit_assistant_project(&mut self, project: &document_core::AnimationProject) {
        self.project = project.clone();
        self.history.record_reviewed(
            &self.baseline,
            &self.project,
            self.timeline.clip,
            self.timeline.frame,
        );
        self.writable_cel = None;
        self.capture_baseline();
        self.tidy_selection();
        self.sync_paint();
        self.canvas.park();
    }

    fn assistant_busy(&self, ctx: &egui::Context) -> bool {
        self.pending_edit
            || self.canvas.has_unfinished_edit()
            || self.text_dialog.is_some()
            || self.library.is_editing()
            || self.guided_morph.is_some()
            || self.artboard_size.is_some()
            || self.folder_assets.is_pending()
            || self.recovery_dialog.is_open()
            || self.keyboard_settings.is_open()
            || self.crash_reports.is_open()
            || ctx.wants_keyboard_input()
            || ctx.input(|i| i.pointer.any_down())
    }

    pub(super) fn revoke_shared_document(&mut self) {
        if self
            .assistant
            .session
            .as_ref()
            .is_some_and(|s| s.id == self.document_id)
        {
            self.assistant = Default::default();
        }
    }

    pub(super) fn assistant_ui(&mut self, ctx: &egui::Context) {
        if !self.assistant.open && !self.assistant.sharing() {
            return;
        }
        let mut assistant = std::mem::take(&mut self.assistant);
        if let Some(session) = &mut assistant.session {
            match session.ended.try_recv() {
                Ok(Ok(())) => {
                    self.status = "Assistant disconnected.".into();
                    return;
                }
                Ok(Err(error)) => {
                    self.report_error("Assistant connection failed", error.to_string());
                    return;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.report_error(
                        "Assistant connection stopped",
                        "Share the tab again to reconnect.",
                    );
                    return;
                }
                Err(oneshot::error::TryRecvError::Empty) => (),
            }
            session.refresh(self);
            if session.id == self.document_id && self.assistant_enabled {
                // Save As can rename the shared document without changing its identity.
                session.name = self.name();
            }
            let had_proposal = session.preview_available();
            while let Ok(event) = session.events.try_recv() {
                let access = session.access(self, ctx);
                event(self, session, access);
            }
            if session.pending.is_some() || session.draft.is_some() {
                assistant.open |= !had_proposal && session.preview_available();
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        let mut stop = false;
        if assistant.open {
            let mut visible = true;
            egui::Window::new("Assistant sharing")
                .open(&mut visible)
                .default_width(490.)
                .show(ctx, |ui| {
                    if ui
                        .checkbox(&mut self.assistant_enabled, "Enable assistant sharing")
                        .changed()
                        && !self.assistant_enabled
                    {
                        assistant.session = None;
                        assistant.code.clear();
                    }
                    if !self.assistant_enabled {
                        return;
                    }
                    if let Some(s) = &mut assistant.session {
                        ui.horizontal(|ui| {
                            ui.label(format!("Shared tab · {}", s.name));
                            if ui.button("Stop sharing").clicked() {
                                stop = true;
                            }
                        });
                        if s.id != self.document_id {
                            ui.label("Sharing is paused while another tab is active.");
                            return;
                        }
                        let busy = self.pending_edit || self.canvas.has_unfinished_edit();
                        if let Some(review) = &mut s.pending {
                            match review.show(ui, ctx, busy) {
                                review::Decision::Apply => {
                                    if let Err(e) = s.apply(self, ctx) {
                                        self.report_error(
                                            "Could not apply assistant edits",
                                            e["message"].as_str().unwrap_or_default(),
                                        );
                                    }
                                }
                                review::Decision::Reject => s.finish("rejected", None),
                                review::Decision::Wait => (),
                            }
                        } else if let Some(draft) = &mut s.draft {
                            match draft.show(ui, ctx, busy) {
                                draft::Decision::Keep(key) => {
                                    if let Err(err) = s.keep(self, ctx, key) {
                                        self.report_error(
                                            "Could not keep assistant draft",
                                            err["message"].as_str().unwrap_or_default(),
                                        );
                                    }
                                }
                                draft::Decision::Discard => s.finish("rejected", None),
                                draft::Decision::Wait => (),
                            }
                        } else if let Some(message) = s.decision {
                            ui.label(message);
                        }
                    } else {
                        ui.label(format!(
                            "Share {} with the assistant that issued this code.",
                            self.name()
                        ));
                        ui.horizontal(|ui| {
                            ui.label("Pairing code");
                            ui.add(
                                egui::TextEdit::singleline(&mut assistant.code)
                                    .id(egui::Id::new("assistant pairing code"))
                                    .char_limit(64),
                            );
                        });
                        if ui.button("Share this tab").clicked() {
                            let code = assistant.code.trim().to_ascii_lowercase();
                            if code.len() == 24 && code.bytes().all(|b| b.is_ascii_hexdigit()) {
                                assistant.session = Some(Session::start(self, ctx, code));
                                assistant.code.clear();
                            } else {
                                self.report_warning(
                                    "Invalid pairing code",
                                    "Enter the 24-character pairing code from the assistant.",
                                );
                            }
                        }
                    }
                });
            assistant.open = visible;
        }
        if !stop {
            self.assistant = assistant;
        }
    }
}
