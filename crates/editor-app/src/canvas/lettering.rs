use super::{egui, is_editable, point_text::PointText, Canvas, CanvasStyle, Document, Pos2};
use document_core::typography::TextStyle;

impl Canvas {
    pub(crate) fn text_controls(&mut self, ui: &mut egui::Ui) {
        let style = self
            .point_text
            .as_mut()
            .map_or(&mut self.text_defaults, |s| &mut s.style);
        let controls = ui.vertical(|ui| crate::type_editor::format_controls(ui, style));
        if let Some(session) = &mut self.point_text {
            session.controls = Some(controls.response.rect.intersect(ui.clip_rect()));
            // DragValue releases focus on Enter/Escape. Keep that event out of
            // lettering on the same frame, before the canvas takes focus again.
            session.number_focused = controls.inner
                || session.number_focused
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)
                    });
        }
    }

    pub(crate) fn text_active(&self) -> bool {
        self.point_text.is_some()
    }

    pub(crate) fn text_needs_correction(&self) -> bool {
        self.point_text.as_ref().is_some_and(|s| s.error.is_some())
    }

    // Finish in the current drawing before a toolbar or timeline click can switch it.
    pub(crate) fn text_should_finish(&self, ctx: &egui::Context) -> bool {
        self.point_text.as_ref().is_some_and(|session| {
            // Plain typing continues to belong to the text tool, even when a
            // printable key has been assigned to Save in the artwork editor.
            let save_chord = ctx.input(|i| i.modifiers.command)
                && (crate::keymap::pressed(ctx, crate::keymap::Action::Save)
                    || crate::keymap::pressed(ctx, crate::keymap::Action::SaveAs));
            save_chord
                || ctx.input(|i| {
                    i.viewport().close_requested()
                        || i.pointer.primary_pressed()
                            && i.pointer.interact_pos().is_some_and(|p| !session.hit(p))
                })
        })
    }

    pub(super) fn start_text(
        &mut self,
        doc: &Document,
        frame: u32,
        point: Pos2,
        selection: &mut Vec<u64>,
        style: &CanvasStyle,
    ) {
        if let Some((o, text)) = doc.text_at(point.into(), frame) {
            *selection = vec![o.id];
            let mut session = PointText::new(
                Some(o.id),
                o.layer,
                o.local_to_world([0., 0.], frame),
                text.clone(),
                o.fill_at(frame),
            );
            let [left, top, right, bottom] = o.bounds(frame);
            session.opening_local = Some(o.world_to_local(point.into(), frame));
            session.original_bounds = Some(egui::Rect::from_min_max(
                egui::pos2(left, top),
                egui::pos2(right, bottom),
            ));
            self.point_text = Some(session);
        } else if is_editable(doc, style.layer) {
            selection.clear();
            self.point_text = Some(PointText::new(
                None,
                style.layer,
                point.into(),
                TextStyle {
                    content: String::new(),
                    ..self.text_defaults.clone()
                },
                style.fill,
            ));
        }
        if let Some(session) = &mut self.point_text {
            session.frame = frame;
            if session.target.is_none() {
                session.new_object_style = Some(style.clone());
            }
        }
    }

    pub(crate) fn finish_text(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        selection: &mut Vec<u64>,
    ) -> Result<bool, String> {
        let Some(session) = &mut self.point_text else {
            return Ok(false);
        };
        let cleared = session.style.content.trim().is_empty();
        let result = if !is_editable(doc, session.layer) {
            Err("Choose a visible, unlocked layer for the text.".into())
        } else if cleared {
            Ok(session.target.filter(|id| {
                if let Some(index) = doc
                    .objects
                    .iter()
                    .position(|o| o.id == *id && o.layer == session.layer && o.text.is_some())
                {
                    doc.objects.remove(index);
                    true
                } else {
                    false
                }
            }))
        } else if let Some(id) = session.target {
            match doc
                .objects
                .iter_mut()
                .find(|o| o.id == id && o.layer == session.layer && o.text.is_some())
            {
                Some(o) if o.text.as_ref() == Some(&session.style) => Ok(None),
                Some(o) => session
                    .style
                    .clone()
                    .apply_at(o, session.frame)
                    .map(|()| Some(id)),
                None => Err("The lettering is no longer available to edit.".into()),
            }
        } else {
            session.insert(doc).map(Some)
        };
        match result {
            Ok(id) => {
                if id.is_some() {
                    self.text_defaults = TextStyle {
                        content: String::new(),
                        ..session.style.clone()
                    };
                }
                session.surrender_focus(ctx);
                self.point_text = None;
                if let Some(id) = id {
                    if cleared {
                        selection.clear()
                    } else {
                        *selection = vec![id];
                    }
                }
                ctx.request_repaint();
                Ok(id.is_some())
            }
            Err(error) => {
                session.error = Some(error.clone());
                Err(error)
            }
        }
    }
}
