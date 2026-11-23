pub mod edit;
pub mod inspect;
pub mod ipc;
pub mod live;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct CliError {
    pub code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}
