use super::{AnimationAction, Timeline, MAX_ONIONS};
use crate::keymap::{self, Action};
use document_core::{Clip, MAX_FRAMES};
use eframe::egui::{
    self, vec2, Align, Color32, Layout, Rect, Response, Sense, Shape, Stroke, Ui, WidgetInfo,
    WidgetType,
};

impl Timeline {
    pub(super) fn transport(&mut self, ui: &mut Ui, clip: &mut Clip) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            if Icon::Previous
                .show(ui, false)
                .on_hover_ui(|ui| {
                    ui.label(keymap::hint(
                        ui.ctx(),
                        "Previous frame",
                        Action::PreviousFrame,
                    ));
                })
                .clicked()
            {
                self.step_frame(clip, false);
            }
            ui.add_enabled_ui(clip.frames > 1, |ui| {
                let play = if self.is_playing() {
                    Icon::Pause
                } else {
                    Icon::Play
                };
                if play
                    .show(ui, self.is_playing())
                    .on_hover_ui(|ui| {
                        ui.label(keymap::hint(ui.ctx(), "Play or pause", Action::Play));
                    })
                    .clicked()
                {
                    self.toggle_clip(clip);
                    ui.ctx().request_repaint();
                }
            });
            if Icon::Next
                .show(ui, false)
                .on_hover_ui(|ui| {
                    ui.label(keymap::hint(ui.ctx(), "Next frame", Action::NextFrame));
                })
                .clicked()
            {
                self.step_frame(clip, true);
            }
            if Icon::Loop
                .show(ui, clip.looping)
                .on_hover_text("Loop playback")
                .clicked()
            {
                self.stop();
                clip.looping = !clip.looping;
                changed = true;
            }
            ui.separator();
            let mut number = self.frame + 1;
            let counter = ui
                .add(
                    egui::DragValue::new(&mut number)
                        .range(1..=clip.frames)
                        .update_while_editing(false)
                        .prefix("Frame "),
                )
                .on_hover_text("Go to frame. Click to type or drag to scrub.");
            if counter.clicked() || counter.drag_started() {
                self.stop();
            }
            if counter.changed() {
                self.select_frame(number - 1);
                self.reveal_frame = true;
            }
            if let Some(frames) = clip_number(
                ui,
                &mut self.editing_length,
                clip.session_id(),
                clip.frames,
                MAX_FRAMES,
                if clip.frames == 1 {
                    " frame"
                } else {
                    " frames"
                },
                "Clip length in frames. Click to type or drag to adjust.",
            ) {
                self.stop();
                if self.result(clip.set_length(frames)).is_some() {
                    self.frame = self.frame.min(frames - 1);
                    self.deselect_grid();
                    self.reveal_frame = true;
                    changed = true;
                }
            }
            if let Some(fps) = clip_number(
                ui,
                &mut self.editing_fps,
                clip.session_id(),
                clip.fps,
                document_core::MAX_FPS,
                " fps",
                "Clip frame rate. Click to type or drag to adjust.",
            ) {
                self.stop();
                changed |= self.result(clip.set_fps(fps)).is_some();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().interact_size.x = 23.;
                let future_changed = onion_direction(
                    ui,
                    "Future",
                    "+",
                    &mut self.next_onions,
                    &mut self.next_tint,
                );
                let past_changed = onion_direction(
                    ui,
                    "Past",
                    "−",
                    &mut self.previous_onions,
                    &mut self.previous_tint,
                );
                if past_changed || future_changed {
                    self.onion = true;
                }
                if Icon::SkinSettings
                    .show(ui, false)
                    .on_hover_ui(|ui| {
                        ui.label(keymap::hint(
                            ui.ctx(),
                            "Onion skin settings",
                            Action::OnionSettings,
                        ));
                    })
                    .clicked()
                {
                    self.onion_settings_open = !self.onion_settings_open;
                }
                if Icon::Onion
                    .show(ui, self.onion)
                    .on_hover_ui(|ui| {
                        ui.label(keymap::hint(ui.ctx(), "Show onion skins", Action::Onion));
                    })
                    .clicked()
                {
                    self.onion = !self.onion;
                }
            });
        });
        ui.add(egui::Separator::default().spacing(2.));
        changed
    }

    pub(super) fn drawing_actions(
        &mut self,
        ui: &mut Ui,
        clip: &mut Clip,
        selected: Option<u64>,
        active_layer: u64,
    ) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.;
            let playing = self.is_playing();
            ui.add_enabled_ui(!playing, |ui| {
                let layer = clip.drawing_at(self.frame).layers.iter().find(|l| l.id == active_layer);
                let editable = layer.is_some_and(|l| l.visible && !l.locked);
                let blank = ui.add_enabled_ui(editable, |ui| {
                    Icon::Blank
                        .show(ui, false)
                        .on_disabled_hover_text("Choose a visible, unlocked layer to add a blank frame.")
                        .on_hover_ui(|ui| {
                            if let Some(layer) = layer {
                                ui.label(keymap::hint(ui.ctx(), &format!("Blank artwork on {}", layer.name), Action::BlankFrame));
                            }
                            ui.label("Start here inside a hold, or advance one frame from a drawing start.");
                            ui.label("Other layers and nested clips keep playing.");
                        })
                        .clicked()
                }).inner;
                if blank {
                    changed |= self.run_action(AnimationAction::Blank, clip, selected, active_layer);
                }
                if ui.add_enabled_ui(editable, |ui| Icon::Duplicate
                    .show(ui, false)
                    .on_hover_ui(|ui| {
                        ui.label(keymap::hint(ui.ctx(), "Copy the active layer drawing", Action::DuplicateFrames));
                    })).inner
                    .clicked()
                {
                    changed |= self.run_action(AnimationAction::Duplicate, clip, selected, active_layer);
                }
            });
            ui.separator();
            let (can_key, can_remove) = self.key_actions(clip, selected);
            ui.add_enabled_ui(can_key && !playing, |ui| {
                if Icon::AddKey
                    .show(ui, false)
                    .on_hover_ui(|ui| {
                        ui.label(keymap::hint(ui.ctx(), &format!("Key {} at its current value", self.channel.label()), Action::Keyframe));
                    })
                    .clicked()
                {
                    changed |= self.run_action(AnimationAction::Keyframe, clip, selected, active_layer);
                }
            });
            ui.add_enabled_ui(!playing && can_remove, |ui| {
                if Icon::RemoveKey
                    .show(ui, false)
                    .on_hover_ui(|ui| {
                        ui.label(keymap::hint(ui.ctx(), "Remove selected keys or the current track key", Action::Delete));
                    })
                    .clicked()
                {
                    changed |= self.remove_keys(clip, selected);
                }
            });
            ui.separator();
            ui.add_enabled_ui(!playing && crate::commands::is_editable(clip.drawing_at(self.frame),active_layer), |ui| {
                let label = ui.label("Hold");
                let (start, mut duration) = clip.layer_exposure(self.frame,active_layer);
                if ui
                    .add(
                        egui::DragValue::new(&mut duration)
                            .range(1..=MAX_FRAMES)
                            .suffix(" fr"),
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Duration of the drawing on the active layer")
                    .changed()
                    && self
                        .result(clip.set_layer_exposure(self.frame,active_layer,duration))
                        .is_some()
                {
                    self.select_frame(self.frame.min(start + duration - 1));
                    changed = true;
                }
            });
            ui.menu_button(format!("Track: {}", self.channel.label()), |ui| {
                changed |= self.key_menu(ui, clip, selected.filter(|_| !playing));
            });
            if ui
                .add_enabled(selected.is_some() && !playing, egui::Button::new("Curve"))
                .on_hover_text("Edit the active property curve")
                .clicked()
            {
                self.graph_open = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.timeline_scale(ui);
            });
        });
        changed
    }
}

// Apply numeric edits at the end of a gesture. Resizing a long clip can touch many drawings.
fn clip_number(
    ui: &mut Ui,
    edit: &mut Option<(u64, u32)>,
    clip_id: u64,
    current: u32,
    max: u32,
    suffix: &str,
    hint: &str,
) -> Option<u32> {
    let mut value = edit
        .filter(|(id, _)| *id == clip_id)
        .map_or(current, |(_, n)| n);
    let field = ui
        .add(
            egui::DragValue::new(&mut value)
                .range(1..=max)
                .speed(0.2)
                .update_while_editing(false)
                .suffix(suffix),
        )
        .on_hover_text(hint);
    if field.changed() {
        *edit = Some((clip_id, value));
    }
    if edit.is_some() && (field.drag_stopped() || (!field.dragged() && !field.has_focus())) {
        *edit = None;
        return (value != current).then_some(value);
    }
    None
}

fn onion_direction(
    ui: &mut Ui,
    direction: &str,
    sign: &str,
    count: &mut u32,
    tint: &mut [u8; 4],
) -> bool {
    let number = ui
        .add(
            egui::DragValue::new(count)
                .range(0..=MAX_ONIONS)
                .speed(0.2)
                .prefix(sign),
        )
        .on_hover_text(format!("{direction} onion poses. Zero hides this side."));
    number.widget_info(|| {
        let mut info = WidgetInfo::drag_value(ui.is_enabled(), f64::from(*count));
        info.label = Some(format!("{direction} onion poses"));
        info
    });
    let color = crate::color_picker::button(ui, tint)
        .on_hover_text(format!("{direction} onion tint and opacity"));
    color.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::ColorButton,
            ui.is_enabled(),
            format!("{direction} onion tint"),
        )
    });
    number.changed() || color.changed()
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    NewClip,
    DuplicateClip,
    DeleteClip,
    ExportClip,
    ExportSet,
    Previous,
    Play,
    Pause,
    Next,
    Loop,
    Blank,
    Duplicate,
    AddKey,
    RemoveKey,
    Onion,
    SkinSettings,
}

impl Icon {
    fn label(self) -> &'static str {
        match self {
            Self::NewClip => "New clip",
            Self::DuplicateClip => "Duplicate clip",
            Self::DeleteClip => "Delete clip",
            Self::ExportClip => "Export clip",
            Self::ExportSet => "Export set",
            Self::Previous => "Previous",
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Next => "Next",
            Self::Loop => "Loop playback",
            Self::Blank => "Blank frame",
            Self::Duplicate => "Duplicate",
            Self::AddKey => "Add key",
            Self::RemoveKey => "Remove key",
            Self::Onion => "Onion skin",
            Self::SkinSettings => "Onion skin settings",
        }
    }
    pub(super) fn show(self, ui: &mut Ui, selected: bool) -> Response {
        let label = self.label();
        let has_caption = matches!(
            self,
            Self::Onion | Self::Blank | Self::Duplicate | Self::AddKey
        );
        let caption = has_caption.then(|| {
            ui.painter().layout_no_wrap(
                label.to_owned(),
                egui::TextStyle::Button.resolve(ui.style()),
                Color32::WHITE,
            )
        });
        let width = caption.as_ref().map_or(24., |text| 28. + text.size().x);
        let (_, rect) = ui.allocate_space(vec2(width, 23.));
        let response = ui.interact(
            rect,
            egui::Id::new(("timeline-action", label)),
            Sense::click(),
        );
        response.widget_info(|| {
            if matches!(self, Self::Onion | Self::Loop) {
                WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), selected, label)
            } else {
                WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label)
            }
        });
        let visuals = ui.style().interact_selectable(&response, selected);
        let p = ui.painter();
        if selected || response.hovered() || response.has_focus() || response.highlighted() {
            p.rect_filled(rect, 3., visuals.weak_bg_fill);
        }
        if response.has_focus() {
            p.rect_stroke(rect, 3., visuals.fg_stroke, egui::StrokeKind::Inside);
        }
        let ink = visuals.fg_stroke.color;
        let center = rect.left_center() + vec2(12., 0.);
        self.draw(p, center, ink, selected);
        if let Some(caption) = caption {
            p.galley_with_override_text_color(
                rect.left_center() + vec2(23., -caption.size().y * 0.5),
                caption,
                ink,
            );
        }
        response
    }

    fn draw(self, painter: &egui::Painter, center: egui::Pos2, ink: Color32, selected: bool) {
        let at = |x, y| center + vec2(x, y);
        let stroke = Stroke::new(1.4_f32, ink);
        let line = |a, b| {
            painter.line_segment([a, b], stroke);
        };
        match self {
            Self::Previous | Self::Next | Self::Play => {
                let dir = if matches!(self, Self::Previous) {
                    -1.
                } else {
                    1.
                };
                painter.add(Shape::convex_polygon(
                    vec![at(-4. * dir, -5.), at(4. * dir, 0.), at(-4. * dir, 5.)],
                    ink,
                    Stroke::NONE,
                ));
                if !matches!(self, Self::Play) {
                    line(at(6. * dir, -5.), at(6. * dir, 5.));
                }
            }
            Self::Pause => {
                for x in [-3., 3.] {
                    painter.rect_filled(Rect::from_center_size(at(x, 0.), vec2(3., 10.)), 0., ink);
                }
            }
            Self::Loop => {
                painter.add(Shape::line(
                    vec![
                        at(-7., 0.),
                        at(-7., -2.),
                        at(-5., -4.),
                        at(6., -4.),
                        at(3., -7.),
                    ],
                    stroke,
                ));
                painter.add(Shape::line(
                    vec![at(7., 0.), at(7., 2.), at(5., 4.), at(-6., 4.), at(-3., 7.)],
                    stroke,
                ));
            }
            Self::SkinSettings => {
                painter.add(Shape::line(
                    vec![at(-4., -2.), at(0., 2.), at(4., -2.)],
                    stroke,
                ));
            }
            Self::Onion => {
                painter.rect_stroke(
                    Rect::from_center_size(center, vec2(17., 10.)),
                    5.,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                painter.circle_filled(at(if selected { 3.5 } else { -3.5 }, 0.), 3., ink);
            }
            Self::Blank => {
                painter.add(Shape::closed_line(
                    vec![
                        at(-5., -7.),
                        at(2., -7.),
                        at(5., -4.),
                        at(5., 7.),
                        at(-5., 7.),
                    ],
                    stroke,
                ));
                line(at(-2., 1.), at(2., 1.));
                line(at(0., -1.), at(0., 3.));
            }
            Self::NewClip => {
                line(at(-6., 0.), at(6., 0.));
                line(at(0., -6.), at(0., 6.));
            }
            Self::DeleteClip => {
                line(at(-6., -4.), at(6., -4.));
                line(at(-2., -6.), at(2., -6.));
                painter.add(Shape::line(
                    vec![at(-4., -2.), at(-3., 6.), at(3., 6.), at(4., -2.)],
                    stroke,
                ));
            }
            Self::ExportClip | Self::ExportSet => {
                painter.add(Shape::line(
                    vec![at(-6., 1.), at(-6., 6.), at(6., 6.), at(6., 1.)],
                    stroke,
                ));
                line(at(0., 2.), at(0., -7.));
                painter.add(Shape::line(
                    vec![at(-4., -3.), at(0., -7.), at(4., -3.)],
                    stroke,
                ));
            }
            Self::Duplicate | Self::DuplicateClip => {
                painter.add(Shape::line(
                    vec![at(-6., 3.), at(-6., -6.), at(3., -6.)],
                    stroke,
                ));
                painter.rect_stroke(
                    Rect::from_min_max(at(-3., -3.), at(6., 6.)),
                    1.,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
            Self::AddKey | Self::RemoveKey => {
                painter.add(Shape::closed_line(
                    vec![at(-7., 0.), at(-2., -5.), at(3., 0.), at(-2., 5.)],
                    stroke,
                ));
                line(at(3., 3.), at(8., 3.));
                if matches!(self, Self::AddKey) {
                    line(at(5.5, 0.5), at(5.5, 5.5));
                }
            }
        }
    }
}
