use super::Editor;
use eframe::egui;
use std::collections::VecDeque;
use std::time::Duration;

const HISTORY_LIMIT: usize = 20;
const BANNER_SECONDS: f64 = 10.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn color(self, visuals: &egui::Visuals) -> egui::Color32 {
        match self {
            Self::Error => visuals.error_fg_color,
            Self::Warning => visuals.warn_fg_color,
        }
    }
}

struct Notice {
    title: &'static str,
    details: String,
    severity: Severity,
    count: usize,
}

#[derive(Default)]
pub(super) struct Notices {
    history: VecDeque<Notice>,
    banner: bool,
    shown_at: Option<f64>,
    pub(super) open: bool,
    canvas_error: Option<String>,
    timeline_error: Option<String>,
    refinement_warning: Option<String>,
    pub(super) worker_reports_seen: u64,
}

impl Editor {
    pub(super) fn report_error(&mut self, title: &'static str, details: impl Into<String>) {
        self.report_notice(Severity::Error, title, details.into());
    }

    pub(super) fn report_warning(&mut self, title: &'static str, details: impl Into<String>) {
        self.report_notice(Severity::Warning, title, details.into());
    }

    fn report_notice(&mut self, severity: Severity, title: &'static str, details: String) {
        self.status.clone_from(&details);
        self.notices.push(severity, title, details);
    }
}

impl Notices {
    pub(super) fn recovery_failure(&mut self, name: &str, details: &str) {
        self.push(
            Severity::Warning,
            "Recovery checkpoint failed",
            format!("{name}\n{details}"),
        );
    }

    pub(super) fn observe_canvas_error(&mut self, error: Option<&str>) {
        if let Some(error) = changed(&mut self.canvas_error, error) {
            self.push(Severity::Error, "Canvas error", error.to_owned());
        }
    }

    pub(super) fn observe_workspace(&mut self, timeline: Option<&str>, refinement: Option<&str>) {
        if let Some(error) = changed(&mut self.timeline_error, timeline) {
            self.push(Severity::Error, "Timeline edit failed", error.to_owned());
        }
        if let Some(warning) = changed(&mut self.refinement_warning, refinement) {
            self.push(
                Severity::Warning,
                "Canvas preview warning",
                warning.to_owned(),
            );
        }
    }

    pub(super) fn observe_worker_reports(&mut self, count: u64) {
        if count > self.worker_reports_seen {
            self.worker_reports_seen = count;
            self.push(
                Severity::Error,
                "A background task stopped",
                "Open Help > Crash reports for the crash report.".into(),
            );
        }
    }

    fn push(&mut self, severity: Severity, title: &'static str, details: String) {
        if let Some(previous) = self.history.front_mut().filter(|previous| {
            previous.severity == severity && previous.title == title && previous.details == details
        }) {
            previous.count = previous.count.saturating_add(1);
        } else {
            self.history.push_front(Notice {
                title,
                details,
                severity,
                count: 1,
            });
            self.history.truncate(HISTORY_LIMIT);
        }
        self.banner = !self.open;
        self.shown_at = None;
    }

    pub(super) fn has_history(&self) -> bool {
        !self.history.is_empty()
    }

    pub(super) fn open_history(&mut self) {
        self.open = true;
        self.banner = false;
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, menu_open: bool) {
        if self.open {
            self.show_history(ctx);
        } else if self.banner
            && !menu_open
            && !ctx.memory(|m| m.any_popup_open() || m.top_modal_layer().is_some())
        {
            self.show_banner(ctx);
        }
    }

    fn show_banner(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let remaining = BANNER_SECONDS - (now - *self.shown_at.get_or_insert(now));
        if remaining <= 0. {
            self.banner = false;
            return;
        }
        ctx.request_repaint_after(Duration::from_secs_f64(remaining));

        let notice = &self.history[0];
        let mut details = false;
        let mut dismiss = false;
        egui::Area::new("editor-issue-banner".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12., 76.))
            .show(ctx, |ui| {
                let color = notice.severity.color(ui.visuals());
                egui::Frame::popup(ui.style())
                    .stroke(egui::Stroke::new(1_f32, color))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width((ctx.screen_rect().width() - 56.).clamp(200., 340.));
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(notice.title).strong().color(color),
                            )
                            .truncate(),
                        );
                        ui.add(egui::Label::new(&notice.details).truncate());
                        ui.horizontal(|ui| {
                            details = ui.button("Details").clicked();
                            dismiss = ui.button("Dismiss").clicked();
                        });
                    });
            });
        if details {
            self.open_history();
            ctx.request_repaint();
        } else if dismiss {
            self.banner = false;
            ctx.request_repaint();
        }
    }

    fn show_history(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let modal = egui::Modal::new("editor-issue-history".into()).show(ctx, |ui| {
            let screen = ctx.screen_rect();
            ui.set_width((screen.width() - 48.).clamp(220., 560.));
            ui.heading("Recent issues");
            egui::ScrollArea::vertical()
                .max_height((screen.height() - 145.).clamp(100., 480.))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, notice) in self.history.iter().enumerate() {
                        if index > 0 {
                            ui.separator();
                        }
                        ui.push_id(index, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(notice.severity.color(ui.visuals()), notice.title);
                                if notice.count > 1 {
                                    ui.weak(format!("{} times", notice.count));
                                }
                            });
                            ui.add(egui::Label::new(&notice.details).wrap().selectable(true));
                            if ui.small_button("Copy details").clicked() {
                                ctx.copy_text(format!("{}\n{}", notice.title, notice.details));
                            }
                        });
                    }
                });
            close = ui.button("Close").clicked();
        });
        if close || modal.should_close() {
            self.open = false;
            ctx.request_repaint();
        }
    }
}

fn changed<'a>(previous: &mut Option<String>, current: Option<&'a str>) -> Option<&'a str> {
    if previous.as_deref() == current {
        return None;
    }
    *previous = current.map(str::to_owned);
    current
}
