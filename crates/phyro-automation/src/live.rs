use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// Only these messages cross the bridge. In particular there is no publish or file operation.
#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "tool",
    content = "arguments",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Request {
    GetContext {},
    Inspect {
        clip_id: u64,
        tick: u32,
        query: Query,
        #[serde(default)]
        offset: usize,
        #[serde(default = "page_size")]
        limit: usize,
    },
    RenderFrame {
        clip_id: u64,
        tick: u32,
    },
    ProposeEdits {
        document: String,
        expected_revision: u64,
        request_id: String,
        operations: Value,
    },
    BeginDraft {
        document: String,
        expected_revision: u64,
        request_id: String,
    },
    AppendDraftStep {
        proposal_id: String,
        step: usize,
        label: String,
        operations: Value,
    },
    FinishDraft {
        proposal_id: String,
        last_step: usize,
    },
    GetProposal {
        proposal_id: String,
    },
}

fn page_size() -> usize {
    32
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    Selection,
    Id { object_id: u64 },
    Name { name: String },
    All,
}

pub type Reply<T = Value> = Result<T, Value>;

pub fn error(code: &str, message: impl ToString) -> Value {
    json!({"code":code,"message":message.to_string(),"retryable":code=="busy"})
}

pub fn schemas() -> Vec<(&'static str, &'static str, Value)> {
    let id = json!({"type":"integer","minimum":1});
    let tick = json!({"type":"integer","minimum":0,"maximum":document_core::MAX_FRAMES-1});
    let string = json!({"type":"string","minLength":1});
    fn object(props: Value, required: &[&str]) -> Value {
        json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
    }
    let operations: Vec<_> = crate::edit::describe()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let mut schema = item["schema"].clone();
            schema["description"] = item["summary"].clone();
            schema
        })
        .collect();
    let query = json!({"oneOf":[
        object(json!({"kind":{"const":"selection"}}), &["kind"]),
        object(json!({"kind":{"const":"all"}}), &["kind"]),
        object(json!({"kind":{"const":"id"},"object_id":id}), &["kind","object_id"]),
        object(json!({"kind":{"const":"name"},"name":string}), &["kind","name"])
    ]});
    vec![
        ("connect","Start pairing or report the existing pairing state.",object(json!({}),&[])),
        ("get_context","Read the shared tab, current clip, playhead, active layer and selected drawing scope.",object(json!({}),&[])),
        ("inspect","Inspect evaluated artwork at a zero-based tick. Exact names can return several candidates. Continue with next_offset.",object(json!({"clip_id":id,"tick":tick,"query":query,"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":64}}),&["clip_id","tick","query"])),
        ("render_frame","Render the committed artboard, including visible references, as an in-memory PNG.",object(json!({"clip_id":id,"tick":tick}), &["clip_id","tick"])),
        ("propose_edits","Stage an ordered atomic batch for the artist. Reuse the same request_id when retrying. Operation frame values are zero-based clip ticks.",object(json!({"document":string,"expected_revision":{"type":"integer","minimum":0},"request_id":{"type":"string","minLength":1,"maxLength":128},"operations":{"type":"array","minItems":1,"maxItems":128,"items":{"oneOf":operations}}}), &["document","expected_revision","request_id","operations"])),
        ("begin_draft","Start a live drawing at the committed revision. Retry with the same request_id.",object(json!({"document":string,"expected_revision":{"type":"integer","minimum":0},"request_id":{"type":"string","minLength":1,"maxLength":128}}), &["document","expected_revision","request_id"])),
        ("append_draft_step","Validate the next numbered visual change. Returned IDs belong to the indicated clip and exposure. Retry with identical content.",object(json!({"proposal_id":string,"step":id,"label":string,"operations":{"type":"array","minItems":1,"maxItems":128,"items":{"oneOf":operations}}}), &["proposal_id","step","label","operations"])),
        ("finish_draft","Seal the last validated step for artist review.",object(json!({"proposal_id":string,"last_step":id}), &["proposal_id","last_step"])),
        ("get_proposal","Read the review decision and committed operation results.",object(json!({"proposal_id":string}), &["proposal_id"])),
    ]
}
