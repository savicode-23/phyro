use super::Editor;
use crate::recovery::Writer;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const IDLE_DELAY: Duration = Duration::from_secs(2);
const MAX_DELAY: Duration = Duration::from_secs(15);

pub(super) struct DraftSession {
    pub(super) root: PathBuf,
    pub(super) writer: Writer,
    recovered_source: Option<PathBuf>,
    scheduled: Option<Checkpoint>,
    failures: Arc<AtomicBool>,
}

struct Checkpoint {
    first_edit: Instant,
    due: Instant,
}

impl Checkpoint {
    fn schedule(pending: &mut Option<Self>, now: Instant) {
        let checkpoint = pending.get_or_insert(Self {
            first_edit: now,
            due: now,
        });
        checkpoint.due = (now + IDLE_DELAY).min(checkpoint.first_edit + MAX_DELAY);
    }
}

impl DraftSession {
    fn new(root: PathBuf, failures: Arc<AtomicBool>) -> Self {
        Self {
            writer: Writer::new(root.clone()),
            root,
            recovered_source: None,
            scheduled: None,
            failures,
        }
    }

    pub(super) fn new_tab(&self) -> Self {
        Self::new(self.root.clone(), self.failures.clone())
    }

    fn poll(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        project: &document_core::AnimationProject,
        path: Option<&Path>,
    ) -> Option<String> {
        let failure = self.writer.take_error();
        let Some(checkpoint) = &self.scheduled else {
            return failure;
        };
        if now < checkpoint.due {
            ctx.request_repaint_after(checkpoint.due - now);
            return failure;
        }
        // Snapshot the committed edit only when due. The writer serializes its shared cels.
        let source = path
            .or(self.recovered_source.as_deref())
            .map(Path::to_owned);
        let wake = ctx.clone();
        let failures = self.failures.clone();
        match self
            .writer
            .checkpoint_notifying(source, project.clone(), move || {
                failures.store(true, Ordering::Release);
                wake.request_repaint();
            }) {
            Ok(()) => self.scheduled = None,
            Err(error) => {
                self.scheduled = Some(Checkpoint {
                    first_edit: now,
                    due: now + MAX_DELAY,
                });
                ctx.request_repaint_after(MAX_DELAY);
                return Some(format!("Could not start the recovery checkpoint. {error}"));
            }
        }
        failure
    }
}

impl Editor {
    pub(super) fn enable_draft_recovery(&mut self, root: PathBuf) {
        self.recovery_dialog.scan(Some(&root), true);
        self.drafts = Some(DraftSession::new(root, Arc::default()));
    }

    pub(super) fn record_draft_change(&mut self) {
        if self.baseline_is_saved {
            self.retire_draft();
        } else if let Some(drafts) = &mut self.drafts {
            Checkpoint::schedule(&mut drafts.scheduled, Instant::now());
        }
    }

    pub(super) fn park_draft_deadline(&mut self) {
        if let Some(pending) = self.drafts.as_ref().and_then(|d| d.scheduled.as_ref()) {
            let due = self.inactive_draft_due.get_or_insert(pending.due);
            *due = (*due).min(pending.due);
        }
    }

    pub(super) fn retire_draft(&mut self) {
        if let Some(drafts) = &mut self.drafts {
            drafts.scheduled = None;
            drafts.recovered_source = None;
            if let Err(error) = drafts.writer.retire() {
                self.report_warning("Could not remove recovery draft", error.to_string());
            }
        }
    }

    pub(super) fn poll_draft_recovery(&mut self, ctx: &egui::Context, now: Instant) {
        let worker_failed = self
            .drafts
            .as_ref()
            .is_some_and(|drafts| drafts.failures.swap(false, Ordering::AcqRel));
        if let Some(error) = self
            .drafts
            .as_mut()
            .and_then(|drafts| drafts.poll(ctx, now, &self.baseline, self.path.as_deref()))
        {
            let name = self.name();
            self.notices.recovery_failure(&name, &error);
            self.status = error;
        }
        // Parked tabs need attention only when a checkpoint is due or a writer reports a failure.
        if !worker_failed {
            let Some(due) = self.inactive_draft_due else {
                return;
            };
            if now < due {
                ctx.request_repaint_after(due - now);
                return;
            }
        }
        self.inactive_draft_due = None;
        for tab in self.documents.inactive_mut() {
            if let Some(drafts) = &mut tab.drafts {
                if let Some(error) = drafts.poll(ctx, now, &tab.baseline, tab.path.as_deref()) {
                    let name = tab.path.as_deref().and_then(Path::file_name);
                    let name =
                        name.map_or_else(|| "Untitled".into(), |name| name.to_string_lossy());
                    self.notices.recovery_failure(&name, &error);
                    tab.status = error;
                }
                if let Some(pending) = &drafts.scheduled {
                    let due = self.inactive_draft_due.get_or_insert(pending.due);
                    *due = (*due).min(pending.due);
                }
            }
        }
    }

    pub(super) fn show_recover_drafts(&mut self, ctx: &egui::Context) {
        let Some(project) = self.recovery_dialog.show(ctx) else {
            return;
        };
        let source = self.recovery_dialog.selected_source().map(Path::to_owned);
        let Some(claim) = self.recovery_dialog.take_recovered() else {
            return;
        };
        let stem = source
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy())
            .unwrap_or_else(|| "Untitled".into());
        self.open_document_tab(project, None);
        self.recovered_name = Some(format!("{stem} (recovered).vanim"));
        self.saved = None;
        self.baseline_is_saved = false;
        self.status = "Recovered a copy. Choose Save As to keep it.".into();
        if let Some(drafts) = &mut self.drafts {
            drafts.recovered_source = source;
            // Continue to own the original checkpoint so a second interruption
            // before the next edit can still recover this copy.
            if let Err(error) = drafts.writer.adopt(claim) {
                self.report_warning(
                    "A new recovery checkpoint is needed",
                    format!("Opened the recovered copy. {error}"),
                );
                self.record_draft_change();
            }
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_writes_are_debounced_and_continuous_edits_have_a_deadline() {
        let start = Instant::now();
        let mut pending = None;
        Checkpoint::schedule(&mut pending, start);
        assert_eq!(pending.as_ref().unwrap().due, start + IDLE_DELAY);
        for second in 1..15 {
            Checkpoint::schedule(&mut pending, start + Duration::from_secs(second));
            assert_eq!(
                pending.as_ref().unwrap().due,
                start + (Duration::from_secs(second) + IDLE_DELAY).min(MAX_DELAY)
            );
        }
        Checkpoint::schedule(&mut pending, start + Duration::from_secs(16));
        assert_eq!(pending.unwrap().due, start + MAX_DELAY);
    }

    #[test]
    fn a_parked_checkpoint_failure_wakes_the_editor_after_its_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("drafts");
        let ctx = egui::Context::default();
        let mut editor = Editor::new(&ctx);
        editor.enable_draft_recovery(root.clone());
        // Block checkpoint creation after the recovery directory has been scanned.
        std::fs::write(root, []).unwrap();
        editor.project.rename_clip(0, "Edited drawing").unwrap();
        editor.finish_edit();
        editor.new_document_tab();

        let due = editor.inactive_draft_due.unwrap();
        editor.poll_draft_recovery(&ctx, due);
        assert!(editor.inactive_draft_due.is_none());
        let drafts = editor.drafts.as_ref().unwrap();
        let timeout = Instant::now() + Duration::from_secs(5);
        while !drafts.failures.load(Ordering::Acquire) {
            assert!(
                Instant::now() < timeout,
                "checkpoint failure did not wake the editor"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        editor.poll_draft_recovery(&ctx, due);
        assert!(editor.notices.has_history());
        assert!(editor
            .documents
            .inactive_mut()
            .next()
            .unwrap()
            .status
            .contains("Could not checkpoint the recovery draft"));
    }
}
