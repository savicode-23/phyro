use super::RigPanel;
use crate::{commands::is_editable, path_editor};
use document_core::Document;
use eframe::egui::{self, Color32, DragValue, Painter, Pos2, Ui};

impl RigPanel {
    pub(super) fn skin_controls(
        &mut self,
        ui: &mut Ui,
        doc: &mut Document,
        selection: &[u64],
        child: u64,
        frame: u32,
    ) {
        let &[id] = selection else { return };
        let Some(path) = doc.objects.iter().find(|o| o.id == id) else {
            return;
        };
        if path.is_instance() || path.text.is_some() || path.shape.contours().is_empty() {
            return;
        }
        let editable = is_editable(doc, path.layer);
        let Some(skin) = &path.skin else {
            let has_parent = doc
                .skeleton
                .bones
                .iter()
                .any(|b| b.id == child && b.parent.is_some());
            if ui
                .add_enabled(editable && has_parent, egui::Button::new("Skin path"))
                .on_hover_text("Blend this path between the selected bone and its parent")
                .on_disabled_hover_text(
                    "Select a child bone and a path on an unlocked, visible layer",
                )
                .clicked()
            {
                self.error = doc.bind_skin_at(id, child, frame).err();
            }
            return;
        };

        let name = |id| {
            doc.skeleton
                .bones
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.name.as_str())
        };
        let (Some(parent), Some(child)) = (name(skin.parent), name(skin.child)) else {
            return;
        };
        ui.small(format!("Skin weights · {parent} → {child}"));
        let (previous_contour, previous_node) = path_editor::selected_endpoint(ui.ctx(), id, frame);
        let mut contour = (previous_contour + 1).min(skin.weights.len());
        if skin.weights.len() > 1 {
            ui.add(
                DragValue::new(&mut contour)
                    .range(1..=skin.weights.len())
                    .prefix("Contour "),
            );
        }
        let weights = &skin.weights[contour - 1];
        let mut node = if contour != previous_contour + 1 {
            1
        } else {
            (previous_node + 1).min(weights.len())
        };
        let mut change = None;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                DragValue::new(&mut node)
                    .range(1..=weights.len())
                    .prefix("Node "),
            )
            .on_hover_text("Choose an anchor by number or click it with the Nodes tool");
            ui.label("Child weight");
            let mut percentage = weights[node - 1] * 100.;
            if ui
                .add_enabled(
                    editable,
                    DragValue::new(&mut percentage)
                        .range(0.0..=100.0)
                        .speed(0.5)
                        .suffix("%"),
                )
                .on_hover_text(format!("0% follows {parent}. 100% follows {child}."))
                .changed()
            {
                change = Some(percentage / 100.);
            }
        });
        if contour - 1 != previous_contour || node - 1 != previous_node {
            path_editor::select(ui.ctx(), id, frame, contour - 1, node - 1);
        }
        if let Some(weight) = change {
            self.error = doc.set_skin_weight(id, contour - 1, node - 1, weight).err();
        }
        self.weight_path = Some(id);
    }

    pub(super) fn paint_weight_node(
        &self,
        painter: &Painter,
        doc: &Document,
        frame: u32,
        screen: &impl Fn([f32; 2]) -> Pos2,
    ) {
        let Some(path) = self
            .weight_path
            .and_then(|id| doc.objects.iter().find(|o| o.id == id))
        else {
            return;
        };
        let (contour, node) = path_editor::selected_endpoint(painter.ctx(), path.id, frame);
        let shape = path.skinned_shape_at(frame);
        if let Some(anchor) = shape
            .contours()
            .get(contour)
            .and_then(|nodes| nodes.get(node))
        {
            let center = screen(path.local_to_world(anchor.point, frame));
            if center.is_finite() {
                painter.circle_stroke(center, 6., (3., Color32::BLACK));
                painter.circle_stroke(center, 6., (1.5, Color32::WHITE));
            }
        }
    }
}
