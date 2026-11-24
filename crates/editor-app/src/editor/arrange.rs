#[cfg(test)]
use crate::commands::{self, Alignment};
use crate::timeline::KeyMode;
#[cfg(test)]
pub(crate) use document_core::arrange::available_units;
use document_core::arrange::SelectionScope;
pub(crate) use document_core::arrange::{available_units_in, Action, Reference};
use document_core::{Document, Property};

#[cfg(test)]
pub(crate) fn apply(
    doc: &mut Document,
    ids: &[u64],
    frame: u32,
    mode: KeyMode,
    action: Action,
) -> Result<bool, String> {
    apply_in(doc, ids, frame, mode, action, SelectionScope::Groups)
}

pub(crate) fn apply_in(
    doc: &mut Document,
    ids: &[u64],
    frame: u32,
    mode: KeyMode,
    action: Action,
    scope: SelectionScope,
) -> Result<bool, String> {
    let updates = document_core::arrange::position_updates_in(doc, ids, frame, action, scope)?;
    if updates
        .iter()
        .any(|(index, _)| !mode.allows(&doc.objects[*index], frame, Property::Position))
    {
        return Err(
            "A moving object’s Position track is muted or has no key at this frame in Manual mode."
                .into(),
        );
    }
    let changed = !updates.is_empty();
    for (index, position) in updates {
        let object = &mut doc.objects[index];
        mode.prepare(object, frame, Property::Position);
        object.set_position_at(frame, position);
    }
    Ok(changed)
}

#[cfg(test)]
mod posed_tests;
#[cfg(test)]
mod tests;
