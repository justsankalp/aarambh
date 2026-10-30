//! The model backend, behind a small trait so another provider can be added
//! without touching the judging or digest logic.

use std::fmt;
use std::future::Future;

use serde_json::Value;

/// One structured-output request. `schema` uses lowercase JSON Schema types
/// with `type`, `properties`, `required`, `items`, `enum`, `maxItems`,
/// `minItems` and `description` only; backends translate it as needed.
pub struct Request<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub user: &'a str,
    pub schema: &'a Value,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    MissingKey,
    /// Still rate limited after retrying.
    RateLimited,
    /// Still failing with 5xx or network errors after retrying.
    Unavailable(String),
    /// A request the provider refused (bad model id, bad schema, bad key).
    Rejected {
        status: u16,
        message: String,
    },
    /// The provider declined to answer (safety filters and the like).
    Blocked(String),
    /// The answer wasn't the JSON we asked for.
    BadOutput(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::MissingKey => write!(f, "no API key"),
            LlmError::RateLimited => write!(f, "rate limited; try again later"),
            LlmError::Unavailable(m) => write!(f, "provider unavailable: {m}"),
            LlmError::Rejected { status, message } => {
                write!(f, "request rejected ({status}): {message}")
            }
            LlmError::Blocked(m) => write!(f, "provider declined to answer: {m}"),
            LlmError::BadOutput(m) => write!(f, "unexpected output: {m}"),
        }
    }
}

impl std::error::Error for LlmError {}

pub trait Judge {
    /// Short name for logs and the `model` field, e.g. "gemini".
    fn provider(&self) -> &'static str;

    fn generate_json(&self, req: &Request<'_>) -> impl Future<Output = Result<Value, LlmError>>;
}
