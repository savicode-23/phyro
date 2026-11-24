use document_core::{arrange::is_object_editable, BlendMode, Document, VectorObject};
use eframe::egui::{ComboBox, Ui};
use std::{collections::HashSet, sync::Arc};

fn label(mode: BlendMode) -> String {
    mode.name()
        .split('-')
        .map(|word| format!("{}{}", word[..1].to_ascii_uppercase(), &word[1..]))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn combo(
    ui: &mut Ui,
    caption: &str,
    first: BlendMode,
    mixed: bool,
) -> Option<BlendMode> {
    let mut changed = None;
    ui.horizontal(|ui| {
        super::color_row::number_label(ui, caption);
        ComboBox::from_id_salt(caption)
            .width(ui.available_width().clamp(36., 128.))
            .selected_text(if mixed { "Mixed".into() } else { label(first) })
            .show_ui(ui, |ui| {
                for mode in BlendMode::ALL {
                    if ui
                        .selectable_label(!mixed && first == mode, label(mode))
                        .clicked()
                    {
                        if mixed || first != mode {
                            changed = Some(mode);
                        }
                        ui.close_menu();
                    }
                }
            });
    });
    changed
}

pub(super) fn objects(
    ui: &mut Ui,
    doc: &mut Arc<Document>,
    selected: &[u64],
    scope: super::SelectionScope,
) {
    if selected.is_empty() {
        return;
    }
    let slots: Vec<_> = if let [id] = selected {
        doc.objects
            .iter()
            .position(|o| o.id == *id && is_object_editable(doc, o))
            .into_iter()
            .collect()
    } else {
        let ids: HashSet<_> = scope.resolve(doc, selected).into_iter().collect();
        doc.objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| ids.contains(&o.id).then_some(i))
            .collect()
    };
    let vector_slots: Vec<_> = slots
        .iter()
        .copied()
        .filter(|&slot| super::commands::has_vector_paint(&doc.objects[slot]))
        .collect();
    type Row<'a> = (
        &'static str,
        &'a [usize],
        fn(&VectorObject) -> BlendMode,
        fn(&mut VectorObject) -> &mut BlendMode,
    );
    let rows: [Row<'_>; 3] = [
        (
            "Object blend",
            &slots,
            |o| o.blend_mode,
            |o| &mut o.blend_mode,
        ),
        (
            "Fill blend",
            &vector_slots,
            |o| o.fill_blend_mode,
            |o| &mut o.fill_blend_mode,
        ),
        (
            "Stroke blend",
            &vector_slots,
            |o| o.stroke_blend_mode,
            |o| &mut o.stroke_blend_mode,
        ),
    ];
    for (caption, slots, get, set) in rows {
        if let Some((&first, rest)) = slots.split_first() {
            let mode = get(&doc.objects[first]);
            let mixed = rest.iter().any(|&slot| get(&doc.objects[slot]) != mode);
            if let Some(mode) = combo(ui, caption, mode, mixed) {
                let doc = Arc::make_mut(doc);
                for &slot in slots {
                    *set(&mut doc.objects[slot]) = mode;
                }
            }
        }
    }
}
