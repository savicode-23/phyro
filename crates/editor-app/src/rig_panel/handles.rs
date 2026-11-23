use super::RigPanel;
use document_core::Document;
use eframe::egui::{self, Color32, Pos2, Stroke, Ui};
use std::sync::Arc;

pub(super) struct HandleDrag {
    bone: u64,
    press: Pos2,
    offset: egui::Vec2,
    last: Option<([f32; 2], bool)>,
    preview: Option<Arc<Document>>,
}

impl RigPanel {
    pub(super) fn handles_visible(&self) -> bool {
        self.inspector_open && self.visible && self.pose_handles && !self.placing()
    }

    pub fn dragging_handle(&self) -> bool {
        self.handle_drag.is_some()
    }

    pub fn cancel_handle_drag(&mut self) {
        self.handle_drag = None;
    }

    pub fn handle_preview(&self) -> Option<Arc<Document>> {
        self.handle_drag.as_ref()?.preview.clone()
    }

    /// Keep the pointer offset inside the handle so grabbing its edge cannot
    /// jump the wrist. Only release edits the project or its undo history.
    pub fn handle_input(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        doc: &mut Document,
        frame: u32,
        screen: impl Fn([f32; 2]) -> Pos2,
        world: impl Fn(Pos2) -> Pos2,
    ) -> bool {
        if !self.handles_visible() {
            return false;
        }
        let (focused, pressed, down, released, pos, flip) = ui.input(|i| {
            (
                i.focused,
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
            )
        });
        if !focused || response.secondary_clicked() {
            return self.handle_drag.take().is_some();
        }
        let Some(pos) = pos else {
            return self.handle_drag.take().is_some();
        };
        if self.handle_drag.is_none() {
            if !response.hovered() || !response.interact_rect.contains(pos) {
                return false;
            }
            let Ok(segments) = doc.bone_segments_at(true, frame) else {
                return false;
            };
            let Ok(handles) = doc.ik_handles_at(frame) else {
                return false;
            };
            // Prefer the closest tip; the selected chain wins coincident handles.
            let hit = segments
                .iter()
                .zip(&doc.skeleton.bones)
                .filter(|(_, b)| handles.contains(&b.id))
                .filter_map(|(segment, bone)| {
                    let pin = doc.foot_pin_at(bone.id, frame);
                    let mut distance = pos.distance_sq(screen(segment.tail));
                    if let Some(target) = pin {
                        distance = distance.min(pos.distance_sq(screen(target)));
                    }
                    (distance <= 9_f32.powi(2)).then_some((distance, segment, bone, pin.is_some()))
                })
                .min_by(|a, b| {
                    a.0.total_cmp(&b.0).then_with(|| {
                        (self.selected == Some(b.2.id)).cmp(&(self.selected == Some(a.2.id)))
                    })
                });
            let Some((_, segment, bone, pinned)) = hit else {
                return false;
            };
            ui.ctx().set_cursor_icon(if pinned {
                egui::CursorIcon::PointingHand
            } else {
                egui::CursorIcon::Grab
            });
            response.clone().on_hover_text(if pinned {
                format!("{} · Pinned. Select to release.", bone.name)
            } else {
                format!("Drag {} · Shift to flip bend", bone.name)
            });
            if !pressed {
                return false;
            }
            self.selected = Some(bone.id);
            self.error = None;
            self.handle_drag = Some(HandleDrag {
                bone: bone.id,
                press: pos,
                offset: Pos2::from(segment.tail) - world(pos),
                last: None,
                preview: None,
            });
        }
        let drag = self.handle_drag.as_mut().unwrap();
        if doc.foot_pin_at(drag.bone, frame).is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let target: [f32; 2] = (world(pos) + drag.offset).into();
            if pos.distance_sq(drag.press) >= 1. || drag.last.is_some() {
                if released {
                    self.error = doc.pose_bone_ik(drag.bone, frame, target, flip).err();
                } else if down && drag.last != Some((target, flip)) {
                    drag.last = Some((target, flip));
                    match doc.preview_posed_artwork(drag.bone, frame, target, flip) {
                        Ok(scene) => {
                            self.error = None;
                            drag.preview = Some(Arc::new(scene));
                        }
                        Err(error) => {
                            self.error = Some(error);
                            drag.preview = None;
                        }
                    }
                }
            }
        }
        if released || !down {
            self.handle_drag = None;
        }
        true
    }

    pub(super) fn paint_handle(
        &self,
        painter: &egui::Painter,
        doc: &Document,
        bone: u64,
        frame: u32,
        tip: Pos2,
        screen: &impl Fn([f32; 2]) -> Pos2,
    ) {
        if !tip.is_finite() {
            return;
        }
        let selected = self.selected == Some(bone);
        let color = if selected {
            Color32::from_rgb(255, 196, 100)
        } else {
            Color32::from_rgb(97, 218, 199)
        };
        let stroke = Stroke::new(2_f32, color);
        let pin = doc.foot_pin_at(bone, frame).map(screen);
        let handle = pin.unwrap_or(tip);
        if !handle.is_finite() {
            return;
        }
        if let Some(anchor) = pin {
            if anchor.distance(tip) > 5. {
                painter.line_segment([tip, anchor], stroke);
            }
            let foot = anchor + egui::vec2(0., 7.);
            painter.line_segment(
                [foot - egui::vec2(5., 0.), foot + egui::vec2(5., 0.)],
                stroke,
            );
        } else if let Some((target, _)) = self
            .handle_drag
            .as_ref()
            .filter(|drag| drag.bone == bone)
            .and_then(|drag| drag.last)
        {
            let target = screen(target);
            if target.is_finite() && target.distance(tip) > 5. {
                painter.line_segment([tip, target], (1., color));
                painter.circle_stroke(target, 3., (1., color));
            }
        }
        painter.circle_filled(
            handle,
            if selected { 6. } else { 5. },
            Color32::from_black_alpha(210),
        );
        painter.circle_stroke(handle, if selected { 6. } else { 5. }, stroke);
        if pin.is_some() {
            painter.circle_filled(handle, 2.5, color);
        }
    }
}
