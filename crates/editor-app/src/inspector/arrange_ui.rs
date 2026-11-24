use crate::commands::Alignment;
use crate::editor::arrange::{self, Action, Reference};
use crate::timeline::KeyMode;
use document_core::{arrange::SelectionScope, Document};
use eframe::egui::{self, Ui};
use std::sync::Arc;

#[derive(Clone)]
struct ActionError {
    selection: Arc<[u64]>,
    frame: u32,
    mode: KeyMode,
    reference: Reference,
    message: String,
}

fn units(doc: &Document, ids: &[u64], scope: SelectionScope) -> usize {
    if ids.len() < 2 {
        ids.len()
    } else {
        arrange::available_units_in(doc, ids, scope)
    }
}

fn reference_label(reference: Reference) -> &'static str {
    match reference {
        Reference::BiggestByDirection => "Biggest selected",
        Reference::LargestArea => "Largest area",
        Reference::Bounds => "Selection bounds",
    }
}

fn reference_description(reference: Reference) -> &'static str {
    match reference {
        Reference::BiggestByDirection => "Left, Center X and Right keep the tallest item fixed. Top, Center Y and Bottom keep the widest item fixed.",
        Reference::LargestArea => "Keep the item with the largest bounding box area fixed on both axes.",
        Reference::Bounds => "Align to the combined bounds of the selected items.",
    }
}

fn reference_picker(ui: &mut Ui) -> Reference {
    let id = egui::Id::new("phyro-alignment-reference");
    let previous = ui.data(|d| d.get_temp::<Reference>(id)).unwrap_or_default();
    let mut reference = previous;
    ui.horizontal_wrapped(|ui| {
        let label = ui.label("Relative to");
        egui::ComboBox::from_id_salt("alignment-target")
            .selected_text(reference_label(reference))
            .show_ui(ui, |ui| {
                for target in [
                    Reference::BiggestByDirection,
                    Reference::LargestArea,
                    Reference::Bounds,
                ] {
                    ui.selectable_value(&mut reference, target, reference_label(target))
                        .on_hover_text(reference_description(target));
                }
            })
            .response
            .labelled_by(label.id)
            .on_hover_text(reference_description(reference));
    });
    if reference != previous {
        ui.data_mut(|d| d.insert_temp(id, reference));
    }
    reference
}

fn actions(ui: &mut Ui, count: usize, menu: bool, reference: Reference) -> Option<Action> {
    let mut chosen = None;
    let mut buttons = |ui: &mut Ui, ops: &[(&str, Action, &str)], enabled, disabled: &str| {
        let mut draw = |ui: &mut Ui, row: &[(&str, Action, &str)]| {
            for &(label, action, tip) in row {
                if ui
                    .add_enabled(enabled, egui::Button::new(label))
                    .on_hover_text(tip)
                    .on_disabled_hover_text(disabled)
                    .clicked()
                {
                    chosen = Some(action);
                }
            }
        };
        if menu {
            draw(ui, ops);
        } else {
            for row in ops.chunks(3) {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.;
                    ui.spacing_mut().button_padding.x = 4.;
                    draw(ui, row);
                });
            }
        }
    };
    buttons(
        ui,
        &[
            (
                "Left",
                Action::Align(Alignment::Left, reference),
                "Match left edges",
            ),
            (
                "Center X",
                Action::Align(Alignment::CenterX, reference),
                "Match horizontal centers",
            ),
            (
                "Right",
                Action::Align(Alignment::Right, reference),
                "Match right edges",
            ),
            (
                "Top",
                Action::Align(Alignment::Top, reference),
                "Match top edges",
            ),
            (
                "Center Y",
                Action::Align(Alignment::CenterY, reference),
                "Match vertical centers",
            ),
            (
                "Bottom",
                Action::Align(Alignment::Bottom, reference),
                "Match bottom edges",
            ),
        ],
        count >= if menu { 2 } else { 1 },
        if menu {
            "Select two or more objects or groups"
        } else {
            "Select an object or group"
        },
    );
    ui.separator();
    ui.label("Distribute equal gaps");
    buttons(
        ui,
        &[
            (
                "Horizontal gaps",
                Action::DistributeX,
                "Space objects equally from left to right, keeping both ends fixed",
            ),
            (
                "Vertical gaps",
                Action::DistributeY,
                "Space objects equally from top to bottom, keeping both ends fixed",
            ),
        ],
        count >= 3,
        "Select three or more objects or groups",
    );
    chosen
}

pub(crate) fn menu(
    ui: &mut Ui,
    doc: &Document,
    ids: &[u64],
    scope: SelectionScope,
) -> Option<Action> {
    let count = units(doc, ids, scope);
    let reference = reference_picker(ui);
    actions(ui, count, true, reference)
}

pub(super) fn panel(
    ui: &mut Ui,
    doc: &mut Arc<Document>,
    ids: &[u64],
    frame: u32,
    mode: KeyMode,
    scope: SelectionScope,
) {
    let count = units(doc, ids, scope);
    let reference = if count < 2 {
        ui.horizontal_wrapped(|ui| {
            ui.label("Relative to");
            ui.label("Artboard");
        });
        Reference::Bounds
    } else {
        reference_picker(ui)
    };
    let status = ui.make_persistent_id("arrange-error");
    if let Some(action) = actions(ui, count, false, reference) {
        let result = arrange::apply_in(Arc::make_mut(doc), ids, frame, mode, action, scope);
        ui.data_mut(|d| match result {
            Ok(_) => d.remove::<ActionError>(status),
            Err(message) => d.insert_temp(
                status,
                ActionError {
                    selection: ids.into(),
                    frame,
                    mode,
                    reference,
                    message,
                },
            ),
        });
    }
    if let Some(error) = ui.data(|d| d.get_temp::<ActionError>(status)) {
        if error.selection.as_ref() == ids
            && error.frame == frame
            && error.mode == mode
            && error.reference == reference
        {
            ui.colored_label(ui.visuals().warn_fg_color, error.message);
        } else {
            ui.data_mut(|d| d.remove::<ActionError>(status));
        }
    }
}
