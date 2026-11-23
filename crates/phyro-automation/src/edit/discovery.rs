use serde_json::{json, Value};

fn argument(field: &str) -> Value {
    let pair = json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2});
    match field {
        "clip_id" | "layer_id" | "object_id" => json!({"type":"integer","minimum":1}),
        "exposure_start" | "frame" => {
            json!({"type":"integer","minimum":0,"maximum":document_core::MAX_FRAMES-1})
        }
        "object_ids" => {
            json!({"type":"array","uniqueItems":true,"minItems":1,"items":{"type":"integer","minimum":1}})
        }
        "fill" | "stroke" => {
            json!({"type":"array","minItems":4,"maxItems":4,"items":{"type":"integer","minimum":0,"maximum":255}})
        }
        "position" => pair,
        "name" => json!({"type":"string","maxLength":80}),
        "content" => json!({"type":"string","minLength":1,"maxLength":2048}),
        "closed" => json!({"type":"boolean"}),
        "nodes" => {
            json!({"type":"array","minItems":2,"maxItems":document_core::MAX_PATH_NODES,"items":{
                "type":"object","required":["point"],"additionalProperties":false,
                "properties":{"point":pair,"incoming":pair,"outgoing":pair}
            }})
        }
        "stroke_width" => json!({"type":"number","minimum":0}),
        "font_size" => json!({"type":"number","minimum":0.25,"maximum":4096}),
        "direction" => json!({"enum":["left","center_x","right","top","center_y","bottom"]}),
        "reference" => json!({"enum":["biggest","largest_area","bounds"]}),
        "operation" => {
            json!({"enum":["union","difference","intersection","exclusion","division","cut_path"]})
        }
        _ => unreachable!("every documented operation argument has a schema"),
    }
}

pub fn describe() -> Value {
    fn entry(op: &str, summary: &str, required: &[&str], optional: Value, fields: Value) -> Value {
        let mut example = json!({"op":op, "clip_id":1, "exposure_start":0});
        example
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let required: Vec<_> = ["op", "clip_id", "exposure_start"]
            .into_iter()
            .chain(required.iter().copied())
            .collect();
        let mut properties = json!({"op":{"const":op}});
        for field in required
            .iter()
            .copied()
            .filter(|&f| f != "op")
            .chain(optional.as_object().unwrap().keys().map(String::as_str))
        {
            properties[field] = argument(field);
        }
        let mut result_fields = vec![
            "op",
            "operation_index",
            "clip_id",
            "exposure_start",
            "exposure_duration",
            "affected_ids",
        ];
        if required.contains(&"frame") {
            result_fields.push("frame");
        }
        match op {
            "create_path" | "create_text" => result_fields.push("created_ids"),
            "boolean" => result_fields.extend(["created_ids", "result_ids", "removed_ids"]),
            _ => {}
        }
        json!({"op":op,"summary":summary,"required":required,"optional":optional,"example":example,"result_fields":result_fields,
            "schema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }
    let operations = [
        entry(
            "set_fill",
            "Set a solid RGBA fill throughout this exposure. The artwork can have a solid fill or no fill. Requires a visible, unlocked layer and an unmuted, unkeyed Fill track on a vector object in the source drawing.",
            &["object_id","fill"],
            json!({}),
            json!({"object_id":20,"fill":[220,88,54,255]}),
        ),
        entry(
            "set_position_key",
            "Set the evaluated object position in artboard pixels at an absolute clip tick within this exposure. The first key preserves the previous pose at the start of the exposure. Requires an unmuted Position track.",
            &["object_id","frame","position"],
            json!({}),
            json!({"object_id":20,"frame":4,"position":[63,60]}),
        ),
        entry(
            "create_path",
            "Create an editable Bézier path on a visible, unlocked layer. Provide at least two nodes for an open path or three for a closed path. Each node has a point and optional absolute incoming/outgoing control points in artboard pixels. Omitted handles coincide with the point.",
            &["layer_id","name","nodes","closed","fill"],
            json!({"stroke":{"default":[0,0,0,255]},"stroke_width":{"default":0,"units":"pixels"}}),
            json!({"layer_id":1,"name":"Triangle","nodes":[{"point":[8,8]},{"point":[24,8]},{"point":[24,24]}],"closed":true,"fill":[220,88,54,255]}),
        ),
        entry(
            "boolean",
            "Bake evaluated vector operands in their paint order. operation accepts union, difference, intersection, exclusion, division or cut_path. The last two require exactly two objects. Read result_ids, created_ids and removed_ids.",
            &["object_ids","frame","operation"],
            json!({}),
            json!({"object_ids":[3,5],"frame":0,"operation":"difference"}),
        ),
        entry(
            "align",
            "Align evaluated visual bounds. direction accepts left, center_x, right, top, center_y or bottom. Groups move together. Position follows the editor's Tracked mode. Biggest uses the tallest object for horizontal moves and the widest for vertical moves. Bounds uses the artboard for one unit or the combined bounds for multiple units.",
            &["object_ids","frame","direction"],
            json!({"reference":{"enum":["biggest","largest_area","bounds"],"default":"biggest"}}),
            json!({"object_ids":[3,5],"frame":0,"direction":"center_x"}),
        ),
        entry(
            "create_text",
            "Create editable multiline lettering at a top-left artboard position. The default typeface is Ubuntu Light.",
            &["layer_id","name","position","fill","content"],
            json!({"font_size":{"default":24,"units":"pixels"}}),
            json!({"layer_id":1,"name":"Caption","position":[8,8],"fill":[0,0,0,255],"content":"Lantern Walk","font_size":18}),
        ),
        entry(
            "set_text",
            "Edit existing lettering across its exposure, preserving the existing face, layout, transform and paint.",
            &["object_id","content"],
            json!({"font_size":{"default":"keep existing","units":"pixels"}}),
            json!({"object_id":6,"content":"Night walk"}),
        ),
    ];
    Value::Array(operations.into())
}
