use super::Editor;
use document_core::RasterImage;
use eframe::egui::{Context, Event, Key, Modifiers};
use std::sync::Arc;

type ReadImage = fn() -> Result<Option<Arc<RasterImage>>, String>;

pub(super) struct ImageClipboard {
    pub(super) read: ReadImage,
    command_down: bool,
    plain_command: bool,
    handled_press: bool,
    pub(super) released_paste: bool,
}

impl Default for ImageClipboard {
    fn default() -> Self {
        Self {
            read: || Ok(None),
            command_down: false,
            plain_command: false,
            handled_press: false,
            released_paste: false,
        }
    }
}

impl ImageClipboard {
    pub(super) fn native() -> Self {
        Self {
            read: read_system_image,
            ..Self::default()
        }
    }

    // egui-winit consumes the native Ctrl+V press even when the clipboard has
    // no text and sends no Paste event. In that case the release identifies
    // the gesture. Observe before widgets consume events, dispatch on canvas only.
    pub(super) fn observe(&mut self, ctx: &Context) {
        self.released_paste = false;
        ctx.input(|i| {
            if !i.focused || i.pointer.any_pressed() {
                self.handled_press = false;
                self.plain_command = false;
            }
            if i.modifiers.command && !self.command_down {
                self.handled_press = false;
                self.plain_command = i.modifiers.matches_exact(Modifiers::COMMAND);
            }
            if i.modifiers.shift || i.modifiers.alt {
                self.plain_command = false;
            }
            self.command_down = i.modifiers.command;
            for event in &i.events {
                match event {
                    Event::Paste(_) => self.handled_press = true,
                    Event::Key {
                        key: Key::V | Key::Paste,
                        pressed: true,
                        ..
                    } => {
                        self.handled_press = true;
                    }
                    Event::Key {
                        key: key @ (Key::V | Key::Paste | Key::Insert),
                        pressed: false,
                        modifiers,
                        ..
                    } => {
                        let native = match key {
                            Key::Paste => true,
                            Key::Insert => {
                                cfg!(windows) && modifiers.matches_exact(Modifiers::SHIFT)
                            }
                            _ => {
                                modifiers.matches_exact(Modifiers::COMMAND)
                                    || (!modifiers.any() && self.plain_command)
                            }
                        };
                        self.released_paste |= i.focused && native && !self.handled_press;
                        self.handled_press = false;
                        self.plain_command = false;
                    }
                    Event::Key { pressed: true, .. } | Event::WindowFocused(false) => {
                        self.plain_command = false;
                    }
                    _ => {}
                }
            }
        });
    }
}

fn read_system_image() -> Result<Option<Arc<RasterImage>>, String> {
    let data = match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
        Ok(data) => data,
        Err(arboard::Error::ContentNotAvailable) => return Ok(None),
        Err(e) => return Err(format!("Could not read the clipboard image: {e}")),
    };
    let dimension = |n| u32::try_from(n).map_err(|_| "The clipboard image is too large.");
    RasterImage::from_rgba(
        dimension(data.width)?,
        dimension(data.height)?,
        data.bytes.into_owned(),
    )
    .map(|image| Some(Arc::new(image)))
}

impl Editor {
    pub(super) fn try_paste_image(&mut self) -> bool {
        match (self.image_clipboard.read)() {
            Ok(Some(image)) => {
                match self.paste_raster_image(image) {
                    Ok(()) => self.status = "Pasted image on the active layer.".into(),
                    Err(error) => self.report_error("Could not paste image", error.to_string()),
                }
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.report_error("Could not read clipboard image", e);
                false
            }
        }
    }
}
