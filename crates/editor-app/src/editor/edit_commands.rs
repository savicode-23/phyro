use super::{commands, Clipboard, Editor, Tool};
use crate::keymap::{self, Action, Keymap};
use commands::GroupAction;
use eframe::egui::{self, Context, Event, Key, KeyboardShortcut, Modifiers};
mod clipboard;
mod text_menu;
pub(super) use clipboard::guard_paste_reply;
pub(crate) use clipboard::{clipboard_input, request_paste, ClipboardInput};
pub(super) use text_menu::TextMenu;
use text_menu::{run_text_command, text_enabled};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PasteStyle,
    Duplicate,
    Delete,
    Grouping(GroupAction),
    SelectAll,
    Deselect,
}

use Command::*;

enum TimelineTarget {
    Frames { count: u32 },
    Keys { selected: bool },
}

// Text fields use egui's native editing keys. The canvas uses the user's
// active keymap even when the same command is invoked through this menu.
const COMMANDS: [(Command, &str, Modifiers, Key); 12] = [
    (Undo, "Undo", Modifiers::COMMAND, Key::Z),
    (
        Redo,
        "Redo",
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
        Key::Z,
    ),
    (Cut, "Cut", Modifiers::COMMAND, Key::X),
    (Copy, "Copy", Modifiers::COMMAND, Key::C),
    (Paste, "Paste", Modifiers::COMMAND, Key::V),
    (
        PasteStyle,
        "Paste style",
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
        Key::V,
    ),
    (Duplicate, "Duplicate", Modifiers::COMMAND, Key::D),
    (Delete, "Delete", Modifiers::NONE, Key::Delete),
    (
        Grouping(GroupAction::Group),
        "Group",
        Modifiers::COMMAND,
        Key::G,
    ),
    (
        Grouping(GroupAction::Ungroup),
        "Ungroup",
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
        Key::G,
    ),
    (SelectAll, "Select All", Modifiers::COMMAND, Key::A),
    (Deselect, "Deselect", Modifiers::NONE, Key::Escape),
];

impl Command {
    pub(super) fn action(self) -> Action {
        match self {
            Undo => Action::Undo,
            Redo => Action::Redo,
            Cut => Action::Cut,
            Copy => Action::Copy,
            Paste => Action::Paste,
            PasteStyle => Action::PasteStyle,
            Duplicate => Action::Duplicate,
            Delete => Action::Delete,
            Grouping(GroupAction::Group) => Action::Group,
            Grouping(GroupAction::Ungroup) => Action::Ungroup,
            SelectAll => Action::SelectAll,
            Deselect => Action::Deselect,
        }
    }
}

fn shortcut(bindings: &Keymap, key: Key, modifiers: Modifiers) -> Option<Command> {
    let action = bindings.resolve(key, modifiers)?;
    COMMANDS
        .iter()
        .map(|row| row.0)
        .find(|cmd| cmd.action() == action)
}

impl Editor {
    pub(super) fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let bindings = keymap::active(ui.ctx());
        for (command, label, modifiers, key) in COMMANDS {
            if matches!(command, Grouping(_)) {
                continue;
            }
            if matches!(command, Cut | Duplicate | SelectAll) {
                ui.separator();
            }
            let keys = if self.text_menu.field.is_some() {
                ui.ctx()
                    .format_shortcut(&KeyboardShortcut::new(modifiers, key))
            } else {
                bindings.labels(ui.ctx(), command.action())
            };
            if ui
                .add_enabled(
                    self.text_menu.field.map_or_else(
                        || self.edit_enabled(ui.ctx(), command),
                        |id| text_enabled(ui.ctx(), id, command),
                    ),
                    egui::Button::new(label).shortcut_text(keys),
                )
                .clicked()
            {
                ui.close_menu();
                if let Some(id) = self.text_menu.field {
                    run_text_command(ui.ctx(), id, command, modifiers, key);
                } else {
                    self.run_edit_command(ui.ctx(), command);
                }
            }
        }
    }

    fn timeline_target(&self) -> Option<TimelineTarget> {
        if !self.show_timeline {
            return None;
        }
        let clip = &self.project.clips[self.timeline.clip];
        if let Some(frames) = self.timeline.focused_frames(clip) {
            return Some(TimelineTarget::Frames {
                count: frames.count(),
            });
        }
        self.timeline
            .focused_keys(clip, self.single(), self.canvas.rig.selected_bone())
            .map(|selected| TimelineTarget::Keys { selected })
    }

    pub(super) fn edit_enabled(&self, ctx: &Context, command: Command) -> bool {
        if self.exports.is_open()
            || self.library.is_editing()
            || self.crash_reports.is_open()
            || self.artboard_size.is_some()
            || self.text_dialog.is_some()
            || self.canvas.text_active()
        {
            return false;
        }
        match command {
            Undo => self
                .canvas
                .pen_history_available(false)
                .unwrap_or_else(|| self.can_undo()),
            Redo => self
                .canvas
                .pen_history_available(true)
                .unwrap_or_else(|| self.history.can_redo()),
            _ if self.canvas.path_in_progress() || self.canvas.rig.placing() => false,
            Grouping(action) => action.enabled(self.scene(), &self.selection),
            _ if self.timeline.is_playing() => false,
            Cut | Copy | Duplicate => !self.selection.is_empty(),
            Deselect => match self.timeline_target() {
                Some(TimelineTarget::Frames { count }) => count > 0,
                Some(TimelineTarget::Keys { selected }) => selected,
                None => !self.selection.is_empty(),
            },
            Delete => match self.timeline_target() {
                Some(TimelineTarget::Frames { count }) => {
                    count > 0 && count < self.project.clips[self.timeline.clip].frames
                }
                Some(TimelineTarget::Keys { selected }) => selected,
                None if self.tool == Tool::Node => crate::path_editor::can_delete_selected_nodes(
                    ctx,
                    self.scene(),
                    &self.selection,
                    self.timeline.frame,
                    self.timeline.key_mode,
                ),
                None => !self.selection.is_empty(),
            },
            Paste => commands::is_editable(self.scene(), self.style.layer),
            PasteStyle => commands::has_style_target(self.scene(), &self.selection),
            SelectAll => !self.scene().objects.is_empty(),
        }
    }

    fn copy_selection(&mut self, ctx: &Context) -> bool {
        let mut copied = Clipboard::default();
        copied.copy_project(
            &self.project,
            self.timeline.clip,
            self.timeline.frame,
            &self.selection,
        );
        if copied.is_empty() {
            return false;
        }
        if !copied.publish(ctx) {
            self.status = "Selection copied for use in this Phyro window.".into();
        }
        self.clipboard = copied;
        true
    }

    fn delete_selection(&mut self) {
        self.finish_edit();
        self.canvas.cancel_drag();
        let drawing = self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
        commands::delete(drawing, &self.selection);
        self.selection.clear();
        self.timeline.leave_grid_selection();
        self.finish_edit();
    }

    fn delete_frames(&mut self) {
        self.finish_edit();
        let index = self.timeline.clip;
        let clip = &self.project.clips[index];
        let Some(selection) = self.timeline.focused_frames(clip) else {
            return;
        };
        let ranges: Vec<_> = selection.ranges().collect();
        let Some(&(first, last)) = ranges.first() else {
            return;
        };
        // The playhead lands at the first cut. Check the identity of that drawing
        // before later drawings (which may reuse object IDs) are renumbered.
        let survivor = if last + 1 < clip.frames {
            last + 1
        } else {
            first.saturating_sub(1)
        };
        let clear_selection = self.selection_crossed_drawing((index, clip.exposure(survivor).0));
        if self
            .timeline
            .delete_frames(&mut self.project.clips[index], &ranges)
        {
            self.note(super::Action::Timeline);
            self.canvas.cancel_drag();
            if clear_selection {
                self.selection.clear();
            }
            self.tidy_selection();
            self.sync_paint();
            self.finish_edit();
        }
    }

    pub(super) fn run_edit_command(&mut self, ctx: &Context, command: Command) {
        if !self.edit_enabled(ctx, command) {
            return;
        }
        match command {
            Undo | Redo => {
                let redo = command == Redo;
                if self.canvas.step_pen_history(redo) {
                    self.note(if redo {
                        super::Action::Redo
                    } else {
                        super::Action::Undo
                    });
                    ctx.request_repaint();
                } else {
                    self.restore(redo);
                }
            }
            Copy | Cut => {
                if self.copy_selection(ctx) && command == Cut {
                    self.delete_selection();
                }
            }
            Paste if self.try_paste_image() => {}
            Paste | PasteStyle => request_paste(ctx, command == PasteStyle),
            Duplicate => self.duplicate_selection(),
            Delete => match self.timeline_target() {
                Some(TimelineTarget::Frames { .. }) => self.delete_frames(),
                Some(TimelineTarget::Keys { .. }) => {
                    self.finish_edit();
                    let id = self.single();
                    self.timeline
                        .remove_keys(&mut self.project.clips[self.timeline.clip], id);
                    self.finish_edit();
                }
                None if self.tool == Tool::Node => {
                    if self.pending_edit {
                        self.finish_edit();
                    }
                    let frame = self.timeline.frame;
                    let drawing = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
                    if crate::path_editor::delete_selected_nodes(
                        ctx,
                        drawing,
                        &self.selection,
                        frame,
                        self.timeline.key_mode,
                    ) {
                        self.note(super::Action::EditPath);
                        self.status.clear();
                        self.finish_edit();
                    }
                }
                None => self.delete_selection(),
            },
            Grouping(action) => {
                self.timeline.stop();
                self.finish_edit();
                self.canvas.cancel_drag();
                let doc =
                    self.project.clips[self.timeline.clip].drawing_at_mut(self.timeline.frame);
                action.apply(doc, &mut self.selection);
                self.timeline.leave_grid_selection();
                self.finish_edit();
            }
            SelectAll => {
                let doc = self.scene();
                let ids = doc
                    .objects
                    .iter()
                    .filter(|o| document_core::arrange::is_object_editable(doc, o))
                    .map(|o| o.id)
                    .collect();
                self.select_artwork(ids);
            }
            Deselect if self.timeline_target().is_some() => self.timeline.deselect_grid(),
            Deselect => self.selection.clear(),
        }
    }

    pub(super) fn edit_shortcuts(&mut self, ctx: &Context) -> bool {
        let clipboard = clipboard_input(ctx, false);
        let mut handled = !clipboard.is_empty();
        if std::mem::take(&mut self.image_clipboard.released_paste)
            && keymap::active(ctx).is_default(Action::Paste)
            && self.edit_enabled(ctx, Paste)
        {
            handled |= self.try_paste_image();
        }
        for action in clipboard {
            match action {
                ClipboardInput::Copy => self.run_edit_command(ctx, Copy),
                ClipboardInput::Cut => self.run_edit_command(ctx, Cut),
                ClipboardInput::RequestPaste { style } => {
                    self.run_edit_command(ctx, if style { PasteStyle } else { Paste })
                }
                ClipboardInput::Paste { text, style } => {
                    if !self.edit_enabled(ctx, if style { PasteStyle } else { Paste }) {
                        continue;
                    }
                    let Some(decoded) = Clipboard::from_system(ctx, &text) else {
                        if !style && Clipboard::strip_format_prefix(&text).is_none() {
                            self.try_paste_image();
                        }
                        continue;
                    };
                    self.clipboard = decoded;
                    if style {
                        if !self.clipboard.has_style() {
                            self.status = "Copy a shape or text object to paste its style.".into();
                            continue;
                        }
                        self.finish_edit();
                        let frame = self.timeline.frame;
                        let drawing = self.project.clips[self.timeline.clip].drawing_at_mut(frame);
                        if self.clipboard.paste_style(
                            drawing,
                            &self.selection,
                            frame,
                            self.style.key_mode,
                        ) != 0
                        {
                            if let Some(object) = self.selection.iter().rev().find_map(|id| {
                                drawing.objects.iter().find(|object| {
                                    object.id == *id && crate::commands::has_vector_paint(object)
                                })
                            }) {
                                self.style.read_paint(object, frame);
                                self.path_paints.adopt_style(&self.style);
                            }
                            self.finish_edit();
                        }
                    } else {
                        self.paste();
                    }
                }
            }
        }
        let bindings = keymap::active(ctx);
        let commands: Vec<_> = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::Key {
                        key,
                        modifiers,
                        repeat,
                        pressed: true,
                        ..
                    } if input.focused && !input.pointer.any_down() => {
                        shortcut(&bindings, *key, *modifiers)
                            .filter(|cmd| !repeat || matches!(cmd, Undo | Redo))
                    }
                    _ => None,
                })
                .collect()
        });
        handled |= !commands.is_empty();
        for cmd in commands {
            self.run_edit_command(ctx, cmd);
        }
        handled
    }
}

pub(crate) fn grouping_shortcut(ctx: &Context) -> Option<GroupAction> {
    let bindings = keymap::active(ctx);
    ctx.input(|input| {
        input.events.iter().find_map(|event| match event {
            Event::Key {
                key,
                modifiers,
                pressed: true,
                repeat: false,
                ..
            } => match shortcut(&bindings, *key, *modifiers) {
                Some(Grouping(action)) => Some(action),
                _ => None,
            },
            _ => None,
        })
    })
}
