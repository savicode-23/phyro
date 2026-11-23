mod frame_selection;
mod navigator;
mod ruler;
mod shortcuts;
mod toolbar;
mod tracks;
use frame_selection::FrameFocus;
pub(crate) use frame_selection::FrameSelection;
pub(crate) use shortcuts::AnimationAction;
pub(crate) const MAX_ONIONS: u32 = 8;
const MIN_FRAME_WIDTH: f32 = 3.;
pub(crate) const DEFAULT_FRAME_WIDTH: f32 = 18.;
const MAX_FRAME_WIDTH: f32 = 120.;
use document_core::{AnimationProject, Clip, Document, Interpolation};
use eframe::egui::{self, Button, Ui};
use std::time::{Duration, Instant};
pub(crate) use tracks::KeyMode;
use tracks::{Channel, KeyValue};

pub struct Timeline {
    pub frame: u32,
    pub(crate) export_request: Option<crate::exports::AnimationExport>,
    navigator_dialog: Option<navigator::Dialog>,
    pub(crate) key_mode: KeyMode,
    pub(crate) layer_request: Option<u64>,
    active_layer: u64,
    cell_menu: Option<(u64, u64, u32)>,
    revealed_object: Option<(u64, u32, u64)>,
    pub(crate) guided_morph_request: Option<u64>,
    pub clip: usize,
    pub onion: bool,
    pub previous_onions: u32,
    pub next_onions: u32,
    pub previous_tint: [u8; 4],
    pub next_tint: [u8; 4],
    playback: Option<(Instant, u32)>,
    frame_focus: Option<FrameFocus>,
    channel: Channel,
    copied_key: Option<(Channel, KeyValue)>,
    frame_width: f32,
    expanded: bool,
    graph_open: bool,
    onion_settings_open: bool,
    reveal_frame: bool,
    editing_fps: Option<(u64, u32)>,
    editing_length: Option<(u64, u32)>,
    gesture: Option<ruler::Gesture>,
    keys: ruler::KeySelection,
    error: Option<String>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            frame: 0,
            export_request: None,
            navigator_dialog: None,
            key_mode: KeyMode::default(),
            guided_morph_request: None,
            layer_request: None,
            active_layer: 1,
            cell_menu: None,
            revealed_object: None,
            clip: 0,
            onion: false,
            previous_onions: 1,
            next_onions: 1,
            previous_tint: [244, 107, 133, 84],
            next_tint: [120, 168, 244, 84],
            playback: None,
            frame_focus: None,
            channel: Channel::Position,
            copied_key: None,
            frame_width: DEFAULT_FRAME_WIDTH,
            expanded: true,
            graph_open: false,
            onion_settings_open: false,
            reveal_frame: true,
            editing_fps: None,
            editing_length: None,
            gesture: None,
            keys: Default::default(),
            error: None,
        }
    }
}
impl Timeline {
    pub(crate) fn frame_width(&self) -> f32 {
        self.frame_width
    }

    pub(crate) fn set_frame_width(&mut self, width: f32) {
        self.frame_width = width.clamp(MIN_FRAME_WIDTH, MAX_FRAME_WIDTH);
    }

    pub(crate) fn reset_for_document(&mut self) {
        *self = Self {
            frame_width: self.frame_width,
            ..Self::default()
        };
    }

    pub fn panel(ctx: &egui::Context, contents: impl FnOnce(&mut Ui)) {
        let available = ctx.available_rect().height();
        let max_height = (available - 140.).max(98.);
        let default_height = if available < 550. { 178. } else { 206. };
        egui::TopBottomPanel::bottom("timeline")
            .default_height(default_height)
            .height_range(98.0..=max_height)
            .resizable(true)
            .show(ctx, |ui| {
                let top = ui.max_rect().center_top();
                ui.painter().hline(
                    top.x - 14.0..=top.x + 14.0,
                    top.y,
                    egui::Stroke::new(2.0_f32, ui.visuals().weak_text_color()),
                );
                ui.add_space(4.);
                contents(ui);
            });
    }

    pub fn is_playing(&self) -> bool {
        self.playback.is_some()
    }
    pub(crate) fn park(&mut self) {
        self.stop();
        self.gesture = None;
    }

    pub fn stop(&mut self) {
        self.playback = None;
    }
    #[cfg(test)]
    pub fn toggle(&mut self, doc: &Document) {
        self.toggle_playback(doc.frames, true);
    }
    pub fn toggle_clip(&mut self, clip: &Clip) {
        self.toggle_playback(clip.frames, clip.looping);
    }
    fn toggle_playback(&mut self, frames: u32, looping: bool) {
        self.error = None;
        if self.is_playing() {
            self.stop();
        } else if frames > 1 {
            if !looping && self.frame + 1 >= frames {
                self.frame = 0;
            }
            self.frame_focus = None;
            self.playback = Some((Instant::now(), self.frame));
        }
    }
    pub fn advance_clip(&mut self, ctx: &egui::Context, clip: &Clip) {
        if let Some((started, first)) = self.playback {
            let (frame, wait) = playback_sample(
                first,
                started.elapsed(),
                clip.frames,
                clip.fps,
                clip.looping,
            );
            self.reveal_frame |= self.frame != frame;
            self.frame = frame;
            if let Some(wait) = wait {
                ctx.request_repaint_after(wait);
            } else {
                self.stop();
            }
        }
    }
    pub fn key_selected_track(&self, doc: &mut Document, selected: Option<u64>) -> bool {
        if self.is_playing() || self.frame >= doc.frames {
            return false;
        }
        let Some(index) = selected.and_then(|id| doc.objects.iter().position(|o| o.id == id))
        else {
            return false;
        };
        let o = &doc.objects[index];
        if !crate::commands::is_editable(doc, o.layer) || self.channel.is_muted(o) {
            return false;
        }
        self.channel.add(&mut doc.objects[index], self.frame)
    }
    fn select_frame(&mut self, tick: u32) {
        self.error = None;
        self.stop();
        if let Some(focus) = &mut self.frame_focus {
            focus.single(tick);
        }
        self.frame = tick;
    }
    pub(crate) fn focused_frames(&self, clip: &Clip) -> Option<&FrameSelection> {
        self.frame_focus
            .as_ref()
            .filter(|focus| focus.clip == clip.session_id())
            .map(|focus| &focus.selection)
    }
    pub(crate) fn delete_frames(&mut self, clip: &mut Clip, ranges: &[(u32, u32)]) -> bool {
        self.reveal_frame = true;
        self.frame_edit(clip.delete_frame_ranges(ranges))
    }
    fn jump_to_clip(&mut self, index: usize) {
        self.stop();
        self.editing_fps = None;
        self.editing_length = None;
        self.frame = 0;
        self.reveal_frame = true;
        self.clip = index;
        self.frame_focus = None;
        self.gesture = None;
        self.keys = Default::default();
        self.error = None;
    }
    pub(crate) fn restore_playhead(&mut self, clip: usize, frame: u32) {
        self.error = None;
        self.deselect_grid();
        if self.clip != clip {
            self.jump_to_clip(clip);
        }
        if self.frame != frame {
            self.select_frame(frame);
        }
        self.reveal_frame = true;
    }
    fn result<T>(&mut self, result: Result<T, String>) -> Option<T> {
        match result {
            Ok(x) => {
                self.error = None;
                Some(x)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    fn frame_edit(&mut self, result: Result<u32, String>) -> bool {
        if let Some(tick) = self.result(result) {
            self.select_frame(tick);
            true
        } else {
            false
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &mut AnimationProject,
        selected: Option<u64>,
        active_layer: u64,
        selected_bone: Option<(u64, u32, u64)>,
    ) -> bool {
        self.active_layer = active_layer;
        self.clip = self.clip.min(project.clips.len() - 1);
        self.frame = self.frame.min(project.clips[self.clip].frames - 1);
        if let Some(focus) = &mut self.frame_focus {
            let clip = &project.clips[self.clip];
            if focus.clip != clip.session_id() {
                self.frame_focus = None;
            } else if focus.anchor >= clip.frames
                || focus
                    .selection
                    .ranges()
                    .next_back()
                    .is_some_and(|(_, last)| last >= clip.frames)
            {
                focus.single(self.frame);
            }
        }
        let old_clip = project.clips[self.clip].id;
        let navigator_changed = self.navigator(ui, project);
        let mut changed = self.transport(ui, &mut project.clips[self.clip]);
        changed |= navigator_changed;
        let selected = selected.filter(|_| old_clip == project.clips[self.clip].id);
        let clip = &mut project.clips[self.clip];
        if !self.channel.supports_image()
            && selected.is_some_and(|id| {
                clip.drawing_at(self.frame)
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .is_some_and(|o| o.image.is_some())
            })
        {
            self.channel = Channel::Position;
        }
        self.sync_key_selection(clip, selected, selected_bone);
        changed |= self.drawing_actions(ui, clip, selected, active_layer);
        let clip_length = clip.frames;
        if self.graph_open {
            let mut open = true;
            egui::Window::new("Animation curve")
                .open(&mut open)
                .default_size([324., 196.])
                .min_size([220., 104.])
                .scroll(true)
                .show(ui.ctx(), |ui| {
                    if let Some(obj) = selected.and_then(|id| {
                        clip.drawing_at_mut(self.frame)
                            .objects
                            .iter_mut()
                            .find(|o| o.id == id)
                    }) {
                        egui::ComboBox::from_label("Property")
                            .selected_text(self.channel.label())
                            .show_ui(ui, |ui| {
                                for c in Channel::ALL
                                    .into_iter()
                                    .filter(|c| obj.image.is_none() || c.supports_image())
                                {
                                    ui.selectable_value(&mut self.channel, c, c.label());
                                }
                            });
                        changed |= crate::curve_editor::show(
                            ui,
                            obj,
                            self.channel,
                            self.frame,
                            clip_length,
                        );
                    } else {
                        ui.label("Select an object to edit its animation curve.");
                    }
                });
            self.graph_open = open;
        }

        if self.onion_settings_open {
            let mut open = true;
            egui::Window::new("Onion skin settings")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.checkbox(&mut self.onion, "Show onion skins");
                    self.onion_controls(ui);
                });
            self.onion_settings_open = open;
        }
        changed |= self.ruler(ui, clip, selected, selected_bone);
        if changed {
            self.error = None;
        }
        changed
    }

    fn timeline_scale(&mut self, ui: &mut Ui) {
        // Keep the compact default and both ends of the original zoom range.
        let ratio = self.frame_width / DEFAULT_FRAME_WIDTH;
        let factor = |compact| {
            if compact {
                (DEFAULT_FRAME_WIDTH / MIN_FRAME_WIDTH).ln() / 95.
            } else {
                (MAX_FRAME_WIDTH / DEFAULT_FRAME_WIDTH).ln() / 100.
            }
        };
        let mut percent = (100. + ratio.ln() / factor(ratio < 1.)).round() as u32;
        // This toolbar lays out right-to-left. Keeping the rail to the right
        // of the number also gives a drag on the number room to reach 200%.
        let rail_width = (ui.available_width() - 136.).clamp(48., 112.);
        ui.spacing_mut().slider_width = rail_width;
        if ui
            .add(
                egui::Slider::new(&mut percent, 5..=200)
                    .text("Timeline scale")
                    .suffix("%")
                    .drag_value_speed(195. / f64::from(rail_width * 0.8)),
            )
            .on_hover_text("Drag the slider or click the percentage to type a scale.")
            .changed()
        {
            self.set_frame_width(
                DEFAULT_FRAME_WIDTH * ((percent as f32 - 100.) * factor(percent < 100)).exp(),
            );
            self.reveal_frame = true;
        }
    }

    fn onion_controls(&mut self, ui: &mut Ui) {
        use crate::keymap::{self, Action};
        egui::Grid::new("onion-directions").show(ui, |ui| {
            for (label, hint, action, count, tint) in [
                (
                    "Previous",
                    "Cycle previous poses",
                    Action::PreviousSkins,
                    &mut self.previous_onions,
                    &mut self.previous_tint,
                ),
                (
                    "Next",
                    "Cycle upcoming poses",
                    Action::NextSkins,
                    &mut self.next_onions,
                    &mut self.next_tint,
                ),
            ] {
                ui.label(label);
                let suffix = if *count == 1 { " pose" } else { " poses" };
                ui.add(
                    egui::DragValue::new(count)
                        .range(0..=MAX_ONIONS)
                        .suffix(suffix),
                )
                .on_hover_ui(|ui| {
                    ui.label(format!(
                        "{}. Zero hides this side.",
                        keymap::hint(ui.ctx(), hint, action)
                    ));
                });
                crate::color_picker::button(ui, tint).on_hover_text("Tint and opacity");
                ui.end_row();
            }
        });
    }

    fn key_menu(&mut self, ui: &mut Ui, clip: &mut Clip, selected: Option<u64>) -> bool {
        ui.menu_button(format!("Keying: {}", self.key_mode.label()), |ui| {
            for mode in [KeyMode::Tracked, KeyMode::Manual, KeyMode::Auto] {
                if ui
                    .selectable_value(&mut self.key_mode, mode, mode.label())
                    .on_hover_text(mode.hint())
                    .clicked()
                {
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        ui.checkbox(&mut self.expanded, "Show keyed tracks");
        let object = selected.and_then(|id| {
            clip.drawing_at(self.frame)
                .objects
                .iter()
                .find(|o| o.id == id)
        });
        let image = object.is_some_and(|o| o.image.is_some());
        egui::Grid::new("property-picker")
            .num_columns(2)
            .show(ui, |ui| {
                for (i, channel) in Channel::ALL
                    .into_iter()
                    .filter(|c| !image || c.supports_image())
                    .enumerate()
                {
                    if ui
                        .selectable_value(&mut self.channel, channel, channel.label())
                        .clicked()
                    {
                        ui.close_menu();
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        let t = self.frame;
        let Some(object) = object else {
            return false;
        };
        let id = object.id;
        let morph_label = if object.guided_morph_span(t).is_some() {
            "Edit morph…"
        } else {
            "Guided morph…"
        };
        if ui
            .add_enabled(
                object.can_guided_morph()
                    && crate::commands::is_editable(clip.drawing_at(t), object.layer),
                Button::new(morph_label),
            )
            .on_disabled_hover_text("Select an editable single-outline path.")
            .clicked()
        {
            self.guided_morph_request = Some(id);
            ui.close_menu();
        }
        let value = self.channel.get(object, t);
        let frames = self.channel.frames(object);
        let mut muted = self.channel.is_muted(object);
        let mute_changed = ui
            .add_enabled(
                !frames.is_empty(),
                egui::Checkbox::new(&mut muted, "Mute track"),
            )
            .changed();
        let mut clear = false;
        if !frames.is_empty() {
            ui.menu_button("Delete track…", |ui| {
                if ui.button(format!("Delete {} keys", frames.len())).clicked() {
                    clear = true;
                    ui.close_menu();
                }
            });
        }
        let unsupported = self.channel == Channel::Geometry
            && !self
                .channel
                .compatible(object, &self.channel.sample(object, t));
        if unsupported {
            ui.small("Use an editable path for Geometry keys. Guided morph can match different node counts on a single outline.");
        }
        let mut interpolation = object.interpolation;
        let old_interpolation = interpolation;
        if ui
            .add_enabled(value.is_some(), Button::new("Copy key"))
            .clicked()
        {
            self.copied_key = value.clone().map(|v| (self.channel, v));
            ui.close_menu();
        }
        let paste = self
            .copied_key
            .clone()
            .filter(|(c, _)| *c == self.channel)
            .map(|(_, v)| v);
        let do_paste = ui
            .add_enabled(paste.is_some(), Button::new("Paste key"))
            .clicked();
        let remove = ui
            .add_enabled(value.is_some(), Button::new("Remove key"))
            .clicked();
        ui.separator();
        ui.label("Object interpolation");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut interpolation, Interpolation::Linear, "Linear");
            ui.selectable_value(&mut interpolation, Interpolation::Hold, "Hold");
        });
        if do_paste || remove || mute_changed || clear || old_interpolation != interpolation {
            let object = clip
                .drawing_at_mut(t)
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .expect("selected object exists");
            object.interpolation = interpolation;
            if mute_changed || clear {
                self.channel.set_muted(object, muted && !clear);
            }
            if clear {
                for f in frames {
                    self.channel.remove(object, f);
                }
            }
            if remove {
                self.channel.remove(object, t);
            }
            if do_paste {
                self.channel.set(object, t, paste.unwrap());
            }
            ui.close_menu();
            true
        } else {
            false
        }
    }
}
fn unique_name(project: &AnimationProject, set: u64, stem: &str) -> String {
    let mut label = stem.to_owned();
    for index in 2.. {
        if !project
            .clips
            .iter()
            .any(|c| c.set_id == set && c.name.to_lowercase() == label.to_lowercase())
        {
            return label;
        }
        label = format!("{stem} {index}");
    }
    unreachable!()
}

/// Base the playhead on elapsed time so a delayed redraw skips to the correct tick.
pub(crate) fn playback_sample(
    first: u32,
    elapsed: Duration,
    frames: u32,
    fps: u32,
    looping: bool,
) -> (u32, Option<Duration>) {
    let rate = u128::from(fps.max(1));
    let second = 1_000_000_000;
    let phase = elapsed.as_nanos() * rate;
    let tick = u128::from(first) + phase / second;
    let frames = u128::from(frames.max(1));
    if !looping && tick >= frames {
        return ((frames - 1) as u32, None);
    }
    let remaining = (second - phase % second).div_ceil(rate);
    (
        (tick % frames) as u32,
        Some(Duration::from_nanos(remaining as u64)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_handles_delayed_redraws_and_one_shot_clips() {
        assert_eq!(
            playback_sample(3, Duration::from_millis(725), 5, 12, true),
            (1, Some(Duration::from_millis(25)))
        );
        assert_eq!(
            playback_sample(0, Duration::from_secs(7), 8, 12, false),
            (7, None)
        );
        assert_eq!(
            playback_sample(0, Duration::from_millis(784), 8, 10, false),
            (7, Some(Duration::from_millis(16)))
        );
    }
    #[test]
    fn a_delayed_last_tick_reveals_the_end_of_one_shot_playback() {
        let mut clip = AnimationProject::default().clips.remove(0);
        clip.set_length(1_000).unwrap();
        clip.looping = false;
        let mut timeline = Timeline {
            frame: 990,
            playback: Some((Instant::now() - Duration::from_secs(10), 990)),
            reveal_frame: false,
            ..Timeline::default()
        };
        timeline.advance_clip(&egui::Context::default(), &clip);
        assert_eq!(timeline.frame, 999);
        assert!(!timeline.is_playing());
        assert!(timeline.reveal_frame);
    }
    #[test]
    fn inserting_a_key_preserves_the_interpolated_pose() {
        let timeline = Timeline {
            frame: 2,
            ..Timeline::default()
        };
        let mut doc = Document::default();
        let id = doc.add_rectangle([0.; 2], [1.; 2], [255; 4]);
        doc.objects[0].keys.insert(0, [2.; 2]);
        doc.objects[0].keys.insert(4, [6.; 2]);
        assert!(timeline.key_selected_track(&mut doc, Some(id)));
        assert_eq!(doc.objects[0].keys[&2], [4.; 2]);
        assert!(!timeline.key_selected_track(&mut doc, Some(id)));
    }
}
