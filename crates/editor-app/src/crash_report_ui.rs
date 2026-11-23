use crate::diagnostics::{Diagnostics, Report};
use eframe::egui;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct CrashReportUi {
    open: bool,
    recovery: bool,
    focus_button: bool,
    reports: Vec<Report>,
    selected: usize,
    text: Option<String>,
    descriptions: HashMap<String, String>,
    error: Option<String>,
    acknowledge_error: Option<String>,
    copy_feedback: Option<CopyFeedback>,
    clipboard: ReportClipboard,
    #[cfg(windows)]
    windows_hang: crate::diagnostics::windows_hang::Lookup,
}

enum CopyFeedback {
    Confirmed,
    #[cfg(windows)]
    HangEvidenceAdded,
    Unconfirmed(String),
}

#[derive(Default)]
struct ReportClipboard {
    // X11 owns clipboard contents through this handle after the dialog is closed.
    desktop: Option<arboard::Clipboard>,
    #[cfg(test)]
    test_write: Option<fn(&egui::Context, String) -> CopyFeedback>,
}

impl ReportClipboard {
    fn write(&mut self, ctx: &egui::Context, text: String) -> CopyFeedback {
        #[cfg(test)]
        if let Some(write) = self.test_write {
            return write(ctx, text);
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            // eframe's native Wayland clipboard has no synchronous write receipt.
            ctx.copy_text(text);
            return CopyFeedback::Unconfirmed("Copy requested. If the clipboard is empty, select the preview and copy it manually.".into());
        }
        let _ = ctx;
        if self.desktop.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.desktop = Some(clipboard),
                Err(error) => return Self::failed(error),
            }
        }
        match self
            .desktop
            .as_mut()
            .expect("clipboard was initialized")
            .set_text(text)
        {
            Ok(()) => CopyFeedback::Confirmed,
            Err(error) => Self::failed(error),
        }
    }

    fn failed(error: arboard::Error) -> CopyFeedback {
        CopyFeedback::Unconfirmed(format!(
            "Could not write to the clipboard ({error}). Select the preview to copy it manually."
        ))
    }
}

fn label(report: &Report) -> String {
    format!("{}  ·  {}", report.timestamp_utc(), report.summary())
}

fn with_description(report: &str, description: &str) -> String {
    let mut result = report.to_owned();
    if !description.is_empty() {
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("\nWhat I was doing just before the problem\n");
        result.push_str(description);
    }
    result
}

impl CrashReportUi {
    pub(crate) fn initialize(&mut self, store: Option<&Diagnostics>) {
        let Some(store) = store else { return };
        if let Ok(reports) = store.pending() {
            if !reports.is_empty() {
                self.display(store, reports, true);
            }
        }
    }

    pub(crate) fn open(&mut self, store: Option<&Diagnostics>) {
        match store {
            Some(store) => match store.reports() {
                Ok(reports) => self.display(store, reports, false),
                Err(error) => {
                    self.unavailable(format!("Could not list the saved reports. {error}"))
                }
            },
            None => {
                self.unavailable("Crash report storage is unavailable for this session.".into())
            }
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    fn unavailable(&mut self, message: String) {
        self.reports.clear();
        self.descriptions.clear();
        self.text = None;
        self.recovery = false;
        self.open = true;
        self.focus_button = true;
        self.copy_feedback = None;
        self.error = Some(message);
    }

    fn display(&mut self, store: &Diagnostics, reports: Vec<Report>, recovery: bool) {
        self.open = true;
        self.recovery = recovery;
        self.focus_button = true;
        self.reports = reports;
        self.descriptions.clear();
        self.selected = 0;
        self.read(store);
    }

    fn read(&mut self, store: &Diagnostics) {
        self.copy_feedback = None;
        self.text = None;
        self.error = None;
        if let Some(report) = self.reports.get(self.selected) {
            match store.read(report) {
                Ok(text) => {
                    #[cfg(windows)]
                    self.windows_hang.select(report, &text);
                    self.text = Some(text);
                }
                Err(error) => self.error = Some(format!("Could not read this report. {error}")),
            }
        }
    }

    fn close(&mut self, store: Option<&Diagnostics>) {
        if let Some(store) = store {
            self.acknowledge_error = None;
            let reviewed = if self.recovery {
                self.reports.as_slice()
            } else if self.text.is_some() {
                std::slice::from_ref(&self.reports[self.selected])
            } else {
                &[]
            };
            for report in reviewed {
                if let Err(error) = store.acknowledge(report) {
                    self.acknowledge_error =
                        Some(format!("Could not mark a report as reviewed. {error}"));
                }
            }
        }
        self.open = false;
        self.text = None;
        self.descriptions.clear();
        self.reports.clear();
    }

    fn choose(&mut self, ui: &mut egui::Ui, store: Option<&Diagnostics>) {
        if self.reports.len() > 1 {
            let old = self.selected;
            egui::ComboBox::from_label("Report")
                .selected_text(label(&self.reports[old]))
                .show_ui(ui, |ui| {
                    for (index, report) in self.reports.iter().enumerate() {
                        ui.push_id(&report.filename, |ui| {
                            ui.selectable_value(&mut self.selected, index, label(report))
                                .on_hover_text(&report.filename);
                        });
                    }
                });
            if self.selected != old {
                if let Some(store) = store {
                    self.read(store);
                }
            }
        } else if let Some(report) = self.reports.first() {
            ui.label(label(report));
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui, height: f32) {
        let Some(text) = &self.text else { return };
        let name = &self.reports[self.selected].filename;
        ui.label("Copy the report and paste it into your feedback to the developer.");
        let question = ui.label("What were you doing just before the problem? (optional)");
        let id = ui.make_persistent_id(("crash feedback", name));
        let description = self.descriptions.entry(name.clone()).or_insert_with(|| {
            egui::text_edit::TextEditState::default().store(ui.ctx(), id);
            String::new()
        });
        let note = egui::ScrollArea::vertical()
            .id_salt(("feedback scroll", name))
            .max_height(ui.text_style_height(&egui::TextStyle::Body) * 3. + 8.)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(description)
                        .id(id)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(question.id)
            });
        if note.inner.changed() {
            self.copy_feedback = None;
        }
        let manual = matches!(self.copy_feedback, Some(CopyFeedback::Unconfirmed(_)));
        let combined;
        let mut preview = if manual {
            combined = with_description(text, description);
            combined.as_str()
        } else {
            text.as_str()
        };
        egui::ScrollArea::vertical()
            .id_salt(("report scroll", name))
            .max_height(height)
            .show(ui, |ui| {
                // A read-only string buffer supports native selection, navigation and Copy.
                ui.add(
                    egui::TextEdit::multiline(&mut preview)
                        .id_salt(("report preview", name))
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(1),
                );
            });
    }

    /// Returns true only on the frame that the desktop clipboard confirms a full-report write.
    pub(crate) fn show(&mut self, ctx: &egui::Context, store: Option<&Diagnostics>) -> bool {
        if !self.open {
            return false;
        }
        let mut copied = false;
        let mut close = false;
        let modal = egui::Modal::new("phyro crash reports".into()).show(ctx, |ui| {
            let screen = ctx.screen_rect().size();
            ui.set_width((screen.x - 48.).clamp(160., 590.));
            let heading = if self.recovery {
                "A diagnostic report is available"
            } else {
                "Crash reports"
            };
            ui.heading(heading);
            if self.recovery {
                ui.label("Phyro saved a report on this device. You can reopen it from the Help menu under Crash reports.");
            }
            self.choose(ui, store);
            #[cfg(windows)]
            if let Some(text) = &mut self.text {
                if self.windows_hang.update(ctx, text) && self.copy_feedback.is_some() {
                    self.copy_feedback = Some(CopyFeedback::HangEvidenceAdded);
                }
            }
            let shortcut = egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::C,
            );
            let copy_key = self.text.is_some() && ui.input_mut(|input| {
                let key = input.consume_shortcut(&shortcut);
                // egui-winit translates Command+C to Copy even when Shift is down.
                let shifted = input.modifiers.matches_exact(shortcut.modifiers)
                    || input.events.iter().any(|event| matches!(
                        event,
                        egui::Event::Key { key: egui::Key::C, pressed: false, modifiers, .. }
                            if modifiers.matches_exact(shortcut.modifiers)
                    ));
                if shifted {
                    if let Some(index) = input.events.iter().position(|event| matches!(event, egui::Event::Copy)) {
                        input.events.remove(index);
                        return true;
                    }
                }
                key
            });
            if self.text.is_some() {
                self.preview(ui, (screen.y - 315.).clamp(48., 320.));
            } else if self.error.is_none() {
                ui.label("No crash reports have been recorded.");
            }
            if let Some(error) = &self.acknowledge_error {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
                if let Some(store) = store {
                    if ui.button("Retry").clicked() {
                        if self.reports.is_empty() {
                            self.open(Some(store));
                        } else {
                            self.read(store);
                        }
                    }
                }
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if let Some(text) = &self.text {
                    let copy = ui.button("Copy report").on_hover_text(format!(
                        "Copy the entire report ({})",
                        ctx.format_shortcut(&shortcut)
                    ));
                    if self.focus_button {
                        copy.request_focus();
                    }
                    if copy.clicked() || copy_key {
                        let report = &self.reports[self.selected];
                        let description =
                            self.descriptions.get(&report.filename).map_or("", String::as_str);
                        let result = self.clipboard.write(ctx, with_description(text, description));
                        copied = matches!(result, CopyFeedback::Confirmed);
                        self.copy_feedback = Some(result);
                    }
                }
                let dismiss = ui.button("Close");
                if self.focus_button && self.text.is_none() {
                    dismiss.request_focus();
                }
                close = dismiss.clicked();
                if matches!(self.copy_feedback, Some(CopyFeedback::Confirmed)) {
                    ui.label("Report copied to clipboard.");
                }
            });
            #[cfg(windows)]
            if matches!(self.copy_feedback, Some(CopyFeedback::HangEvidenceAdded)) {
                ui.label("Windows confirmed an application hang. Copy the report again to include it.");
            }
            if let Some(CopyFeedback::Unconfirmed(message)) = &self.copy_feedback {
                ui.label(message);
                if let (Some(store), Some(report)) = (store, self.reports.get(self.selected)) {
                    let path = store.directory().join(&report.filename);
                    ui.add(egui::Label::new(format!("Saved report file: {}", path.display()))
                        .selectable(true)
                        .wrap());
                }
            }
            self.focus_button = false;
        });
        if close || modal.should_close() {
            self.close(store);
        }
        copied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, FullOutput, Key, Modifiers, OutputCommand, Pos2};

    fn mock_success(ctx: &egui::Context, report: String) -> CopyFeedback {
        ctx.copy_text(report);
        CopyFeedback::Confirmed
    }

    fn frame(
        ctx: &egui::Context,
        ui: &mut CrashReportUi,
        store: Option<&Diagnostics>,
        events: Vec<Event>,
    ) -> (bool, FullOutput) {
        let mut copied = false;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(800., 500.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                copied = ui.show(ctx, store);
            },
        );
        (copied, out)
    }

    fn control(out: &FullOutput, label: &str) -> Option<Pos2> {
        out.shapes.iter().rev().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                let rect = t.galley.rect.translate(t.pos.to_vec2());
                s.clip_rect.contains_rect(rect).then_some(rect.center())
            }
            _ => None,
        })
    }

    fn click(
        ctx: &egui::Context,
        ui: &mut CrashReportUi,
        store: &Diagnostics,
        point: Pos2,
    ) -> (bool, FullOutput) {
        frame(ctx, ui, Some(store), vec![Event::PointerMoved(point)]);
        let button = |pressed| Event::PointerButton {
            pos: point,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, ui, Some(store), vec![button(true)]);
        frame(ctx, ui, Some(store), vec![button(false)])
    }

    fn key(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            modifiers,
            pressed,
            repeat: false,
        }
    }

    fn copied_text(out: &FullOutput) -> Vec<&str> {
        out.platform_output
            .commands
            .iter()
            .filter_map(|cmd| match cmd {
                OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn enter_description(
        ctx: &egui::Context,
        viewer: &mut CrashReportUi,
        store: &Diagnostics,
        note: &str,
    ) -> FullOutput {
        frame(ctx, viewer, Some(store), vec![]);
        let (_, out) = frame(ctx, viewer, Some(store), vec![]);
        let label = control(
            &out,
            "What were you doing just before the problem? (optional)",
        )
        .unwrap();
        click(ctx, viewer, store, label + egui::vec2(0., 22.));
        frame(
            ctx,
            viewer,
            Some(store),
            vec![Event::Paste(note.to_owned())],
        );
        // Allow the modal to recenter when a long note expands the feedback field.
        frame(ctx, viewer, Some(store), vec![]).1
    }

    #[test]
    fn optional_description_is_verbatim_private_to_each_report_and_discarded_on_close() {
        let temp = tempfile::tempdir().unwrap();
        let store = Diagnostics::in_dir(temp.path().to_owned(), "UI test").unwrap();
        store.record_startup_failure("renderer", None).unwrap();
        store.record_startup_failure("window", Some(42)).unwrap();
        let ctx = egui::Context::default();
        ctx.style_mut(|s| s.animation_time = 0.);
        let mut viewer = CrashReportUi::default();
        viewer.clipboard.test_write = Some(mock_success);
        viewer.initialize(Some(&store));
        let first = viewer.reports[0].clone();
        let second = viewer.reports[1].clone();
        let original = store.read(&first).unwrap();
        let note = "  Käytin kynää 🖋\n形を選択\nYksi\nKaksi\nKolme\nNeljä\n  ";
        let out = enter_description(&ctx, &mut viewer, &store, note);
        assert_eq!(viewer.descriptions[&first.filename], note);
        assert!(control(&out, "Close").is_some());
        let (copied, out) = click(
            &ctx,
            &mut viewer,
            &store,
            control(&out, "Copy report").unwrap(),
        );
        assert!(copied);
        let expected = format!(
            "{}\n\nWhat I was doing just before the problem\n{note}",
            original.trim_end_matches('\n')
        );
        assert_eq!(copied_text(&out), [expected.as_str()]);
        click(
            &ctx,
            &mut viewer,
            &store,
            control(&out, &label(&first)).unwrap(),
        );
        let (_, menu) = frame(&ctx, &mut viewer, Some(&store), vec![]);
        // If both reports share the same timestamp/type, the last painted label is the second row.
        let row = control(&menu, &label(&second)).unwrap();
        click(&ctx, &mut viewer, &store, row);
        assert_eq!(viewer.reports[viewer.selected].filename, second.filename);
        let (_, out) = frame(&ctx, &mut viewer, Some(&store), vec![]);
        let (_, out) = click(
            &ctx,
            &mut viewer,
            &store,
            control(&out, "Copy report").unwrap(),
        );
        assert_eq!(copied_text(&out), [store.read(&second).unwrap()]);
        click(&ctx, &mut viewer, &store, control(&out, "Close").unwrap());
        assert!(viewer.descriptions.is_empty());
        assert!(store.pending().unwrap().is_empty());
        assert_eq!(store.read(&first).unwrap(), original);
        viewer.open(Some(&store));
        frame(&ctx, &mut viewer, Some(&store), vec![]);
        assert_eq!(viewer.descriptions[&first.filename], "");
    }

    #[test]
    fn clipboard_failure_keeps_complete_selectable_feedback_and_saved_file_available() {
        let temp = tempfile::tempdir().unwrap();
        let store = Diagnostics::in_dir(temp.path().to_owned(), "UI test").unwrap();
        let report = store.record_startup_failure("renderer", None).unwrap();
        let body = store.read(&report).unwrap();
        let ctx = egui::Context::default();
        let mut viewer = CrashReportUi::default();
        viewer.clipboard.test_write =
            Some(|_, _| CopyFeedback::Unconfirmed("Clipboard is busy.".into()));
        viewer.open(Some(&store));
        let note = "Valitsin tekstin.";
        let out = enter_description(&ctx, &mut viewer, &store, note);
        let (copied, out) = click(
            &ctx,
            &mut viewer,
            &store,
            control(&out, "Copy report").unwrap(),
        );
        assert!(!copied);
        assert!(copied_text(&out).is_empty());
        assert!(control(&out, "Report copied to clipboard.").is_none());
        let (_, out) = frame(&ctx, &mut viewer, Some(&store), vec![]);
        assert!(control(&out, "Clipboard is busy.").is_some());
        let path = store.directory().join(&report.filename);
        assert!(control(&out, &format!("Saved report file: {}", path.display())).is_some());
        let complete = format!(
            "{}\n\nWhat I was doing just before the problem\n{note}",
            body.trim_end_matches('\n')
        );
        let text_point = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == complete => Some(
                    t.galley
                        .rect
                        .translate(t.pos.to_vec2())
                        .intersect(s.clip_rect)
                        .center(),
                ),
                _ => None,
            })
            .unwrap();
        click(&ctx, &mut viewer, &store, text_point);
        let mods = Modifiers::COMMAND | Modifiers::CTRL;
        let (copied, out) = frame(
            &ctx,
            &mut viewer,
            Some(&store),
            vec![key(Key::A, mods, true), Event::Copy],
        );
        assert!(!copied);
        assert_eq!(copied_text(&out), [complete.as_str()]);
        assert_eq!(store.read(&report).unwrap(), body);
    }

    #[test]
    fn recovery_copies_exact_report_acknowledges_on_dismiss_and_reopens_from_help() {
        let temp = tempfile::tempdir().unwrap();
        let store = Diagnostics::in_dir(temp.path().to_owned(), "UI test").unwrap();
        let report = store.record_startup_failure("renderer", None).unwrap();
        let expected = store.read(&report).unwrap();
        let ctx = egui::Context::default();
        ctx.style_mut(|s| s.animation_time = 0.);
        let mut viewer = CrashReportUi::default();
        viewer.clipboard.test_write = Some(mock_success);
        viewer.initialize(Some(&store));
        assert!(viewer.is_open());
        frame(&ctx, &mut viewer, Some(&store), vec![]);
        let (_, out) = frame(&ctx, &mut viewer, Some(&store), vec![]);
        assert!(control(&out, "A diagnostic report is available").is_some());
        let (copied, out) = click(
            &ctx,
            &mut viewer,
            &store,
            control(&out, "Copy report").unwrap(),
        );
        assert!(copied);
        assert_eq!(copied_text(&out), [expected.as_str()]);
        assert!(control(&out, "Report copied to clipboard.").is_some());
        let (_, out) = frame(&ctx, &mut viewer, Some(&store), vec![]);
        assert!(copied_text(&out).is_empty());
        click(&ctx, &mut viewer, &store, control(&out, "Close").unwrap());
        assert!(!viewer.is_open());
        assert!(store.pending().unwrap().is_empty());
        let mut next_launch = CrashReportUi::default();
        next_launch.clipboard.test_write = Some(mock_success);
        next_launch.initialize(Some(&store));
        assert!(!next_launch.is_open());
        next_launch.open(Some(&store));
        frame(&ctx, &mut next_launch, Some(&store), vec![]);
        let (_, out) = frame(&ctx, &mut next_launch, Some(&store), vec![]);
        assert!(control(&out, "Crash reports").is_some());
        let mods = Modifiers::COMMAND | Modifiers::CTRL | Modifiers::SHIFT;
        let (copied, out) = frame(
            &ctx,
            &mut next_launch,
            Some(&store),
            vec![key(Key::C, mods, true)],
        );
        assert!(copied);
        assert_eq!(copied_text(&out), [expected.as_str()]);
        let mut keyboard_copy = false;
        let out = ctx.run(
            egui::RawInput {
                modifiers: mods,
                events: vec![Event::Copy],
                ..Default::default()
            },
            |ctx| keyboard_copy = next_launch.show(ctx, Some(&store)),
        );
        assert!(keyboard_copy);
        assert_eq!(copied_text(&out), [expected.as_str()]);
        frame(
            &ctx,
            &mut next_launch,
            Some(&store),
            vec![key(Key::Escape, Modifiers::NONE, true)],
        );
        assert!(!next_launch.is_open());
    }

    #[test]
    fn closing_history_acknowledges_the_displayed_report_and_leaves_others_pending() {
        let temp = tempfile::tempdir().unwrap();
        let store = Diagnostics::in_dir(temp.path().to_owned(), "UI test").unwrap();
        store.record_startup_failure("renderer", None).unwrap();
        store.record_startup_failure("window", None).unwrap();
        let ctx = egui::Context::default();
        let mut viewer = CrashReportUi::default();
        viewer.open(Some(&store));
        let unseen = viewer.reports[1].clone();
        frame(&ctx, &mut viewer, Some(&store), vec![]);
        let (_, out) = frame(&ctx, &mut viewer, Some(&store), vec![]);
        click(&ctx, &mut viewer, &store, control(&out, "Close").unwrap());
        assert!(!viewer.is_open());
        assert_eq!(store.pending().unwrap(), [unseen]);
        assert_eq!(store.reports().unwrap().len(), 2);
    }

    #[test]
    fn clean_launch_stays_in_editor_and_help_explains_empty_or_unavailable_storage() {
        let temp = tempfile::tempdir().unwrap();
        let store = Diagnostics::in_dir(temp.path().to_owned(), "UI test").unwrap();
        let mut viewer = CrashReportUi::default();
        viewer.clipboard.test_write = Some(mock_success);
        let ctx = egui::Context::default();
        for (storage, message) in [
            (Some(&store), "No crash reports have been recorded."),
            (
                None,
                "Crash report storage is unavailable for this session.",
            ),
        ] {
            viewer.initialize(storage);
            assert!(!viewer.is_open());
            viewer.open(storage);
            frame(&ctx, &mut viewer, storage, vec![]);
            let (copied, out) = frame(&ctx, &mut viewer, storage, vec![]);
            assert!(!copied);
            assert!(control(&out, message).is_some());
            assert!(control(&out, "Copy report").is_none());
            frame(
                &ctx,
                &mut viewer,
                storage,
                vec![key(Key::Escape, Modifiers::NONE, true)],
            );
            assert!(!viewer.is_open());
            frame(
                &ctx,
                &mut viewer,
                storage,
                vec![key(Key::Escape, Modifiers::NONE, false)],
            );
        }
    }
}
