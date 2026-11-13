//! The API's error envelope: an HTTP status, a stable English code, the message as a Msg for the UI and as English
//! text for logs.

use crate::msg::Msg;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Field errors of a request body: dotted path → message.
pub type Fields = BTreeMap<String, Msg>;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub msg: Msg,
    pub fields: Option<Fields>,
    pub details: Option<Box<Value>>,
}

impl ApiError {
    pub fn new(status: u16, code: &'static str, msg: Msg) -> Self {
        ApiError { status, code, msg, fields: None, details: None }
    }

    pub fn bad_request(msg: Msg) -> Self {
        Self::new(400, "bad_request", msg)
    }

    pub fn validation(fields: Fields) -> Self {
        let msg = fields.values().next().cloned().unwrap_or_else(|| msg!("Some of the values are not valid"));
        ApiError { fields: Some(fields), ..Self::new(400, "validation", msg) }
    }

    pub fn field(path: &str, msg: Msg) -> Self {
        let mut f = Fields::new();
        f.insert(path.to_string(), msg);
        Self::validation(f)
    }

    pub fn not_found(msg: Msg) -> Self {
        Self::new(404, "not_found", msg)
    }

    pub fn conflict(msg: Msg) -> Self {
        Self::new(409, "conflict", msg)
    }

    pub fn busy(job_id: Option<&str>) -> Self {
        ApiError {
            details: Some(Box::new(json!({ "job_id": job_id }))),
            ..Self::new(409, "busy", msg!("Another task is running for this instance. Try again when it has finished."))
        }
    }

    pub fn internal(msg: Msg) -> Self {
        Self::new(500, "internal", msg)
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    /// The English sentence (`error.message`).
    pub fn message(&self) -> String {
        self.msg.english()
    }

    pub fn to_json(&self) -> Value {
        let mut e = Map::new();
        e.insert("code".into(), json!(self.code));
        e.insert("message".into(), json!(self.message()));
        e.insert("msg".into(), json!(self.msg));
        if let Some(f) = &self.fields {
            e.insert("fields".into(), json!(f));
        }
        if let Some(d) = &self.details {
            e.insert("details".into(), (**d).clone());
        }
        json!({ "ok": false, "error": e })
    }
}

impl From<crate::docker::DockerError> for ApiError {
    fn from(e: crate::docker::DockerError) -> Self {
        use crate::docker::DockerError;
        match &e {
            DockerError::Unavailable(_) => ApiError::new(503, "docker_unavailable", e.msg()),
            DockerError::Api { status: 404, .. } => ApiError::not_found(e.msg()),
            DockerError::Api { status: 409, .. } => ApiError::conflict(e.msg()),
            _ => ApiError::new(502, "docker_error", e.msg()),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::internal(msg!("Could not read or write the app's data: {error}", error = e.to_string()))
    }
}
