use super::{commands, number};
use document_core::{Document, Shape, Stencil, VectorObject};
use eframe::egui::{self, Ui};
use std::sync::Arc;

pub(super) fn show(ui: &mut Ui, doc: &mut Arc<Document>, selected: &mut Vec<u64>, frame: u32) {
    if selected.is_empty() {
        return;
    }
    let single = match selected.as_slice() {
        [id] => {
            let Some(i) = doc
                .objects
                .iter()
                .position(|o| o.id == *id && commands::is_editable(doc, o.layer))
            else {
                return;
            };
            Some(i)
        }
        _ => None,
    };
    ui.collapsing("Clip and mask", |ui| {
        let status = ui.make_persistent_id(("clip-error", selected.as_slice()));
        if let Some(action) = clip_actions(ui, doc, selected, false) {
            match action.apply(Arc::make_mut(doc), selected, frame) {
                Ok(()) => ui.data_mut(|d| d.remove::<String>(status)),
                Err(message) => ui.data_mut(|d| d.insert_temp(status, message)),
            }
        } else if let Some(i) = single.filter(|&i| doc.objects[i].clip_source.is_none()) {
            stencil(ui, doc, i);
        } else {
            captured_alpha(ui, doc, selected);
        }
        if let Some(message) = ui.data(|d| d.get_temp::<String>(status)) {
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
    });
}

#[derive(Clone, Copy)]
pub(crate) enum ClipAction {
    SetClip,
    SetMask,
    Release,
}

impl ClipAction {
    pub(crate) fn apply(
        self,
        doc: &mut Document,
        selected: &mut Vec<u64>,
        frame: u32,
    ) -> Result<(), String> {
        *selected = match self {
            Self::SetClip => doc.set_clip(selected, frame, None),
            Self::SetMask => doc.set_clip(selected, frame, Some([0, 255])),
            Self::Release => doc.release_clip(selected, frame),
        }?;
        Ok(())
    }
}

pub(crate) fn clip_actions(
    ui: &mut Ui,
    doc: &Document,
    selected: &[u64],
    menu: bool,
) -> Option<ClipAction> {
    let set = doc.can_set_clip(selected);
    let release = doc.can_release_clip(selected);
    if menu || selected.len() > 1 {
        for (action, label, help) in [
            (
                ClipAction::SetClip,
                "Set Clip",
                "Clip the selected artwork to the outline of the topmost selected path or shape.",
            ),
            (
                ClipAction::SetMask,
                "Set Mask",
                "Use the topmost selected path or shape as a left-to-right alpha mask.",
            ),
        ] {
            let button = ui.add_enabled(set, egui::Button::new(label));
            if button
                .on_hover_text(help)
                .on_disabled_hover_text("Select the artwork and a separate path or shape above it.")
                .clicked()
            {
                return Some(action);
            }
        }
    }
    if menu || release {
        let mask = release
            && selected
                .first()
                .and_then(|id| doc.objects.iter().find(|o| o.id == *id))
                .and_then(|o| o.stencil.as_ref())
                .is_some_and(|s| s.alpha.is_some());
        let label = if mask { "Release Mask" } else { "Release Clip" };
        if ui
            .add_enabled(release, egui::Button::new(label))
            .on_hover_text("Reveal the artwork and restore the path as an editable object.")
            .on_disabled_hover_text("Select clipped or masked artwork.")
            .clicked()
        {
            return Some(ClipAction::Release);
        }
    }
    None
}
pub(super) fn pair(ui: &mut Ui, label: &str, values: &mut [f32; 2], minimum: Option<f32>) {
    ui.horizontal(|ui| {
        ui.label(label);
        for (axis, v) in ["X", "Y"].into_iter().zip(values) {
            ui.push_id((label, axis), |ui| number(ui, v, 0.01, "", minimum));
        }
    });
}
fn stencil(ui: &mut Ui, doc: &mut Arc<Document>, index: usize) {
    if doc.objects[index].stencil.is_none() {
        ui.horizontal_wrapped(|ui| {
            for (label, shape) in [
                ("Rectangle clip", Shape::Rectangle),
                ("Ellipse clip", Shape::Ellipse),
            ] {
                if ui.button(label).clicked() {
                    Arc::make_mut(doc).objects[index].stencil = Some(Stencil {
                        fill_rule: Default::default(),
                        shape,
                        origin: [0.1; 2],
                        size: [0.8; 2],
                        alpha: None,
                    });
                }
            }
        });
    }
    let Some(s) = &doc.objects[index].stencil else {
        return;
    };
    let (mut origin, mut size, mut alpha) = (s.origin, s.size, s.alpha);
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(alpha.is_none(), "Clip").clicked() {
            alpha = None;
        }
        if ui.selectable_label(alpha.is_some(), "Alpha mask").clicked() && alpha.is_none() {
            alpha = Some([0, 255]);
        }
    });
    pair(ui, "Offset", &mut origin, None);
    pair(ui, "Size", &mut size, Some(0.001));
    if let Some([left, right]) = &mut alpha {
        alpha_controls(ui, left, right);
    }
    if (origin, size, alpha) != (s.origin, s.size, s.alpha) {
        let s = Arc::make_mut(doc).objects[index].stencil.as_mut().unwrap();
        (s.origin, s.size, s.alpha) = (origin, size, alpha);
    }
}

fn alpha_controls(ui: &mut Ui, left: &mut u8, right: &mut u8) {
    ui.add(egui::Slider::new(left, 0..=255).text("Left alpha"));
    ui.add(egui::Slider::new(right, 0..=255).text("Right alpha"));
}

fn captured_alpha(ui: &mut Ui, doc: &mut Arc<Document>, selected: &[u64]) {
    let Some(first) = doc.objects.iter().find(|o| Some(&o.id) == selected.first()) else {
        return;
    };
    let Some(source) = first.clip_source.as_ref() else {
        return;
    };
    let Some(original) = first.stencil.as_ref().and_then(|s| s.alpha) else {
        return;
    };
    let layer = first.layer;
    let group = first.group;
    let path_id = source.source.object.id;
    let matches = |o: &VectorObject| {
        o.layer == layer
            && o.group == group
            && o.clip_source
                .as_ref()
                .is_some_and(|clip| clip.source.object.id == path_id)
    };
    let selected: std::collections::HashSet<_> = selected.iter().copied().collect();
    if doc
        .objects
        .iter()
        .any(|o| selected.contains(&o.id) && !matches(o))
    {
        return;
    }
    let [mut left, mut right] = original;
    alpha_controls(ui, &mut left, &mut right);
    if [left, right] != original {
        for object in Arc::make_mut(doc)
            .objects
            .iter_mut()
            .filter(|o| matches(o) && (group.is_some() || selected.contains(&o.id)))
        {
            if let Some(s) = &mut object.stencil {
                s.alpha = Some([left, right]);
            }
        }
    }
}
