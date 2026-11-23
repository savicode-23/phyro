mod keys;
pub(super) use keys::KeySelection;
use keys::Scope;
mod layout;
mod paint;
mod tweens;
use self::layout::{Grid, Row, Tracks, LABEL_WIDTH};
use super::{Channel, Timeline};
use document_core::{Clip, VectorObject};
use eframe::egui::{self, Color32, Rect, Sense, Ui};

use crate::theme::MINT as ACCENT;
const KEY: Color32 = Color32::from_rgb(243, 184, 98);

fn has_key(channel: Channel, object: &VectorObject, tick: u32) -> bool {
    // Reading a geometry key through get() clones its full path.
    if channel == Channel::Geometry {
        object.animated.geometry_keys.contains_key(&tick)
    } else {
        channel.get(object, tick).is_some()
    }
}

#[derive(Copy, Clone)]
pub(super) enum Gesture {
    BoxFrames { press_x: f32, dragging: bool },
    MoveKeys { from: u32 },
    BoxKeys { origin: egui::Vec2 },
    Boundary { from: u32, layer: u64 },
}

#[derive(Clone, Copy)]
struct BoneTrack {
    id: u64,
    index: usize,
    start: u32,
    end: u32,
}

impl BoneTrack {
    fn selected(
        clip: &Clip,
        frame: u32,
        selection: Option<(u64, u32, u64)>,
    ) -> Option<(Self, &str)> {
        let (session, exposure, id) = selection?;
        let (start, duration) = clip.rig_exposure(frame);
        if session != clip.session_id() || start != exposure {
            return None;
        }
        let (index, bone) = clip
            .drawing_at(frame)
            .skeleton
            .bones
            .iter()
            .enumerate()
            .find(|(_, b)| b.id == id && !b.rotation_keys.is_empty())?;
        Some((
            Self {
                id,
                index,
                start,
                end: start + duration,
            },
            &bone.name,
        ))
    }

    fn bone(self, clip: &Clip) -> Option<&document_core::Bone> {
        clip.cels
            .get(&self.start)?
            .skeleton
            .bones
            .get(self.index)
            .filter(|bone| bone.id == self.id)
    }

    fn key_at(self, clip: &Clip, tick: u32) -> bool {
        (self.start..self.end).contains(&tick)
            && self
                .bone(clip)
                .is_some_and(|bone| bone.rotation_keys.contains_key(&tick))
    }
}

impl Timeline {
    pub(super) fn ruler(
        &mut self,
        ui: &mut Ui,
        clip: &mut Clip,
        selected: Option<u64>,
        selected_bone: Option<(u64, u32, u64)>,
    ) -> bool {
        self.sync_key_selection(clip, selected, selected_bone);
        let exposure = clip.exposure(self.frame).0;
        let disclosure = ui.make_persistent_id((
            "timeline-track-group",
            clip.session_id(),
            exposure,
            selected,
        ));
        let collapsed = ui
            .data(|data| data.get_temp::<bool>(disclosure))
            .unwrap_or(false);
        let tracks = Tracks::new(
            clip,
            self.frame,
            selected,
            selected_bone,
            self.channel,
            self.expanded,
            collapsed,
        );
        let reveal = tracks
            .selected
            .map(|id| (clip.session_id(), tracks.exposure.start, id));
        let reveal_rows = reveal != self.revealed_object;
        self.revealed_object = reveal;
        let area_id = egui::Id::new(("clip-ruler", clip.session_id()));
        let area = egui::ScrollArea::both()
            .id_salt(area_id)
            .min_scrolled_height(0.)
            .min_scrolled_width(0.)
            .auto_shrink([false, false])
            .animated(false)
            .drag_to_scroll(false);
        // The label gutter and frame header are anchored in the parent. Reconstructing
        // this origin from the rounded content position accumulates subpixels on long clips.
        let viewport_origin = ui.available_rect_before_wrap().min;
        area.show_viewport(ui, |ui, viewport| {
            let cell = self.frame_width;
            let size = egui::vec2(LABEL_WIDTH + cell * clip.frames as f32, tracks.height);
            let (content, _) = ui.allocate_exact_size(size, Sense::hover());
            let view =
                Rect::from_min_size(viewport_origin, viewport.size()).intersect(ui.clip_rect());
            let grid = Grid {
                content,
                view,
                cell,
            };
            let response = ui.interact(
                grid.frame_area(),
                ui.id().with("timeline-grid"),
                Sense::click_and_drag(),
            );
            if reveal_rows {
                if let Some((top, bottom)) = tracks.selected_rows() {
                    let body = grid.body_clip();
                    let y0 = content.top() + top;
                    let y1 = content.top() + bottom;
                    if y0 < body.top() || y1 > body.bottom() {
                        let x = view.center().x;
                        // In a short dock, keep the first editable track fully visible.
                        // Its layer label can scroll behind the pinned frame header.
                        let top = y0.max(y1 - body.height().max(0.));
                        ui.scroll_to_rect(
                            Rect::from_min_max(
                                egui::pos2(x, top - layout::HEADER),
                                egui::pos2(x, y1),
                            ),
                            None,
                        );
                    }
                }
            }
            let mut changed = self.ruler_input(ui, &response, &grid, clip, &tracks);
            changed |= self.layer_cell_menu(&response, &grid, clip, &tracks);
            if std::mem::take(&mut self.reveal_frame) || self.is_playing() {
                // Include the sticky label gutter when revealing a frame to the left.
                let frame_x = grid.x(self.frame);
                let left = grid.time_view().left();
                if frame_x < left || frame_x + cell > grid.view.right() {
                    let rect = Rect::from_min_max(
                        egui::pos2(frame_x - LABEL_WIDTH, view.center().y),
                        egui::pos2(frame_x + cell, view.center().y),
                    );
                    ui.scroll_to_rect(rect, None);
                }
            }
            let updated = changed.then(|| {
                Tracks::new(
                    clip,
                    self.frame,
                    selected,
                    selected_bone,
                    self.channel,
                    self.expanded,
                    collapsed,
                )
            });
            self.paint_sheet(
                ui,
                &grid,
                clip,
                updated.as_ref().unwrap_or(&tracks),
                (disclosure, collapsed),
            );
            changed
        })
        .inner
    }

    fn layer_cell_menu(
        &mut self,
        response: &egui::Response,
        grid: &Grid,
        clip: &mut Clip,
        tracks: &Tracks,
    ) -> bool {
        let mut changed = false;
        if response.secondary_clicked() {
            self.cell_menu = response.interact_pointer_pos().and_then(|pos| {
                let Row::Layer(index) = grid.row_at(pos, tracks)? else {
                    return None;
                };
                let layer = clip.cels.get(&tracks.drawing_start)?.layers.get(index)?.id;
                Some((clip.session_id(), layer, grid.tick(pos.x, clip.frames)))
            });
        }
        if let Some((_, layer, tick)) = self
            .cell_menu
            .filter(|(session, _, tick)| *session == clip.session_id() && *tick < clip.frames)
        {
            response.context_menu(|ui| {
                let editable = !self.is_playing()
                    && crate::commands::is_editable(clip.drawing_at(tick), layer);
                ui.add_enabled_ui(editable, |ui| {
                    let blank = ui.button("Blank layer drawing").clicked();
                    let copy = ui.button("Copy layer drawing").clicked();
                    if blank || copy {
                        self.active_layer = layer;
                        self.layer_request = Some(layer);
                        let result = if blank {
                            clip.blank_layer_frame(tick, layer)
                        } else {
                            clip.duplicate_layer_frame(tick, layer)
                        };
                        self.reveal_frame = true;
                        changed |= self.frame_edit(result);
                        ui.close_menu();
                    }
                });
            });
        }
        changed
    }

    fn ruler_input(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        grid: &Grid,
        clip: &mut Clip,
        tracks: &Tracks,
    ) -> bool {
        if ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
            self.gesture = None;
            return false;
        }
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            return false;
        };
        let tick = grid.tick(pos.x, clip.frames);
        let over = response.contains_pointer() && response.rect.contains(pos);
        let band = if over {
            grid.band_at(pos, tracks)
        } else {
            None
        };
        let layer = band.and_then(|band| match band.row {
            Row::Layer(index) => clip.cels[&tracks.drawing_start].layers.get(index),
            _ => None,
        });
        let (extend, additive) =
            ui.input(|i| (i.modifiers.shift, i.modifiers.command || i.modifiers.ctrl));
        let boundary = layer.filter(|l| {
            // At ordinary zoom only the leading edge retimes a drawing. Compressed
            // cells keep the entire start cell as a draggable target.
            !extend
                && !additive
                && l.visible
                && !l.locked
                && tick > 0
                && (grid.cell <= 6. || pos.x - grid.x(tick) <= (grid.cell * 0.3).min(5.))
                && clip.layer_exposure(tick, l.id).0 == tick
        });
        if matches!(self.gesture, Some(Gesture::Boundary { .. })) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else if over && self.gesture.is_none() && (band.is_none() || layer.is_some()) {
            if boundary.is_some() {
                response
                    .clone()
                    .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                    .on_hover_text("Drag this edge to move the start of the drawing.");
            } else {
                response.clone().on_hover_ui(|ui| {
                    ui.label("Drag to select frames. Shift-click to select a range.");
                    ui.label(if ui.ctx().os() == egui::os::OperatingSystem::Mac {
                        "Command-click to toggle a frame. Command-drag to add a range."
                    } else {
                        "Ctrl-click to toggle a frame. Ctrl-drag to add a range."
                    });
                    ui.label(crate::keymap::hint(
                        ui.ctx(),
                        "Delete the selected frames across all layers",
                        crate::keymap::Action::Delete,
                    ));
                });
            }
        }
        if over && ui.input(|i| i.pointer.primary_pressed()) {
            let key_row = band.filter(|b| {
                matches!(
                    b.row,
                    Row::Property(_) | Row::Bone | Row::Child | Row::Object
                )
            });
            if let Some(band) = key_row {
                let row = band.row;
                let keep_keys = extend || additive;
                self.stop();
                self.frame_focus = None;
                self.keys.scope = Some(tracks.scope);
                let center =
                    egui::pos2(grid.x(tick) + grid.cell * 0.5, grid.band(&band).center().y);
                let on_diamond = (pos.x - center.x).abs() <= (grid.cell * 0.5).min(6.)
                    && (pos.y - center.y).abs() <= 6.;
                if on_diamond && tracks.key_at(row, clip, tick) {
                    if let Row::Property(channel) = row {
                        self.channel = channel;
                    }
                    if !keep_keys && !self.keys.marks.contains(&(row, tick)) {
                        self.keys.marks.clear();
                    }
                    if keep_keys && self.keys.marks.remove(&(row, tick)) {
                        self.gesture = None;
                    } else {
                        self.keys.marks.insert((row, tick));
                        self.gesture = Some(Gesture::MoveKeys { from: tick });
                    }
                    self.select_frame(tick);
                } else {
                    if !keep_keys {
                        self.keys.marks.clear();
                    }
                    self.gesture = Some(Gesture::BoxKeys {
                        origin: pos - grid.content.min,
                    });
                }
            } else {
                self.stop();
                self.error = None;
                let session = clip.session_id();
                let focus = self
                    .frame_focus
                    .get_or_insert_with(|| super::FrameFocus::new(session, self.frame));
                if focus.clip != session {
                    *focus = super::FrameFocus::new(session, self.frame);
                }
                focus.start(tick, extend, additive);
                self.frame = tick;
                self.keys.clear_marks();
                self.keys.scope = Some(tracks.scope);
                if let Some(l) = layer {
                    self.layer_request = Some(l.id);
                    self.active_layer = l.id;
                }
                self.gesture = Some(if let Some(l) = boundary {
                    Gesture::Boundary {
                        from: tick,
                        layer: l.id,
                    }
                } else {
                    Gesture::BoxFrames {
                        press_x: pos.x,
                        dragging: false,
                    }
                });
            }
        }
        let view = grid.time_view();
        let mut drag_frames = false;
        if let Some(Gesture::BoxFrames { press_x, dragging }) = &mut self.gesture {
            let started = !*dragging && (pos.x - *press_x).abs() >= 3.;
            *dragging |= started;
            drag_frames = *dragging;
            if drag_frames {
                let x = pos
                    .x
                    .min(view.right() - 0.5 / ui.pixels_per_point())
                    .max(view.left());
                let tick = grid.tick(x, clip.frames);
                if started || tick != self.frame {
                    if let Some(focus) = &mut self.frame_focus {
                        focus.extend_to(tick);
                    }
                    self.frame = tick;
                }
            }
        }
        let drag_keys = matches!(
            self.gesture,
            Some(Gesture::BoxKeys { .. } | Gesture::MoveKeys { .. })
        ) && ui.input(|i| i.pointer.is_decidedly_dragging());
        if (drag_frames || drag_keys) && ui.input(|i| i.pointer.primary_down()) {
            let dt = ui.input(|i| i.stable_dt.min(0.05));
            let edge = |value: f32, low: f32, high: f32| {
                ((low + 12. - value).max(0.) - (value - high + 12.).max(0.)).clamp(-30., 30.)
                    * 60.
                    * dt
            };
            let dx = edge(pos.x, view.left(), view.right()).clamp(
                -(grid.content.right() - view.right()).max(0.),
                (grid.view.left() - grid.content.left()).max(0.),
            );
            let dy = if drag_keys {
                edge(pos.y, grid.rows_top(), view.bottom())
            } else {
                0.
            };
            let delta = egui::vec2(dx, dy);
            if delta != egui::Vec2::ZERO {
                ui.scroll_with_delta(delta);
                ui.ctx().request_repaint();
            }
        }
        if ui.input(|i| i.pointer.primary_released()) {
            if let Some(Gesture::BoxKeys { origin }) = self.gesture {
                self.gesture = None;
                let origin = grid.content.min + origin;
                if origin.distance(pos) < 3. {
                    if let Some(Row::Property(channel)) = grid.row_at(pos, tracks) {
                        self.channel = channel;
                    }
                    self.select_frame(tick);
                } else {
                    let area = Rect::from_two_pos(origin, pos)
                        .expand(2.)
                        .intersect(grid.content);
                    let first = grid.tick(area.left(), clip.frames);
                    let last = grid.tick(area.right(), clip.frames);
                    if area.is_positive() {
                        let object = tracks.object(clip);
                        for band in tracks.bands(
                            area.top() - grid.content.top(),
                            area.bottom() - grid.content.top(),
                        ) {
                            let y = grid.band(&band).center().y;
                            if area.y_range().contains(y)
                                && matches!(band.row, Row::Property(_) | Row::Bone | Row::Child)
                            {
                                let span = tracks.row_exposure(band.row);
                                for t in first.max(span.start)..=last.min(span.end - 1) {
                                    let x = grid.x(t) + grid.cell * 0.5;
                                    if area.x_range().contains(x)
                                        && keys::key_at(
                                            band.row,
                                            clip,
                                            object,
                                            tracks.bone,
                                            &tracks.exposure,
                                            t,
                                        )
                                    {
                                        self.keys.marks.insert((band.row, t));
                                    }
                                }
                            }
                        }
                    }
                }
                return false;
            }
            return self.finish_gesture(clip, tick);
        }
        false
    }

    fn finish_gesture(&mut self, clip: &mut Clip, to: u32) -> bool {
        let result = match self.gesture.take() {
            Some(Gesture::MoveKeys { from }) if from != to => {
                self.keys.edit(clip, Some(to as i32 - from as i32), to)
            }
            Some(Gesture::Boundary { from, layer }) if to != from => {
                clip.move_layer_boundary(from, to, layer)
            }
            _ => return false,
        };
        self.frame_edit(result.map(|()| to))
    }
}

#[cfg(test)]
mod tests;
