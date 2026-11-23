use crate::CliError;
use document_core::{
    AnimationProject, Clip, Document, KeyValue, Layer, Property, Shape, VectorObject,
};
use serde_json::{json, Map, Value};
use std::{collections::HashMap, ops::Bound};

pub fn clip_by_id(project: &AnimationProject, id: u64) -> Result<&Clip, CliError> {
    project
        .clip_index(id)
        .map(|index| &project.clips[index])
        .ok_or_else(|| {
            CliError::new(
                "unknown_clip",
                format!("No clip has ID {id}. Use inspect to list clip IDs."),
            )
            .with_details(json!({"clip_id": id}))
        })
}

fn selected_clips(
    project: &AnimationProject,
    clip_id: Option<u64>,
    frame: Option<u32>,
) -> Result<impl Iterator<Item = &Clip>, CliError> {
    let clips = match clip_id {
        Some(id) => std::slice::from_ref(clip_by_id(project, id)?),
        None => project.clips.as_slice(),
    };
    if let Some(tick) = frame {
        if clips.iter().all(|clip| tick >= clip.frames) {
            let lengths: Vec<_> = clips
                .iter()
                .map(|c| json!({"clip_id": c.id, "frames": c.frames}))
                .collect();
            return Err(CliError::new(
                "frame_out_of_range",
                format!(
                    "Frame {tick} is outside the selected clips. Frame numbering starts at zero."
                ),
            )
            .with_details(json!({"frame": tick, "clips": lengths})));
        }
    }
    Ok(clips
        .iter()
        .filter(move |clip| frame.is_none_or(|tick| tick < clip.frames)))
}

#[derive(Clone, Copy)]
struct Exposure<'a> {
    clip: &'a Clip,
    start: u32,
    duration: u32,
    drawing: &'a Document,
}

// Pair each drawing start with its successor. A held drawing is visited once,
// regardless of its duration. A frame filter looks up only the containing cel.
fn exposures(clip: &Clip, frame: Option<u32>) -> impl Iterator<Item = Exposure<'_>> {
    let (first, last) = match frame {
        Some(tick) => {
            let (start, _) = clip.exposure(tick);
            (start, start)
        }
        None => (0, clip.frames - 1),
    };
    let ends = clip
        .cels
        .range((Bound::Excluded(first), Bound::Unbounded))
        .map(|(&start, _)| start)
        .chain(std::iter::once(clip.frames));
    clip.cels
        .range(first..=last)
        .zip(ends)
        .map(move |((&start, drawing), end)| Exposure {
            clip,
            start,
            duration: end - start,
            drawing,
        })
}

pub fn inspect(
    project: &AnimationProject,
    clip_id: Option<u64>,
    frame: Option<u32>,
) -> Result<Value, CliError> {
    let mut summaries = Vec::new();
    for clip in selected_clips(project, clip_id, frame)? {
        let page = clip.drawing_at(0);
        let mut summary = json!({
            "clip_id": clip.id, "set_id": clip.set_id, "name": clip.name, "frames": clip.frames,
            "include_in_set_export": clip.include_in_set_export,
            "fps": clip.fps, "looping": clip.looping, "pivot": clip.origin,
            "artboard": {"width": page.width, "height": page.height},
            "exposure_count": clip.cels.len(), "events": clip.events,
        });
        let mut drawings = Vec::new();
        for exposure in exposures(clip, frame) {
            let doc = exposure.drawing;
            drawings.push(json!({
                "exposure_start": exposure.start, "exposure_duration": exposure.duration,
                "object_count": doc.objects.len(), "layer_count": doc.layers.len(),
                "bone_count": doc.skeleton.bones.len(),
            }));
            if let Some(tick) = frame {
                summary["drawing"] = drawing_report(exposure, tick)?;
            }
        }
        summary["exposures"] = Value::Array(drawings);
        summaries.push(summary);
    }
    Ok(json!({
        "coordinates": {
            "unit": "artboard_pixels", "origin": "top_left", "x_axis": "right", "y_axis": "down",
            "rotation": "clockwise_degrees", "frame_base": 0, "color": "unpremultiplied_rgba8",
        },
        "animation_sets": project.animation_sets,
        "clips": summaries,
    }))
}

fn drawing_report(exposure: Exposure<'_>, tick: u32) -> Result<Value, CliError> {
    let doc = exposure.drawing;
    let layers_by_id: HashMap<_, _> = doc.layers.iter().map(|layer| (layer.id, layer)).collect();
    let layers: Vec<_> = doc
        .layers
        .iter()
        .map(|l| {
            json!({
                "layer_id": l.id, "name": l.name, "locked": l.locked, "visible": l.visible,
            })
        })
        .collect();
    let rotations = doc
        .skeleton
        .posed_rotations_at(tick)
        .map_err(|error| CliError::new("invalid_rig", error))?;
    let bones: Vec<_> = doc
        .skeleton
        .bones
        .iter()
        .map(|b| {
            json!({
                "bone_id": b.id, "name": b.name, "parent_id": b.parent, "rest_head": b.head,
                "rest_tail": b.tail, "rotation": rotations[&b.id], "rotation_keys": b.rotation_keys,
                "foot_pin": doc.foot_pin_at(b.id, tick),
            })
        })
        .collect();
    let objects: Vec<_> = doc
        .objects
        .iter()
        .map(|o| object_report(exposure, o, layers_by_id[&o.layer], tick))
        .collect();
    Ok(json!({
        "frame": tick, "exposure_start": exposure.start, "exposure_duration": exposure.duration,
        "layers": layers, "bones": bones, "objects": objects,
    }))
}

/// Evaluate only the requested page. A selection is valid solely in the drawing
/// from which its IDs were captured.
pub fn page(
    clip: &Clip,
    tick: u32,
    query: &crate::live::Query,
    selection: &[u64],
    offset: usize,
    limit: usize,
) -> Result<Value, CliError> {
    use crate::live::Query;
    if tick >= clip.frames || !(1..=64).contains(&limit) {
        return Err(CliError::new(
            "invalid_arguments",
            "Check the clip tick and choose a page size from 1 to 64.",
        ));
    }
    let exposure = exposures(clip, Some(tick))
        .next()
        .expect("valid clip has a drawing");
    let doc = exposure.drawing;
    let mut objects = Vec::new();
    let mut total = 0;
    for obj in &doc.objects {
        let matches = match query {
            Query::All => true,
            Query::Selection => selection.contains(&obj.id),
            Query::Id { object_id } => *object_id == obj.id,
            Query::Name { name } => *name == obj.name,
        };
        if !matches {
            continue;
        }
        if total >= offset && objects.len() < limit {
            let layer = doc
                .layers
                .iter()
                .find(|l| l.id == obj.layer)
                .expect("validated layer");
            let mut item = object_report(exposure, obj, layer, tick);
            item["keys_at_frame"] = keys_at_frame(obj, tick);
            item["layer_locked"] = json!(layer.locked);
            item["layer_visible"] = json!(layer.visible);
            let (start, duration) = clip.layer_exposure(tick, layer.id);
            item["layer_hold"] = json!({"start":start,"duration":duration});
            objects.push(item);
        }
        total += 1;
    }
    let end = offset.saturating_add(objects.len());
    Ok(
        json!({"objects":objects,"total":total,"ambiguous":matches!(query, Query::Name {..}) && total > 1,
        "exposure_start":exposure.start,"exposure_duration":exposure.duration,"next_offset":(end < total).then_some(end)}),
    )
}

fn object_kind(object: &VectorObject) -> &'static str {
    if object.image.is_some() {
        return "image";
    }
    if object.clip_instance.is_some() {
        return "nested_clip";
    }
    if object.symbol.is_some() {
        return "symbol";
    }
    if object.embedded_artwork.is_some() {
        return "independent_artwork";
    }
    if object.text.is_some() {
        return "text";
    }
    match object.shape {
        Shape::Rectangle => "rectangle",
        Shape::Ellipse => "ellipse",
        Shape::Path { .. } => "path",
        Shape::Compound { .. } => "compound_path",
    }
}

fn object_scope(exposure: Exposure<'_>, o: &VectorObject) -> Value {
    json!({
        "clip_id": exposure.clip.id, "clip_name": exposure.clip.name,
        "exposure_start": exposure.start, "exposure_duration": exposure.duration,
        "layer_id": o.layer, "object_id": o.id, "group_id": o.group, "name": o.name, "kind": object_kind(o),
    })
}

fn property_name(property: Property) -> &'static str {
    match property {
        Property::Position => "position",
        Property::Rotation => "rotation",
        Property::Scale => "scale",
        Property::Opacity => "opacity",
        Property::Fill => "fill",
        Property::Stroke => "stroke",
        Property::StrokeWidth => "stroke_width",
        Property::Visibility => "visibility",
        Property::Skew => "skew",
        Property::Geometry => "geometry",
        Property::MotionProgress => "motion_progress",
    }
}

fn keys_at_frame(object: &VectorObject, tick: u32) -> Value {
    let mut keys = Map::new();
    for property in Property::ALL {
        let Some(key) = property.get(object, tick) else {
            continue;
        };
        let value = match key {
            KeyValue::Pair(pair) => json!(pair),
            KeyValue::Scalar(scalar) => json!(scalar),
            KeyValue::Color(color) => json!(color),
            KeyValue::Visible(visible) => json!(visible),
            KeyValue::Shape(shape) => json!(shape),
        };
        keys.insert(
            property_name(property).into(),
            json!({
                "value": value, "outgoing_easing": property.easing_at(object, tick),
            }),
        );
    }
    if let Some(key) = object
        .clip_instance
        .as_ref()
        .and_then(|instance| instance.drawing_keys.get(&tick))
    {
        keys.insert(
            "child_drawing".into(),
            json!({"value": key, "outgoing_easing": "Hold"}),
        );
    }
    Value::Object(keys)
}

fn object_report(exposure: Exposure<'_>, o: &VectorObject, layer: &Layer, tick: u32) -> Value {
    let mut report = object_scope(exposure, o);
    let mut editable = Vec::new();
    if layer.visible && !layer.locked {
        if !Property::Position.is_muted(o) {
            editable.push("position");
        }
        if !o.is_instance()
            && o.image.is_none()
            && (o.no_fill || o.gradient.is_none())
            && o.animated.fill_keys.is_empty()
            && !Property::Fill.is_muted(o)
        {
            editable.push("fill");
        }
        if o.text.is_some() {
            editable.push("text");
        }
    }
    let mut keyframes = Map::new();
    for p in Property::ALL {
        if p.last(o).is_some() {
            keyframes.insert(property_name(p).into(), json!(p.frames(o)));
        }
    }
    if let Some(child) = &o.clip_instance {
        report["source_clip_id"] = json!(child.clip_id);
        if !child.drawing_keys.is_empty() {
            keyframes.insert(
                "child_drawing".into(),
                json!(child.drawing_keys.keys().collect::<Vec<_>>()),
            );
        }
    }
    if let Some(symbol) = &o.symbol {
        report["symbol_id"] = json!(symbol.definition);
    }
    if let Some(text) = &o.text {
        report["text"] = json!(text);
    }
    if let Some(image) = &o.image {
        report["image_size"] = json!([image.width(), image.height()]);
        report["include_in_export"] = json!(!o.reference_only);
    }
    report["layer_name"] = json!(layer.name);
    report["sampled_frame"] = json!(tick);
    report["position"] = json!(o.position_at(tick));
    report["base_position"] = json!(o.position);
    report["size"] = json!(o.size);
    report["rotation"] = json!(o.rotation_at(tick));
    report["scale"] = json!(o.scale_at(tick));
    report["opacity"] = json!(o.opacity_at(tick));
    report["fill"] = json!(o.fill_at(tick));
    report["base_fill"] = json!(o.fill);
    report["no_fill"] = json!(o.no_fill);
    report["no_stroke"] = json!(o.no_stroke);
    report["stroke"] = json!(o.stroke_at(tick));
    report["stroke_width"] = json!(o.stroke_width_at(tick));
    report["visible"] = json!(layer.visible && o.visible_at(tick));
    report["keyframes"] = Value::Object(keyframes);
    report["muted_properties"] = json!(o
        .animated
        .muted
        .iter()
        .copied()
        .map(property_name)
        .collect::<Vec<_>>());
    report["editable_properties"] = json!(editable);
    report
}

pub fn lookup(
    project: &AnimationProject,
    name: Option<&str>,
    object_id: Option<u64>,
    clip_id: Option<u64>,
    frame: Option<u32>,
) -> Result<Value, CliError> {
    if name.is_some() == object_id.is_some() || name == Some("") {
        return Err(CliError::new(
            "invalid_arguments",
            "Specify one nonempty object --name or one --object-id.",
        ));
    }
    let mut matches = Vec::new();
    for clip in selected_clips(project, clip_id, frame)? {
        for exposure in exposures(clip, frame) {
            for object in &exposure.drawing.objects {
                if name == Some(&object.name) || object_id == Some(object.id) {
                    matches.push((exposure, object));
                }
            }
        }
    }
    match matches.as_slice() {
        [] => Err(CliError::new("object_not_found", "No object matches this selector. Use inspect with --clip and --frame to list object names and IDs.")
            .with_details(json!({"name": name, "object_id": object_id, "clip_id": clip_id, "frame": frame}))),
        [(exposure, object)] => {
            let layer = exposure.drawing.layers.iter().find(|l| l.id == object.layer)
                .expect("loaded artwork has valid layers");
            let tick = frame.unwrap_or(exposure.start);
            let mut report = object_report(*exposure, object, layer, tick);
            report["keys_at_frame"] = keys_at_frame(object, tick);
            Ok(report)
        }
        _ => Err(CliError::new("ambiguous_object", "Several objects match. Select a match by --clip, --frame and --object-id.")
            .with_details(json!({"matches": matches.iter().map(|(e,o)| object_scope(*e, o)).collect::<Vec<_>>()}))),
    }
}

#[cfg(test)]
mod tests;
