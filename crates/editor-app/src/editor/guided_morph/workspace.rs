use super::MorphDialog;
use document_core::{Document, VectorObject};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};
use std::time::Duration;

mod art;
use art::Art;

pub(super) const MATTE: Color32 = Color32::from_rgb(218, 225, 231);
pub(super) const INK: Color32 = Color32::from_rgb(46, 66, 76);
pub(super) use crate::theme::MINT;

pub(super) struct Workspace {
    matching: bool,
    playing: bool,
    clock: Option<(f64, u32)>,
    art: Art,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            matching: false,
            playing: true,
            clock: None,
            art: Default::default(),
        }
    }
}

impl Workspace {
    pub(super) fn reset_previews(&mut self) {
        self.art = Default::default();
        self.clock = None;
    }
}

impl MorphDialog {
    pub(super) fn window(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        object: &VectorObject,
        fps: u32,
    ) -> (bool, bool) {
        let id = egui::Id::new("shape-morph-window");
        ctx.memory_mut(|m| m.set_modal_layer(egui::LayerId::new(egui::Order::Middle, id)));
        let screen = ctx.screen_rect();
        let body = vec2(
            (screen.width() * 0.56).min(540.),
            (screen.height() * 0.46).min(300.),
        );
        // Start over the right-hand inspector to leave the drawing in view. egui remembers
        // the user's position and size on subsequent opens through the stable window ID.
        let style = ctx.style();
        let frame = egui::Frame::window(&style);
        let title_font = egui::TextStyle::Heading.resolve(&style);
        let title_height = ctx
            .fonts(|f| f.row_height(&title_font))
            .max(style.spacing.interact_size.y)
            + frame.inner_margin.sum().y
            + frame.stroke.width;
        let chrome = frame.total_margin().sum() + vec2(0., title_height);
        let pos = pos2(screen.right() - body.x - chrome.x - 12., screen.top() + 44.);
        let mut open = true;
        let outcome = egui::Window::new("Shape morph")
            .id(id)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(pos)
            .default_size(body)
            .min_size(vec2(350., 200.))
            .max_size((screen.size() - chrome).max(vec2(1., 1.)))
            .constrain_to(screen)
            .show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                self.workspace(ui, doc, object, fps)
            });
        let (done, cancel) = outcome.and_then(|window| window.inner).unwrap_or_default();
        (done, cancel || !open)
    }

    fn workspace(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        object: &VectorObject,
        fps: u32,
    ) -> (bool, bool) {
        let mut done = false;
        let mut cancel = false;
        let mut swap = false;
        ui.horizontal_wrapped(|ui| {
            let was_matching = self.workspace.matching;
            ui.selectable_value(&mut self.workspace.matching, false, "Preview");
            ui.add_enabled_ui(self.to.is_some(), |ui| {
                ui.selectable_value(&mut self.workspace.matching, true, "Match points");
            });
            if was_matching != self.workspace.matching {
                self.pending = None;
                self.workspace.playing = !self.workspace.matching;
                self.workspace.clock = None;
            }
            if self.to.is_some() {
                ui.menu_button("Options", |ui| {
                    self.settings(ui, doc, object);
                    if self.pair.is_some() && ui.button("Swap poses").clicked() {
                        swap = true;
                        ui.close_menu();
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                done = ui
                    .add_enabled(
                        self.ready(),
                        egui::Button::new(egui::RichText::new("Done").color(INK)).fill(MINT),
                    )
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if swap {
            self.swap_poses(doc);
            ui.ctx().request_repaint();
            return (false, false);
        }
        if self.to.is_none() {
            self.settings(ui, doc, object);
        }
        if let Some(message) = self
            .error
            .as_ref()
            .or_else(|| self.plan.as_ref()?.as_ref().err())
        {
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
        if self.to.is_none() {
            return (done, cancel);
        }
        ui.separator();
        let can_play = self.plan.as_ref().is_some_and(Result::is_ok);
        if can_play
            && !ui.ctx().wants_keyboard_input()
            && !ui.memory(|m| m.any_popup_open())
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.toggle_playback(ui);
        }
        if self.workspace.matching {
            // Give the two editable outlines the space normally used by the playback strip.
            self.match_points(ui, object, ui.available_height());
            return (done, cancel);
        }
        self.prepare_art(object);
        if can_play {
            self.advance(ui, fps);
        }
        let compact = ui.available_height() < 340.;
        let strip_height = if compact { 48. } else { 80. };
        let stage_height = (ui.available_height() - strip_height - 40.).max(64.);
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), stage_height), Sense::hover());
        ui.painter().rect_filled(rect, 3., MATTE);
        if let Some(shape) = &self.between {
            let frame = self.current_frame();
            self.workspace.art.paint_current(ui, rect, frame, shape);
        }
        ui.separator();
        self.filmstrip(ui, object, strip_height);
        (done, cancel)
    }

    fn current_frame(&self) -> u32 {
        self.start + ((self.end - self.start) as f32 * self.progress).round() as u32
    }

    fn seek(&mut self, frame: u32) {
        let progress = (frame - self.start) as f32 / (self.end - self.start) as f32;
        if progress != self.progress {
            self.progress = progress;
            self.sample_preview();
        }
    }

    fn toggle_playback(&mut self, ui: &Ui) {
        self.workspace.playing = !self.workspace.playing;
        self.workspace.clock = None;
        if self.workspace.playing {
            self.workspace.matching = false;
            self.pending = None;
            ui.ctx().request_repaint();
        }
    }

    fn advance(&mut self, ui: &Ui, fps: u32) {
        if !self.workspace.playing {
            return;
        }
        let now = ui.input(|i| i.time);
        let offset = self.current_frame() - self.start;
        let (since, first) = *self.workspace.clock.get_or_insert((now, offset));
        let (tick, wait) = crate::timeline::playback_sample(
            first,
            Duration::from_secs_f64((now - since).max(0.)),
            self.end - self.start + 1,
            fps,
            true,
        );
        self.seek(self.start + tick);
        if let Some(wait) = wait {
            ui.ctx().request_repaint_after(wait);
        }
    }

    fn prepare_art(&mut self, object: &VectorObject) {
        if !self.workspace.art.thumbnails.is_empty() {
            return;
        }
        let (Some(Ok(plan)), Some(target)) = (&self.plan, &self.to) else {
            return;
        };
        let duration = self.end - self.start;
        let steps = duration.min(4);
        let poses = (0..=steps).map(|i| {
            let offset = (i * duration + steps / 2) / steps;
            let shape = if offset == 0 {
                self.from.clone()
            } else if offset == duration {
                target.clone()
            } else {
                plan.sample(self.easing.sample(f64::from(offset) / f64::from(duration)))
            };
            (self.start + offset, shape)
        });
        self.workspace.art.prepare(object, poses);
    }

    fn filmstrip(&mut self, ui: &mut Ui, object: &VectorObject, height: f32) {
        let can_play = self.plan.as_ref().is_some_and(Result::is_ok);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(76.);
                let name = if object.name.is_empty() {
                    "Shape"
                } else {
                    &object.name
                };
                ui.add_sized([76., 18.], egui::Label::new(name).truncate());
                let label = if self.workspace.playing {
                    "Pause"
                } else {
                    "Play"
                };
                if ui
                    .add_enabled(can_play, egui::Button::new(label))
                    .on_hover_text("Play or pause (Space)")
                    .clicked()
                {
                    self.toggle_playback(ui);
                }
                ui.label(format!("Frame {}", self.current_frame() + 1));
            });
            let (rect, response) = ui.allocate_exact_size(
                vec2(ui.available_width().max(1.), height + 20.),
                Sense::click_and_drag(),
            );
            let n = self.workspace.art.thumbnails.len();
            if n == 0 {
                return;
            }
            let tile_w = (rect.width() / n as f32).max(1.);
            let ruler = rect.bottom() - 4.;
            let color = ui.visuals().text_color();
            // Cell centers are actual frame coordinates, including short spans
            // whose five representative frames need rounding.
            let left = rect.left() + tile_w * 0.5;
            let right = rect.right() - tile_w * 0.5;
            let x = |t: f32| egui::lerp(left..=right, t);
            for i in 0..n {
                let tick = self.workspace.art.thumbnails[i].tick;
                let center_x = x((tick - self.start) as f32 / (self.end - self.start) as f32);
                let tile = Rect::from_center_size(
                    pos2(
                        rect.left() + tile_w * (i as f32 + 0.5),
                        rect.top() + height * 0.5,
                    ),
                    vec2((tile_w - 4.).max(1.), height),
                );
                ui.painter().rect_filled(tile, 3., MATTE);
                self.workspace.art.paint_thumbnail(ui, tile, i);
                ui.painter().text(
                    pos2(center_x, ruler - 2.),
                    Align2::CENTER_BOTTOM,
                    (tick + 1).to_string(),
                    FontId::monospace(10.),
                    color,
                );
            }
            ui.painter().line_segment(
                [pos2(left, ruler), pos2(right, ruler)],
                Stroke::new(1.5_f32, MINT),
            );
            for x in [left, right] {
                let p = pos2(x, ruler);
                let d = 3.5;
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        p - vec2(d, 0.),
                        p - vec2(0., d),
                        p + vec2(d, 0.),
                        p + vec2(0., d),
                    ],
                    MINT,
                    Stroke::NONE,
                ));
            }
            if let Some(p) = response
                .interact_pointer_pos()
                .filter(|_| response.clicked() || response.dragged())
            {
                let t = egui::remap_clamp(p.x, left..=right, 0.0..=1.0);
                self.workspace.playing = false;
                self.workspace.clock = None;
                self.seek(self.start + ((self.end - self.start) as f32 * t).round() as u32);
            }
            response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Scrub the morph");
            let cursor = x(self.progress);
            ui.painter()
                .vline(cursor, rect.top()..=ruler + 3., (1., MINT));
            ui.painter()
                .circle_filled(Pos2::new(cursor, rect.top() + 2.), 3., MINT);
        });
    }
}
