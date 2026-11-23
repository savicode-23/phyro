use super::*;
use document_core::VectorObject;

// TextEdit supplies keyboard navigation, clipboard, text Undo and native IME.
// Its logical galley is used for cursor metrics. Actual glyphs are painted by
// the artwork renderer with the object's full transform, paint and layer order.
const METRIC_SIZE: f32 = 32.;

#[derive(Clone, Copy)]
struct Placement {
    origin: Pos2,
    x: egui::Vec2,
    y: egui::Vec2,
}

impl Placement {
    fn from_object(object: &VectorObject, frame: u32, view: View) -> Self {
        let world = object.local_to_world_at(frame);
        let origin = view.screen(world([0., 0.]).into());
        Self {
            origin,
            x: view.screen(world([1., 0.]).into()) - origin,
            y: view.screen(world([0., 1.]).into()) - origin,
        }
    }

    fn screen(self, uv: egui::Vec2) -> Pos2 {
        self.origin + uv.x * self.x + uv.y * self.y
    }

    fn corners(self) -> [Pos2; 4] {
        [
            self.origin,
            self.origin + self.x,
            self.origin + self.x + self.y,
            self.origin + self.y,
        ]
    }

    fn bounds(self) -> Rect {
        Rect::from_points(&self.corners())
    }

    fn uv(self, point: Pos2) -> egui::Vec2 {
        let d = point - self.origin;
        let det = self.x.x * self.y.y - self.x.y * self.y.x;
        if det.abs() < f32::EPSILON {
            return egui::Vec2::ZERO;
        }
        egui::vec2(
            (d.x * self.y.y - d.y * self.y.x) / det,
            (self.x.x * d.y - self.x.y * d.x) / det,
        )
    }
}

impl PointText {
    pub(in crate::canvas) fn preview(&self) -> Option<&Arc<Document>> {
        self.preview.as_ref()
    }

    // Rebuild outlines and the frozen drawing only when the text or its
    // formatting changes. An unchanged session reuses the original artwork.
    fn prepare(&mut self, doc: &Document) {
        if self.prepared.as_ref() == Some(&self.style) {
            return;
        }
        self.prepared = Some(self.style.clone());
        self.preview = None;
        self.error = None;
        self.preview_error = None;
        let index = self
            .target
            .and_then(|id| doc.objects.iter().position(|o| o.id == id));
        if index.is_some_and(|i| doc.objects[i].text.as_ref() == Some(&self.style)) {
            return;
        }
        let empty = self.style.content.trim().is_empty();
        if empty && index.is_none() {
            return;
        }
        let mut draft = doc.clone();
        let result = if empty {
            Ok(())
        } else if let Some(i) = index {
            self.style
                .clone()
                .apply_at(&mut draft.objects[i], self.frame)
        } else {
            self.insert(&mut draft).map(|_| ())
        };
        self.preview_error = result.err();
        if empty || self.preview_error.is_some() {
            if let Some(i) = index {
                draft.objects.remove(i);
            }
        }
        self.preview = Some(Arc::new(draft));
    }

    fn placement(
        &self,
        doc: &Document,
        frame: u32,
        view: View,
        galley: &egui::Galley,
    ) -> Placement {
        let scene = self.preview.as_deref().unwrap_or(doc);
        let object = if let Some(id) = self.target {
            scene.objects.iter().find(|o| o.id == id)
        } else {
            self.preview
                .as_ref()
                .and_then(|draft| draft.objects.last())
                .filter(|o| o.id >= doc.next_id && o.text.is_some())
        };
        if let Some(object) = object {
            return Placement::from_object(object, frame, view);
        }
        let size =
            galley.size().max(egui::vec2(1., 1.)) * (self.style.size * view.scale / METRIC_SIZE);
        if let Some(original) = self
            .target
            .and_then(|id| doc.objects.iter().find(|o| o.id == id))
        {
            let old = Placement::from_object(original, frame, view);
            return Placement {
                origin: old.origin,
                x: old.x.normalized() * size.x,
                y: old.y.normalized() * size.y,
            };
        }
        Placement {
            origin: view.screen(self.anchor.into()),
            x: egui::vec2(size.x, 0.),
            y: egui::vec2(0., size.y),
        }
    }

    pub(in crate::canvas) fn paint_feedback(&self, painter: &egui::Painter) {
        painter.extend(self.feedback.iter().cloned());
    }

    pub(in crate::canvas) fn show(
        &mut self,
        ui: &mut Ui,
        view: View,
        viewport: Rect,
        doc: &Document,
        frame: u32,
    ) -> Outcome {
        let clip = viewport.intersect(ui.clip_rect());
        let id = ui.make_persistent_id("canvas point text content");
        self.id = Some(id);
        let first = std::mem::take(&mut self.opening);
        let active = ui.is_enabled()
            && ui.input(|i| i.focused)
            && ui.memory(|m| m.allows_interaction(ui.layer_id()));
        let writing = active && !self.number_focused;
        let (cancel, commit) = if writing {
            ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                )
            })
        } else {
            (false, false)
        };
        self.prepare(doc);
        let font = FontId::new(
            METRIC_SIZE,
            match self.style.face {
                Typeface::UbuntuLight => FontFamily::Proportional,
                Typeface::HackRegular => FontFamily::Monospace,
            },
        );
        let format = TextFormat {
            font_id: font.clone(),
            line_height: Some(METRIC_SIZE * self.style.line_height),
            extra_letter_spacing: self.style.letter_spacing * METRIC_SIZE / self.style.size,
            valign: egui::Align::TOP,
            ..Default::default()
        };
        let align = self.style.align;
        let layout = |ui: &Ui, text: &str| lettering(ui, text, format.clone(), align);
        let before = layout(ui, &self.style.content);
        let pose = self.placement(doc, frame, view, &before);
        let old = self
            .original_bounds
            .map(|b| Rect::from_min_max(view.screen(b.min), view.screen(b.max)));
        let hit_box = old
            .map_or(pose.bounds(), |r| r.union(pose.bounds()))
            .expand(3.)
            .intersect(clip);
        let pick = ui
            .interact(hit_box, id.with("pick"), egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Text);
        let mut state = if first {
            egui::text_edit::TextEditState::default()
        } else {
            egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default()
        };
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let uv = self
                .opening_local
                .take()
                .map(egui::Vec2::from)
                .unwrap_or_else(|| pose.uv(pointer));
            let at = before.cursor_from_pos(uv * before.size());
            if first {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(at.ccursor)));
            } else if writing {
                state
                    .cursor
                    .pointer_interaction(ui, &pick, at, &before, pick.dragged());
            }
        }
        state.store(ui.ctx(), id);
        if writing && !ui.memory(|m| m.has_focus(id)) {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let mut field = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("point text input")
                .max_rect(Rect::from_min_size(
                    clip.right_bottom() + egui::vec2(64., 64.),
                    egui::vec2(32., 0.),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        field.set_clip_rect(Rect::NOTHING);
        if !active {
            field.disable();
        }
        let mut layouter = |ui: &Ui, text: &str, _: f32| layout(ui, text);
        let edit = egui::TextEdit::multiline(&mut self.style.content)
            .id(id)
            .font(font)
            .frame(false)
            .margin(0.)
            .desired_rows(1)
            .desired_width(32.)
            .char_limit(2048)
            .layouter(&mut layouter)
            .show(&mut field);
        self.prepare(doc);
        let pose = self.placement(doc, frame, view, &edit.galley);
        let bounds = pose.bounds();
        self.rect = old
            .map_or(bounds, |r| r.union(bounds))
            .expand(3.)
            .intersect(clip);
        self.feedback.clear();
        self.feedback.push(egui::Shape::closed_line(
            pose.corners().to_vec(),
            (1., Color32::from_rgb(55, 161, 181)),
        ));
        if let Some(range) = edit.state.cursor.range(&edit.galley) {
            let galley = &edit.galley;
            let metric_size = galley.size().max(egui::vec2(1., 1.));
            let map = |p: Pos2| pose.screen(p.to_vec2() / metric_size);
            let [start, end] = range.sorted_cursors();
            if !range.is_empty() {
                for (row_no, row) in galley
                    .rows
                    .iter()
                    .enumerate()
                    .take(end.rcursor.row + 1)
                    .skip(start.rcursor.row)
                {
                    let left = if row_no == start.rcursor.row {
                        row.x_offset(start.rcursor.column)
                    } else {
                        row.rect.left()
                    };
                    let right = if row_no == end.rcursor.row {
                        row.x_offset(end.rcursor.column)
                    } else {
                        row.rect.right()
                    };
                    let rect = Rect::from_min_max(
                        egui::pos2(left, row.min_y()),
                        egui::pos2(right, row.max_y()),
                    );
                    self.feedback.push(egui::Shape::convex_polygon(
                        [
                            rect.left_top(),
                            rect.right_top(),
                            rect.right_bottom(),
                            rect.left_bottom(),
                        ]
                        .map(map)
                        .to_vec(),
                        Color32::from_rgba_unmultiplied(35, 145, 240, 65),
                        egui::Stroke::NONE,
                    ));
                }
            }
            let at = galley.pos_from_cursor(&range.primary);
            let ends = [map(at.left_top()), map(at.left_bottom())];
            let caret = Rect::from_two_pos(ends[0], ends[1]).expand(1.);
            if writing {
                self.feedback.push(egui::Shape::line_segment(
                    ends,
                    (3., Color32::from_black_alpha(190)),
                ));
                self.feedback
                    .push(egui::Shape::line_segment(ends, (1., Color32::WHITE)));
                ui.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        rect: bounds,
                        cursor_rect: caret,
                    })
                });
            }
            let navigated = ui.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            pressed: true,
                            key: egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                                | egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::Home
                                | egui::Key::End
                                | egui::Key::PageUp
                                | egui::Key::PageDown,
                            ..
                        }
                    )
                })
            });
            self.reveal = ((edit.response.changed() || navigated)
                && !clip.shrink(6.).contains_rect(caret))
            .then_some(caret);
        }
        if let Some(message) = self.error.as_ref().or(self.preview_error.as_ref()) {
            let galley = ui.painter().layout_no_wrap(
                format!("{}  ·  {message}", self.style.content),
                FontId::proportional(13.),
                ui.visuals().error_fg_color,
            );
            self.feedback.push(egui::Shape::galley(
                bounds.left_bottom(),
                galley,
                ui.visuals().error_fg_color,
            ));
        }
        let clicked_elsewhere = active
            && !first
            && ui.input(|i| {
                i.pointer.primary_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| !self.hit(p))
            });
        if cancel || commit || clicked_elsewhere {
            if cancel || self.target.is_none() && self.style.content.trim().is_empty() {
                Outcome::Cancel
            } else {
                Outcome::Commit
            }
        } else {
            if writing && !edit.response.has_focus() {
                edit.response.request_focus();
            }
            Outcome::Editing
        }
    }
}
