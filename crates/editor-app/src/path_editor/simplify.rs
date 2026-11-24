use super::{select, KeyMode};
use crate::keymap::{self, Action};
use document_core::{
    arrange::is_object_editable,
    path_edit::{can_simplify_path, simplify_paths},
    Document,
};
use eframe::egui;
use std::collections::HashSet;

pub(crate) fn button(
    ui: &mut egui::Ui,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
    available: bool,
) -> bool {
    let enabled = available && mode != KeyMode::Auto && !selection.is_empty() && {
        let ids: HashSet<_> = selection.iter().copied().collect();
        doc.objects.iter().any(|o| {
            ids.contains(&o.id)
                && o.visible_at(frame)
                && is_object_editable(doc, o)
                && can_simplify_path(o)
        })
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(Action::SimplifyPath.name())
            .shortcut_text(keymap::label(ui.ctx(), Action::SimplifyPath)),
    )
    .on_hover_text(
        "Smooth and refit the path, removing redundant anchors while preserving sharp corners.",
    )
    .on_disabled_hover_text(if mode == KeyMode::Auto {
        "Switch to Tracked or Manual to simplify paths."
    } else {
        "Simplify requires an editable path without Geometry keys, skin weights, or live effects."
    })
    .clicked()
}

pub(crate) fn apply(
    ctx: &egui::Context,
    doc: &mut Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> Option<usize> {
    if mode == KeyMode::Auto {
        return None;
    }
    let edits = simplify_paths(doc, selection, frame);
    if edits.is_empty() {
        return None;
    }
    let mut removed = 0;
    for (id, count) in edits {
        if count > 0 {
            // The former range may now point at different anchors.
            select(ctx, id, frame, 0, 0);
            removed += count;
        }
    }
    Some(removed)
}
