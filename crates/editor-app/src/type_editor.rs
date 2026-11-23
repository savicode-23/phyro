use document_core::typography::{TextAlign, TextStyle, Typeface};
use document_core::Document;
use eframe::egui;

/// The same settings drive inline lettering and the typography dialog. Return
/// whether a numeric field owns the keyboard while the artist edits on canvas.
pub(crate) fn format_controls(ui: &mut egui::Ui, style: &mut TextStyle) -> bool {
    ui.label("Typeface");
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut style.face, Typeface::UbuntuLight, "Ubuntu Light");
        ui.selectable_value(&mut style.face, Typeface::HackRegular, "Hack Regular");
    });
    let numeric = |ui: &mut egui::Ui, label, value: &mut f32, range, suffix| {
        ui.horizontal_wrapped(|ui| {
            let name = ui.label(label);
            ui.add(
                egui::DragValue::new(value)
                    .range(range)
                    .speed(0.1)
                    .suffix(suffix),
            )
            .labelled_by(name.id)
            .has_focus()
        })
        .inner
    };
    let mut editing_number = numeric(ui, "Size", &mut style.size, 0.25..=4096., " px");
    ui.horizontal_wrapped(|ui| {
        ui.label("Align");
        for (value, name) in [
            (TextAlign::Left, "Left"),
            (TextAlign::Center, "Center"),
            (TextAlign::Right, "Right"),
        ] {
            ui.selectable_value(&mut style.align, value, name);
        }
    });
    ui.collapsing("Spacing", |ui| {
        editing_number |= numeric(
            ui,
            "Letter spacing",
            &mut style.letter_spacing,
            -128.0..=1024.,
            " px",
        );
        editing_number |= numeric(ui, "Line height", &mut style.line_height, 0.25..=12., " ×");
    });
    editing_number
}

pub struct TypeEditor {
    target: u64,
    style: TextStyle,
    focus: bool,
    error: String,
    preview: Option<(TextStyle, Result<egui::TextureHandle, String>)>,
}

pub enum Outcome {
    Editing,
    Cancelled,
    Applied(u64),
}

impl TypeEditor {
    pub fn new(target: u64, style: TextStyle) -> Self {
        Self {
            target,
            style,
            focus: true,
            error: String::new(),
            preview: None,
        }
    }

    fn sample(&mut self, ui: &mut egui::Ui) -> bool {
        if self
            .preview
            .as_ref()
            .is_none_or(|(style, _)| style != &self.style)
        {
            let mut doc = Document {
                width: 300,
                height: 60,
                ..Document::default()
            };
            let result = self
                .style
                .clone()
                .insert(&mut doc, 1, [0.; 2], [240, 240, 240, 255])
                .and_then(|_| {
                    let object = &mut doc.objects[0];
                    let [w, h] = object.size;
                    let scale = (290.0 / w).min(54.0 / h).min(1.);
                    object.size = [w * scale, h * scale];
                    object.position = [(300. - object.size[0]) * 0.5, (60. - object.size[1]) * 0.5];
                    renderer::render_scaled(&doc, 0, ui.ctx().pixels_per_point().clamp(1., 2.))
                })
                .map(|r| {
                    let image = egui::ColorImage::from_rgba_premultiplied(
                        [r.width() as usize, r.height() as usize],
                        r.data(),
                    );
                    ui.ctx()
                        .load_texture("Lettering preview", image, egui::TextureOptions::LINEAR)
                });
            self.preview = Some((self.style.clone(), result));
            self.error.clear();
        }
        match &self.preview.as_ref().unwrap().1 {
            Ok(image) => {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(300., 60.), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 3., egui::Color32::from_gray(27));
                ui.painter().image(
                    image.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                true
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                false
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, doc: &mut Document, frame: u32) -> Outcome {
        let dialog = egui::Modal::new("type-editor".into()).show(ctx, |ui| {
            ui.set_width(310.);
            ui.heading("Edit text");
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.style.content)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            if self.focus {
                response.request_focus();
                self.focus = false;
            }
            format_controls(ui, &mut self.style);
            let valid = self.sample(ui);
            if !self.error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &self.error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let apply = ui.add_enabled(valid, egui::Button::new("Apply")).clicked()
                    || (valid
                        && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)));
                let cancel = ui.button("Cancel").clicked();
                (apply, cancel)
            })
            .inner
        });
        if dialog.inner.1 || dialog.should_close() {
            return Outcome::Cancelled;
        }
        if !dialog.inner.0 {
            return Outcome::Editing;
        }
        let result = doc
            .objects
            .iter_mut()
            .find(|o| o.id == self.target)
            .ok_or_else(|| "The text is no longer in this drawing.".to_owned())
            .and_then(|o| self.style.clone().apply_at(o, frame));
        match result {
            Ok(()) => Outcome::Applied(self.target),
            Err(err) => {
                self.error = err;
                Outcome::Editing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: egui::Key, command: bool) -> egui::RawInput {
        let modifiers = egui::Modifiers {
            command,
            ctrl: command,
            ..Default::default()
        };
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            modifiers,
            ..Default::default()
        }
    }

    #[test]
    fn editing_dialog_commits_once_and_escape_preserves_artwork() {
        let ctx = egui::Context::default();
        let mut doc = Document::default();
        let style = TextStyle::default();
        let id = style
            .clone()
            .insert(&mut doc, 1, [30.; 2], [255; 4])
            .unwrap();
        let mut modal = TypeEditor::new(id, style);
        modal.style.content = "Idle\n01".into();
        let mut outcome = Outcome::Editing;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            outcome = modal.show(ctx, &mut doc, 0);
        });
        assert!(matches!(outcome, Outcome::Editing));
        let _ = ctx.run(key(egui::Key::Enter, true), |ctx| {
            outcome = modal.show(ctx, &mut doc, 0);
        });
        assert!(matches!(outcome, Outcome::Applied(1)));
        assert_eq!(doc.objects.len(), 1);
        assert_eq!(doc.objects[0].text.as_ref().unwrap().content, "Idle\n01");
        let before = doc.clone();
        let mut modal = TypeEditor::new(id, doc.objects[0].text.clone().unwrap());
        modal.style.content = "Different".into();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            outcome = modal.show(ctx, &mut doc, 0);
        });
        let _ = ctx.run(key(egui::Key::Escape, false), |ctx| {
            outcome = modal.show(ctx, &mut doc, 0);
        });
        assert!(matches!(outcome, Outcome::Cancelled));
        assert_eq!(before, doc);
    }
}
