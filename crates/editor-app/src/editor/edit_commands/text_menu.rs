use super::{Command, Command::*};
use eframe::egui::{self, Context, Event, Key, Modifiers};

pub(in crate::editor) struct TextMenu {
    pub(in crate::editor) button: egui::Rect,
    pub(in crate::editor) popup: egui::Rect,
    pub(in crate::editor) field: Option<egui::Id>,
}

impl Default for TextMenu {
    fn default() -> Self {
        Self {
            button: egui::Rect::NOTHING,
            popup: egui::Rect::NOTHING,
            field: None,
        }
    }
}

impl TextMenu {
    pub(in crate::editor) fn prepare(&mut self, ctx: &Context) -> bool {
        self.field = crate::keymap::focused_text_field(ctx);
        self.field.is_some()
            && ctx.input(|i| {
                i.pointer
                    .interact_pos()
                    .is_some_and(|p| self.button.contains(p) || self.popup.contains(p))
                    && (i.pointer.any_down() || i.pointer.any_released())
            })
    }
}

pub(super) fn text_enabled(ctx: &Context, id: egui::Id, command: Command) -> bool {
    let selection = || {
        egui::TextEdit::load_state(ctx, id)
            .and_then(|s| s.cursor.char_range())
            .is_some_and(|range| range.primary.index != range.secondary.index)
    };
    match command {
        PasteStyle | Duplicate | Grouping(_) => false,
        Cut | Copy | Deselect => selection(),
        _ => true,
    }
}

pub(super) fn run_text_command(
    ctx: &Context,
    id: egui::Id,
    command: Command,
    modifiers: Modifiers,
    key: Key,
) {
    ctx.memory_mut(|m| m.request_focus(id));
    let event = match command {
        Cut => Event::Cut,
        Copy => Event::Copy,
        Paste => {
            super::clipboard::request_text_paste(ctx);
            return;
        }
        Deselect => {
            if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
                if let Some(range) = state.cursor.char_range() {
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(range.primary)));
                    state.store(ctx, id);
                }
            }
            return;
        }
        Duplicate | PasteStyle | Grouping(_) => return,
        _ => Event::Key {
            key,
            modifiers,
            pressed: true,
            repeat: false,
            physical_key: None,
        },
    };
    ctx.input_mut(|i| i.events.push(event));
}
