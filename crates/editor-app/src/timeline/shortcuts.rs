use super::Timeline;
use crate::keymap::{self, Action};
use document_core::Clip;
use eframe::egui::{Context, Event};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnimationAction {
    Previous,
    Next,
    Keyframe,
    Blank,
    Duplicate,
    Onion,
    OnionSettings,
    PreviousSkins,
    NextSkins,
}

impl AnimationAction {
    pub(crate) fn from_input(ctx: &Context) -> Vec<Self> {
        let map = keymap::active(ctx);
        ctx.input(|input| {
            if !input.focused || input.pointer.any_down() {
                return Vec::new();
            }
            input
                .events
                .iter()
                .filter_map(|event| {
                    let Event::Key {
                        key,
                        pressed: true,
                        repeat,
                        modifiers,
                        ..
                    } = event
                    else {
                        return None;
                    };
                    match map.resolve(*key, *modifiers)? {
                        Action::PreviousFrame => Some(Self::Previous),
                        Action::NextFrame => Some(Self::Next),
                        _ if *repeat => None,
                        Action::Keyframe => Some(Self::Keyframe),
                        Action::BlankFrame => Some(Self::Blank),
                        Action::DuplicateFrames => Some(Self::Duplicate),
                        Action::Onion => Some(Self::Onion),
                        Action::OnionSettings => Some(Self::OnionSettings),
                        Action::PreviousSkins => Some(Self::PreviousSkins),
                        Action::NextSkins => Some(Self::NextSkins),
                        _ => None,
                    }
                })
                .collect()
        })
    }
}

impl Timeline {
    pub(super) fn step_frame(&mut self, clip: &Clip, forward: bool) {
        let tick = if forward {
            (self.frame + 1).min(clip.frames - 1)
        } else {
            self.frame.saturating_sub(1)
        };
        self.select_frame(tick);
        self.reveal_frame = true;
    }

    pub(crate) fn run_action(
        &mut self,
        action: AnimationAction,
        clip: &mut Clip,
        selected: Option<u64>,
        active_layer: u64,
    ) -> bool {
        use AnimationAction::*;
        self.error = None;
        match action {
            Previous | Next => self.step_frame(clip, action == Next),
            Keyframe => {
                self.stop();
                if selected.is_some() {
                    let changed =
                        self.key_selected_track(clip.drawing_at_mut(self.frame), selected);
                    self.select_current_key(clip, selected);
                    return changed;
                }
            }
            Blank | Duplicate => {
                self.stop();
                let result = if action == Blank {
                    clip.blank_layer_frame(self.frame, active_layer)
                } else {
                    clip.duplicate_layer_frame(self.frame, active_layer)
                };
                self.reveal_frame = true;
                return self.frame_edit(result);
            }
            Onion => self.onion = !self.onion,
            OnionSettings => self.onion_settings_open = !self.onion_settings_open,
            PreviousSkins | NextSkins => {
                let count = if action == PreviousSkins {
                    &mut self.previous_onions
                } else {
                    &mut self.next_onions
                };
                *count = (*count + 1) % (super::MAX_ONIONS + 1);
                self.onion = true;
            }
        }
        false
    }
}
