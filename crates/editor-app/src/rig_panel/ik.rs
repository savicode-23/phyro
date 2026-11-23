use super::RigPanel;
use document_core::Document;
use eframe::egui::{Button, Color32, Painter, Pos2, Ui};

impl RigPanel {
    pub(super) fn ik_controls(
        &mut self,
        ui: &mut Ui,
        doc: &Document,
        id: u64,
        frame: u32,
    ) -> Option<bool> {
        let bone = doc.skeleton.bones.iter().find(|bone| bone.id == id)?;
        let child = bone.parent.is_some();
        let pinned = doc.foot_pin_at(id, frame).is_some();
        let eligible = child
            && doc
                .ik_handles_at(frame)
                .is_ok_and(|handles| handles.contains(&id));
        if self.ik_target == Some(id) && !pinned {
            ui.small(format!(
                "Click where the tip should reach on frame {}.",
                frame + 1
            ));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.ik_flip, "Flip bend");
                if ui.button("Cancel target").clicked() {
                    self.cancel_placement();
                }
            });
            None
        } else {
            let placement_hint = if pinned {
                "Release the foot before moving its handle"
            } else if !child {
                "Select a child bone to pose a two-bone chain"
            } else {
                "Choose two movable joints with distinct pivots"
            };
            let (action, hint) = if pinned {
                ("Release foot", "End contact on this frame")
            } else {
                ("Pin foot", "Plant this tip on this frame until release")
            };
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(eligible && !pinned, Button::new("Place IK target"))
                    .on_disabled_hover_text(placement_hint)
                    .on_hover_text("Click the artboard to key both joint rotations on this frame")
                    .clicked()
                {
                    self.cancel_placement();
                    self.visible = true;
                    self.ik_flip = false;
                    self.ik_target = Some(id);
                }
                if ui
                    .add_enabled(eligible, Button::new(action))
                    .on_hover_text(hint)
                    .on_disabled_hover_text(placement_hint)
                    .clicked()
                {
                    self.cancel_placement();
                    Some(!pinned)
                } else {
                    None
                }
            })
            .inner
        }
    }

    pub(super) fn apply_ik(&mut self, doc: &mut Document, id: u64, frame: u32, target: [f32; 2]) {
        match doc.pose_bone_ik(id, frame, target, self.ik_flip) {
            Ok(()) => self.cancel_placement(),
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn paint_ik_preview(
        &self,
        painter: &Painter,
        doc: &Document,
        id: u64,
        frame: u32,
        target: [f32; 2],
        screen: &impl Fn([f32; 2]) -> Pos2,
    ) {
        let Ok(joints) = doc.preview_bone_ik(id, frame, target, self.ik_flip) else {
            return;
        };
        let [shoulder, elbow, tip] = joints.map(screen);
        let cursor = screen(target);
        if ![shoulder, elbow, tip, cursor]
            .iter()
            .all(|point| point.is_finite())
        {
            return;
        }
        let color = Color32::from_rgb(255, 210, 135);
        for segment in [[shoulder, elbow], [elbow, tip]] {
            painter.line_segment(segment, (4., Color32::from_black_alpha(160)));
            painter.line_segment(segment, (2., color));
        }
        painter.circle_stroke(elbow, 3., (1.5, color));
        painter.circle_stroke(tip, 4., (1.5, color));
        if cursor.distance(tip) > 5. {
            painter.line_segment([tip, cursor], (1., color.gamma_multiply(0.65)));
            painter.circle_stroke(cursor, 3., (1., color));
        }
    }
}
