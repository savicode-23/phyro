//! Apply versioned operations to an isolated copy of the native project.
mod actions;
mod discovery;
#[cfg(test)]
mod tests;

use crate::CliError;
use actions::Operation;
use document_core::AnimationProject;
use serde::Deserialize;
use serde_json::{json, Value};

pub use discovery::describe;

#[derive(Deserialize)]
struct Edit {
    clip_id: u64,
    exposure_start: u32,
    #[serde(flatten)]
    operation: Operation,
}

fn failure(index: usize, input: &Value, message: impl Into<String>) -> CliError {
    let mut scope = json!({"operation_index": index});
    for field in [
        "op",
        "clip_id",
        "exposure_start",
        "frame",
        "object_id",
        "object_ids",
        "layer_id",
    ] {
        if let Some(value) = input.get(field) {
            scope[field] = value.clone();
        }
    }
    CliError::new("invalid_operation", message).with_details(scope)
}

pub fn apply(project: &mut AnimationProject, operations: &Value) -> Result<Value, CliError> {
    let edits = operations
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CliError::new("invalid_batch", "Provide a nonempty operations array."))?;
    let mut draft = project.clone();
    let mut results = Vec::with_capacity(edits.len());
    let mut previous_clip = None;
    for (index, input) in edits.iter().enumerate() {
        let error = |message: String| failure(index, input, message);
        let edit: Edit = serde_json::from_value(input.clone()).map_err(|e| error(e.to_string()))?;
        let clip_index = draft.clip_index(edit.clip_id).ok_or_else(|| {
            error(format!(
                "No clip with ID {}. Inspect the project for its clip IDs.",
                edit.clip_id
            ))
        })?;
        // A parent evaluated later in a batch must see earlier changes to its child clip.
        if previous_clip.is_some_and(|id| id != edit.clip_id) {
            draft.hydrate_symbols().map_err(error)?;
        }
        let clip = &mut draft.clips[clip_index];
        if !clip.cels.contains_key(&edit.exposure_start) {
            return Err(error(format!("No drawing starts at tick {} in clip {}. Use the exposure_start returned by lookup.", edit.exposure_start, edit.clip_id)));
        }
        let (start, duration) = clip.exposure(edit.exposure_start);
        if let Some(frame) = edit.operation.frame() {
            if !(start..start + duration).contains(&frame) {
                return Err(error(format!("Frame {frame} is outside this drawing's exposure ({start} through {}). Look up the drawing that holds at the requested frame.", start + duration - 1)));
            }
        }
        let before = (!clip.layer_starts.is_empty()).then(|| clip.clone());
        let doc = clip.drawing_at_mut(start);
        let mut result = edit.operation.execute(doc, start).map_err(error)?;
        doc.validate().map_err(error)?;
        if let Some(before) = before {
            clip.sync_layer_edits(&before, start).map_err(error)?;
        }
        result["operation_index"] = json!(index);
        result["clip_id"] = json!(edit.clip_id);
        result["exposure_start"] = json!(start);
        result["exposure_duration"] = json!(duration);
        result["op"] = input["op"].clone();
        results.push(result);
        previous_clip = Some(edit.clip_id);
    }
    draft
        .hydrate_symbols()
        .and_then(|()| draft.validate())
        .map_err(|message| failure(edits.len() - 1, &edits[edits.len() - 1], message))?;
    *project = draft;
    Ok(json!(results))
}
