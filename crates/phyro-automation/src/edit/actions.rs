use document_core::{
    arrange::{self, Action, Alignment, Reference},
    typography::TextStyle,
    BooleanOp, Document, PathNode, Property, VectorObject,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    SetFill {
        object_id: u64,
        fill: [u8; 4],
    },
    SetPositionKey {
        object_id: u64,
        frame: u32,
        position: [f32; 2],
    },
    CreatePath {
        layer_id: u64,
        name: String,
        nodes: Vec<Node>,
        closed: bool,
        fill: [u8; 4],
        #[serde(default = "black")]
        stroke: [u8; 4],
        #[serde(default)]
        stroke_width: f32,
    },
    Boolean {
        object_ids: Vec<u64>,
        frame: u32,
        operation: BooleanOp,
    },
    Align {
        object_ids: Vec<u64>,
        frame: u32,
        direction: Alignment,
        #[serde(default)]
        reference: Reference,
    },
    CreateText {
        layer_id: u64,
        name: String,
        position: [f32; 2],
        fill: [u8; 4],
        content: String,
        #[serde(default)]
        font_size: Option<f32>,
    },
    SetText {
        object_id: u64,
        content: String,
        #[serde(default)]
        font_size: Option<f32>,
    },
}

fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Node {
    point: [f32; 2],
    #[serde(default)]
    incoming: Option<[f32; 2]>,
    #[serde(default)]
    outgoing: Option<[f32; 2]>,
}

fn object_index(doc: &Document, id: u64) -> Result<usize, String> {
    let index = doc.objects.iter().position(|o| o.id == id)
        .ok_or_else(|| format!("Object {id} does not exist in this drawing. Look it up using the same clip and exposure."))?;
    if doc.objects[index].locked {
        return Err(format!("Object {id} is locked."));
    }
    if !arrange::is_editable(doc, doc.objects[index].layer) {
        return Err(format!("Object {id} is on a hidden or locked layer."));
    }
    Ok(index)
}

fn selected(doc: &Document, ids: &[u64]) -> Result<(), String> {
    let mut remaining: HashSet<_> = ids.iter().copied().collect();
    if ids.len() != remaining.len() {
        return Err("Use each object ID only once in the selection.".into());
    }
    for object in &doc.objects {
        if remaining.remove(&object.id) && !arrange::is_object_editable(doc, object) {
            return Err(format!(
                "Object {} is locked or on a hidden or locked layer.",
                object.id
            ));
        }
    }
    if let Some(id) = ids.iter().find(|id| remaining.contains(*id)) {
        return Err(format!("Object {id} does not exist in this drawing."));
    }
    Ok(())
}

fn object_mut(doc: &mut Document, id: u64) -> Result<&mut VectorObject, String> {
    let index = object_index(doc, id)?;
    Ok(&mut doc.objects[index])
}

impl Operation {
    pub(super) fn frame(&self) -> Option<u32> {
        match self {
            Self::SetPositionKey { frame, .. }
            | Self::Boolean { frame, .. }
            | Self::Align { frame, .. } => Some(*frame),
            _ => None,
        }
    }

    pub(super) fn execute(self, doc: &mut Document, start: u32) -> Result<Value, String> {
        match self {
            Self::SetFill { object_id, fill } => {
                let object = object_mut(doc, object_id)?;
                if object.is_instance()
                    || object.image.is_some()
                    || (!object.no_fill && object.gradient.is_some())
                {
                    return Err(
                        "Select solid-painted artwork in the source drawing for a fill edit."
                            .into(),
                    );
                }
                if !object.animated.fill_keys.is_empty() || Property::Fill.is_muted(object) {
                    return Err(
                        "This drawing-wide edit requires an unmuted, unkeyed Fill track.".into(),
                    );
                }
                object.fill = fill;
                object.no_fill = false;
                object.gradient = None;
                Ok(json!({"affected_ids": [object_id]}))
            }
            Self::SetPositionKey {
                object_id,
                frame,
                position,
            } => {
                let object = object_mut(doc, object_id)?;
                if Property::Position.is_muted(object) {
                    return Err("Unmute the object's Position track before keying it.".into());
                }
                if object.keys.is_empty() {
                    Property::Position.add(object, start);
                }
                object.set_position_at(frame, position);
                Ok(json!({"affected_ids":[object_id], "frame":frame}))
            }
            Self::CreatePath {
                layer_id,
                name,
                nodes,
                closed,
                fill,
                stroke,
                stroke_width,
            } => {
                let nodes = nodes
                    .into_iter()
                    .map(|n| PathNode {
                        handle_style: Default::default(),
                        point: n.point,
                        incoming: n.incoming.unwrap_or(n.point),
                        outgoing: n.outgoing.unwrap_or(n.point),
                    })
                    .collect();
                let id = doc.add_artboard_path(nodes, closed, layer_id, fill)?;
                let object = doc.objects.last_mut().unwrap();
                object.name = name;
                object.stroke = stroke;
                object.stroke_width = stroke_width;
                Ok(json!({"affected_ids":[id], "created_ids":[id]}))
            }
            Self::Boolean {
                object_ids,
                frame,
                operation,
            } => {
                selected(doc, &object_ids)?;
                let result = doc.boolean(&object_ids, frame, operation)?;
                let old: HashSet<_> = object_ids.iter().copied().collect();
                let remaining: HashSet<_> = result.iter().copied().collect();
                let created: Vec<_> = result.iter().filter(|id| !old.contains(id)).collect();
                let removed: Vec<_> = object_ids
                    .iter()
                    .filter(|id| !remaining.contains(id))
                    .collect();
                Ok(
                    json!({"affected_ids":object_ids, "result_ids":result, "created_ids":created, "removed_ids":removed, "frame":frame}),
                )
            }
            Self::Align {
                object_ids,
                frame,
                direction,
                reference,
            } => {
                selected(doc, &object_ids)?;
                let moves = arrange::position_updates(
                    doc,
                    &object_ids,
                    frame,
                    Action::Align(direction, reference),
                )?;
                let mut affected = Vec::with_capacity(moves.len());
                for (index, position) in moves {
                    let object = &mut doc.objects[index];
                    object.set_position_at(frame, position);
                    affected.push(object.id);
                }
                Ok(json!({"affected_ids":affected, "frame":frame}))
            }
            Self::CreateText {
                layer_id,
                name,
                position,
                fill,
                content,
                font_size,
            } => {
                let mut style = TextStyle {
                    content,
                    ..Default::default()
                };
                if let Some(size) = font_size {
                    style.size = size;
                }
                let id = style.insert(doc, layer_id, position, fill)?;
                doc.objects.last_mut().unwrap().name = name;
                Ok(json!({"affected_ids":[id],"created_ids":[id]}))
            }
            Self::SetText {
                object_id,
                content,
                font_size,
            } => {
                let object = object_mut(doc, object_id)?;
                let mut style = object
                    .text
                    .clone()
                    .ok_or("Choose a text object that still has editable lettering.")?;
                style.content = content;
                if let Some(size) = font_size {
                    style.size = size;
                }
                style.apply_at(object, start)?;
                Ok(json!({"affected_ids":[object_id]}))
            }
        }
    }
}
