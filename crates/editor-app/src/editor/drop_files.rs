use super::{documents, Action, AnimationProject, Editor};
use eframe::egui;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;

enum Request {
    Identify(PathBuf),
    Read(PathBuf, PathBuf),
}

enum Ready {
    Identified(PathBuf, PathBuf),
    Loaded(PathBuf, PathBuf, io::Result<AnimationProject>),
}

/// Resolve aliases off the UI thread, ask the editor whether the tab already
/// exists, then read the file only if necessary. At most one project is queued.
pub(super) struct FileBatch {
    jobs: mpsc::Sender<Request>,
    results: mpsc::Receiver<Ready>,
    paths: VecDeque<PathBuf>,
    failures: Vec<String>,
}

impl FileBatch {
    fn new(ctx: egui::Context) -> io::Result<Self> {
        let (jobs, requests) = mpsc::channel();
        let (replies, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("open documents".into())
            .spawn(move || {
                for request in requests {
                    let ready = match request {
                        Request::Identify(path) => {
                            let key = documents::path_key(&path);
                            Ready::Identified(path, key)
                        }
                        Request::Read(path, key) => {
                            let result =
                                std::panic::catch_unwind(|| project_io::load_animation(&path))
                                    .unwrap_or_else(|_| {
                                        Err(io::Error::other(
                                            "The project decoder stopped unexpectedly.",
                                        ))
                                    });
                            Ready::Loaded(path, key, result)
                        }
                    };
                    if replies.send(ready).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })?;
        Ok(Self {
            jobs,
            results,
            paths: Default::default(),
            failures: Vec::new(),
        })
    }

    fn advance(&mut self) -> bool {
        self.paths
            .pop_front()
            .is_some_and(|path| self.jobs.send(Request::Identify(path)).is_ok())
    }
}

impl Editor {
    pub(super) fn dropped_projects(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        if dropped.is_empty() {
            return;
        }
        let paths: Vec<_> = dropped
            .into_iter()
            .filter_map(|file| file.path)
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("vanim"))
            })
            .collect();
        if paths.is_empty() {
            self.report_warning(
                "No project files in drop",
                "Drop a Phyro project (.vanim) to open it.",
            );
            return;
        }
        if self.canvas.path_in_progress() {
            self.report_warning(
                "Could not open dropped project",
                "Finish or cancel the current path before opening a project.",
            );
            return;
        }
        if self.exports.is_open()
            || self.keyboard_settings.is_open()
            || self.recovery_dialog.is_open()
            || self.exports.busy()
            || self.library.is_editing()
            || self.artboard_size.is_some()
            || self.text_dialog.is_some()
            || self.crash_reports.is_open()
        {
            self.report_warning(
                "Could not open dropped project",
                "Finish the open dialog before dropping a project.",
            );
            return;
        }
        if self.canvas.text_active() {
            let doc = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
            if let Err(error) = self.canvas.finish_text(ctx, doc, &mut self.selection) {
                self.report_error("Could not finish text", error);
                return;
            }
            self.finish_edit();
        }
        self.open_paths(ctx, paths);
    }

    pub(super) fn open_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() || !self.allow_document_change() {
            return;
        }
        self.queue_project_files(ctx, paths);
    }

    pub(crate) fn open_startup_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        self.welcome_preview = Some(Default::default());
        match paths.as_slice() {
            [] => {}
            [path] if !self.recovery_dialog.is_open() && !self.crash_reports.is_open() => {
                self.open_project(path.clone());
            }
            // Show recovery immediately, including when one large project was
            // requested. Resume the ordered open queue when the dialog closes.
            _ => self.queue_project_files(ctx, paths),
        }
    }

    fn queue_project_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        self.timeline.stop();
        self.note(Action::Open);
        if let Some(batch) = &mut self.file_batch {
            batch.paths.extend(paths);
            return;
        }
        if let [path] = paths.as_slice() {
            if let Some(index) = self.document_index(path, None) {
                self.focus_file_tab(index);
                return;
            }
        }
        match FileBatch::new(ctx.clone()) {
            Ok(mut batch) => {
                batch.paths = paths.into();
                batch.advance();
                self.file_batch = Some(batch);
                if let Some(preview) = &mut self.welcome_preview {
                    preview.set_opening(true);
                }
                self.status = "Opening project files…".into();
            }
            Err(error) => self.report_error("Could not start opening files", error.to_string()),
        }
    }

    pub(super) fn poll_file_batch(&mut self, ctx: &egui::Context) {
        let Some(mut batch) = self.file_batch.take() else {
            return;
        };
        match batch.results.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {
                self.file_batch = Some(batch);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.report_error(
                    "Could not finish opening files",
                    "The project loading task stopped.",
                );
                return;
            }
            Ok(Ready::Identified(path, key)) => {
                if let Some(index) = self.document_index(&path, Some(&key)) {
                    self.focus_file_tab(index);
                } else if batch.jobs.send(Request::Read(path, key)).is_ok() {
                    self.file_batch = Some(batch);
                    return;
                }
            }
            Ok(Ready::Loaded(path, key, result)) => match result {
                Ok(doc) => self.open_loaded_with_key(path, key, doc),
                Err(error) => batch.failures.push(self.open_failure_details(&path, error)),
            },
        }
        if batch.advance() {
            self.file_batch = Some(batch);
        } else if !batch.failures.is_empty() {
            let title = if batch.failures.len() == 1 {
                "Could not open project"
            } else {
                "Could not open some projects"
            };
            self.report_error(title, batch.failures.join("\n\n"));
        }
        ctx.request_repaint();
    }
}
