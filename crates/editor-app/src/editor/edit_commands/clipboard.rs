use super::{shortcut, Clipboard, Command, Command::*};
use crate::keymap;
use eframe::egui::{self, Context, Event, Key, Modifiers};
use std::hash::{BuildHasher, RandomState};

pub(crate) enum ClipboardInput {
    Copy,
    Cut,
    RequestPaste { style: bool },
    Paste { text: String, style: bool },
}

pub(crate) fn request_paste(ctx: &Context, style: bool) {
    let request = egui::Id::new("paste style request");
    let command_released = !ctx.input(|i| i.modifiers.command);
    ctx.data_mut(|data| {
        data.insert_temp(
            request,
            PasteRequest {
                active: true,
                command_released,
                style,
            },
        );
    });
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

pub(super) fn request_text_paste(ctx: &Context) {
    // The field owns the native reply. An earlier artwork paste may still
    // be waiting if the system clipboard was empty.
    ctx.data_mut(|d| d.remove::<PasteRequest>(egui::Id::new("paste style request")));
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

#[derive(Clone, Copy)]
struct PasteRequest {
    active: bool,
    style: bool,
    // A newly pressed Ctrl/Cmd after the original gesture identifies native
    // keyboard paste even when the integration only delivers Event::Paste.
    command_released: bool,
}

// Native clipboard reads are asynchronous. A canceled style response must never
// fall through to ordinary Paste in a different field or selection.
pub(in crate::editor) fn guard_paste_reply(ctx: &Context) {
    let id = egui::Id::new("paste style request");
    let Some(mut request) = ctx.data(|d| d.get_temp::<PasteRequest>(id)) else {
        return;
    };
    let mut clear = false;
    ctx.input_mut(|input| {
        if !input.focused || input.pointer.any_pressed() { request.active = false; }
        request.command_released |= !input.modifiers.command;
        let new_command = request.command_released && input.modifiers.command;
        let fresh_paste = new_command || input.events.iter().any(|e| matches!(e, Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command));
        input.events.retain(|e| {
            match e {
                Event::Text(_) | Event::Cut | Event::Copy | Event::WindowFocused(false)
                | Event::Key { pressed: true, .. } => request.active = false,
                Event::Paste(_) if !request.active || new_command => {
                    clear = true;
                    return fresh_paste;
                }
                _ => {}
            }
            true
        });
    });
    ctx.data_mut(|d| {
        if clear {
            d.remove::<PasteRequest>(id)
        } else {
            d.insert_temp(id, request)
        }
    });
}

pub(crate) fn clipboard_input(ctx: &Context, style_requested: bool) -> Vec<ClipboardInput> {
    guard_paste_reply(ctx);
    let request = egui::Id::new("paste style request");
    let pending = ctx.data(|data| data.get_temp::<PasteRequest>(request));
    let bindings = keymap::active(ctx);
    let mut received = false;
    let actions = ctx.input_mut(|i| {
        let mut actions = Vec::new();
        if !i.focused || i.pointer.any_down() {
            return actions;
        }
        let native: Vec<_> = i.events.iter().filter_map(native_command).collect();
        let raw = i
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => shortcut(&bindings, *key, *modifiers),
                _ => None,
            })
            .collect::<Vec<_>>();
        let raw_paste = raw.iter().find(|c| matches!(c, Paste | PasteStyle));
        // egui-winit translates the native clipboard chord into its result.
        // Its key release can carry the original modifiers after Ctrl is up.
        let native_mods = i.events.iter().find_map(|e| match e {
            Event::Key {
                key: Key::V,
                pressed: false,
                modifiers,
                ..
            } if modifiers.command => Some(*modifiers),
            _ => None,
        });
        let native_style = native_mods
            .unwrap_or(i.modifiers)
            .matches_exact(Modifiers::COMMAND | Modifiers::SHIFT);
        let mut requested = pending.filter(|r| r.active).map(|r| r.style);
        if style_requested {
            requested = Some(true);
        }
        i.events.retain_mut(|event| {
            let action = match event {
                Event::Copy if bindings.is_default(keymap::Action::Copy) || raw.contains(&Copy) => {
                    ClipboardInput::Copy
                }
                Event::Cut if bindings.is_default(keymap::Action::Cut) || raw.contains(&Cut) => {
                    ClipboardInput::Cut
                }
                Event::Copy | Event::Cut => return false,
                Event::Paste(text) => {
                    received = true;
                    let native_enabled = bindings.is_default(if native_style {
                        keymap::Action::PasteStyle
                    } else {
                        keymap::Action::Paste
                    });
                    let resolved = if let Some(command) = raw_paste {
                        Some(*command == PasteStyle)
                    } else if native_mods.is_some() {
                        native_enabled.then_some(native_style)
                    } else {
                        requested.or(native_enabled.then_some(native_style))
                    };
                    let Some(style) = resolved else {
                        return false;
                    };
                    ClipboardInput::Paste {
                        text: std::mem::take(text),
                        style,
                    }
                }
                Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    repeat,
                    ..
                } => {
                    let Some(command @ (Cut | Copy | Paste | PasteStyle)) =
                        shortcut(&bindings, *key, *modifiers)
                    else {
                        requested = None;
                        return true;
                    };
                    if *repeat
                        || native.contains(&if command == PasteStyle {
                            Paste
                        } else {
                            command
                        })
                    {
                        return false;
                    }
                    match command {
                        Cut => ClipboardInput::Cut,
                        Copy => ClipboardInput::Copy,
                        _ => ClipboardInput::RequestPaste {
                            style: command == PasteStyle,
                        },
                    }
                }
                Event::Text(_) | Event::WindowFocused(false) => {
                    requested = None;
                    return true;
                }
                _ => return true,
            };
            requested = None;
            actions.push(action);
            false
        });
        actions
    });
    if pending.is_some() && received {
        ctx.data_mut(|d| d.remove::<PasteRequest>(request));
    }
    actions
}

impl Clipboard {
    pub(crate) fn publish(&self, ctx: &Context) -> bool {
        let id = egui::Id::new("local artwork clipboard");
        if let Some(text) = self.encode() {
            ctx.data_mut(|d| d.remove::<(String, Clipboard)>(id));
            ctx.copy_text(text);
            true
        } else {
            let token = format!(
                "Phyro/session-objects/1/{:016x}",
                RandomState::new().hash_one("local clipboard")
            );
            ctx.copy_text(token.clone());
            ctx.data_mut(|d| d.insert_temp(id, (token, self.clone())));
            false
        }
    }

    pub(crate) fn from_system(ctx: &Context, text: &str) -> Option<Clipboard> {
        if text.starts_with("Phyro/session-objects/1/") {
            ctx.data(|d| {
                d.get_temp::<(String, Clipboard)>(egui::Id::new("local artwork clipboard"))
            })
            .filter(|(token, _)| token == text)
            .map(|(_, clipboard)| clipboard)
        } else {
            Self::decode(text)
        }
    }
}

fn native_command(event: &Event) -> Option<Command> {
    match event {
        Event::Cut => Some(Cut),
        Event::Copy => Some(Copy),
        Event::Paste(_) => Some(Paste),
        _ => None,
    }
}
