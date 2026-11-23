use super::*;
use document_core::Clip;

pub(super) fn target(doc: &Document, selection: &[u64], frame: u32) -> Option<usize> {
    let [id] = selection else { return None };
    doc.objects.iter().position(|object| {
        object.id == *id
            && object.visible_at(frame)
            && document_core::arrange::is_object_editable(doc, object)
    })
}

impl Canvas {
    pub(crate) fn editing_pivot(&self) -> bool {
        self.editing_pivot.is_some()
    }

    /// Returns true when resetting a custom pivot changes the drawing.
    pub(crate) fn pivot_controls(
        &mut self,
        ui: &mut egui::Ui,
        clip: &mut Clip,
        frame: u32,
        selection: &[u64],
        enabled: bool,
    ) -> bool {
        let drawing = clip.drawing_at(frame);
        let Some(index) = target(drawing, selection, frame).filter(|_| enabled) else {
            self.editing_pivot = None;
            return false;
        };
        let object = &drawing.objects[index];
        let editing = self.editing_pivot == Some(object.id);
        ui.separator();
        if ui.selectable_label(editing, "Edit pivot")
            .on_hover_text("Click or drag on the canvas to place the transform pivot. Press Escape to return to moving artwork.")
            .clicked()
        {
            self.cancel_drag();
            self.editing_pivot = (!editing).then_some(object.id);
        }
        if object.pivot.is_some()
            && ui
                .small_button("Reset pivot")
                .on_hover_text("Return the transform pivot to the center of the object")
                .clicked()
        {
            return clip.drawing_at_mut(frame).objects[index].reset_pivot_at(frame);
        }
        false
    }
}

const ARM_LENGTH: f32 = 8.;

pub(super) fn over_marker(delta: Vec2) -> bool {
    let delta = delta.abs();
    // Leave the diagonal spaces between the crosshair's arms available for moving artwork.
    delta.length_sq() <= 5. * 5.
        || (delta.max_elem() <= ARM_LENGTH + 1.5 && delta.min_elem() <= 2.5)
}

pub(super) fn marker(painter: &egui::Painter, center: Pos2, highlighted: bool) {
    if !painter.clip_rect().expand(ARM_LENGTH + 2.).contains(center) {
        return;
    }
    let accent = if highlighted {
        Color32::from_rgb(12, 75, 148)
    } else {
        Color32::from_rgb(29, 104, 185)
    };
    if highlighted {
        painter.circle_filled(center, 4., Color32::WHITE);
    }
    for (width, color) in [(3.0, Color32::WHITE), (1.3, accent)] {
        painter.circle_stroke(center, 4., (width, color));
        for axis in [Vec2::X, Vec2::Y, -Vec2::X, -Vec2::Y] {
            painter.line_segment(
                [center + axis * 4., center + axis * ARM_LENGTH],
                (width, color),
            );
        }
    }
    painter.circle_filled(center, 1.2, accent);
}
