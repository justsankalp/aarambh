//! Google Gemini backend (`models.generateContent` with a response schema).

use std::time::Duration;

use serde_json::{Value, json};

use crate::llm::{Judge, LlmError, Request};

pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const MAX_RETRIES: u32 = 4;

pub struct Gemini {
    http: reqwest::Client,
    key: String,
    base_url: String,
    /// First retry delay; doubles each time. Tests set it near zero.
    pub backoff: Duration,
}

impl Gemini {
    pub fn new(key: String, base_url: Option<String>) -> Result<Self, LlmError> {
        if key.trim().is_empty() {
            return Err(LlmError::MissingKey);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::Unavailable(e.to_string()))?;
        Ok(Gemini {
            http,
            key: key.trim().to_owned(),
            base_url: base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.into())
                .trim_end_matches('/')
                .to_owned(),
            backoff: Duration::from_secs(2),
        })
    }

    fn body(req: &Request<'_>) -> Value {
        json!({
            "systemInstruction": { "parts": [{ "text": req.system }] },
            "contents": [{ "role": "user", "parts": [{ "text": req.user }] }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": to_gemini_schema(req.schema),
                "maxOutputTokens": req.max_output_tokens,
            },
        })
    }
}

/// Gemini's schema dialect: uppercase type names, and string enums marked
/// with `format: "enum"`.
pub fn to_gemini_schema(s: &Value) -> Value {
    match s {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let v = match (k.as_str(), v) {
                    ("type", Value::String(t)) => Value::String(t.to_uppercase()),
                    ("properties", Value::Object(props)) => Value::Object(
                        props
                            .iter()
                            .map(|(name, p)| (name.clone(), to_gemini_schema(p)))
                            .collect(),
                    ),
                    ("items", item) => to_gemini_schema(item),
                    (_, other) => other.clone(),
                };
                out.insert(k.clone(), v);
            }
            if map.contains_key("enum") {
                out.insert("format".into(), "enum".into());
            }
            if let Some(Value::Object(props)) = map.get("properties") {
                out.insert(
                    "propertyOrdering".into(),
                    props.keys().cloned().map(Value::String).collect(),
                );
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Pulls the JSON answer out of a generateContent response.
pub fn parse_response(resp: &Value) -> Result<Value, LlmError> {
    let Some(cand) = resp["candidates"].get(0) else {
        let reason = resp["promptFeedback"]["blockReason"]
            .as_str()
            .unwrap_or("no candidates");
        return Err(LlmError::Blocked(reason.to_owned()));
    };
    match cand["finishReason"].as_str().unwrap_or("STOP") {
        "STOP" => {}
        "MAX_TOKENS" => {
            return Err(LlmError::BadOutput(
                "answer was cut off (MAX_TOKENS)".into(),
            ));
        }
        other => return Err(LlmError::Blocked(other.to_owned())),
    }
    let text: String = cand["content"]["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|p| !p["thought"].as_bool().unwrap_or(false))
                .filter_map(|p| p["text"].as_str())
                .collect()
        })
        .unwrap_or_default();
    serde_json::from_str(text.trim()).map_err(|e| LlmError::BadOutput(format!("not JSON: {e}")))
}

/// Google's error message plus any field-level details, which say which
/// part of the request was refused.
pub fn error_message(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return body.chars().take(500).collect();
    };
    let mut msg = v["error"]["message"]
        .as_str()
        .unwrap_or("no message")
        .to_owned();
    for d in v["error"]["details"].as_array().into_iter().flatten() {
        for f in d["fieldViolations"].as_array().into_iter().flatten() {
            let field = f["field"].as_str().unwrap_or("?");
            let why = f["description"].as_str().unwrap_or("");
            msg.push_str(&format!(" [{field}: {why}]"));
        }
        if let Some(r) = d["reason"].as_str() {
            msg.push_str(&format!(" [{r}]"));
        }
    }
    msg
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.min(60)))
}

impl Judge for Gemini {
    fn provider(&self) -> &'static str {
        "gemini"
    }

    async fn generate_json(&self, req: &Request<'_>) -> Result<Value, LlmError> {
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.base_url, req.model
        );
        let body = Self::body(req);
        let mut delay = self.backoff;
        let mut last = LlmError::Unavailable("no attempt made".into());
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            let resp = match self
                .http
                .post(&url)
                .header("x-goog-api-key", &self.key)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last = LlmError::Unavailable(e.to_string());
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if status == 429 || status >= 500 {
                if let Some(wait) = retry_after(&resp) {
                    delay = delay.max(wait);
                }
                last = if status == 429 {
                    LlmError::RateLimited
                } else {
                    LlmError::Unavailable(format!("HTTP {status}"))
                };
                continue;
            }
            let text = resp
                .text()
                .await
                .map_err(|e| LlmError::Unavailable(e.to_string()))?;
            if status != 200 {
                return Err(LlmError::Rejected {
                    status,
                    message: error_message(&text),
                });
            }
            let v: Value =
                serde_json::from_str(&text).map_err(|e| LlmError::BadOutput(e.to_string()))?;
            return parse_response(&v);
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_translated_recursively() {
        let s = json!({
            "type": "object",
            "properties": {
                "edges": { "type": "array", "maxItems": 2, "items": {
                    "type": "object",
                    "properties": { "rel": { "type": "string", "enum": ["a", "b"] } },
                    "required": ["rel"]
                }}
            },
            "required": ["edges"]
        });
        let g = to_gemini_schema(&s);
        assert_eq!(g["type"], "OBJECT");
        assert_eq!(g["propertyOrdering"], json!(["edges"]));
        let rel = &g["properties"]["edges"]["items"]["properties"]["rel"];
        assert_eq!(rel["type"], "STRING");
        assert_eq!(rel["format"], "enum");
        assert_eq!(rel["enum"], json!(["a", "b"]));
        assert_eq!(g["properties"]["edges"]["maxItems"], 2);
    }

    #[test]
    fn error_details_are_kept() {
        let body = r#"{"error":{"code":400,"message":"Request contains an invalid argument.","status":"INVALID_ARGUMENT",
            "details":[{"@type":"type.googleapis.com/google.rpc.BadRequest","fieldViolations":[
            {"field":"generation_config.response_schema","description":"too deep"}]}]}}"#;
        assert_eq!(
            error_message(body),
            "Request contains an invalid argument. [generation_config.response_schema: too deep]"
        );
        assert_eq!(error_message("not json"), "not json");
    }

    #[test]
    fn parses_answers_and_refusals() {
        let ok = json!({"candidates": [{"finishReason": "STOP", "content": {"parts": [
            {"text": "thinking…", "thought": true}, {"text": "{\"a\":"}, {"text": " 1}"}
        ]}}]});
        assert_eq!(parse_response(&ok).unwrap(), json!({"a": 1}));
        let blocked = json!({"promptFeedback": {"blockReason": "SAFETY"}});
        assert_eq!(
            parse_response(&blocked),
            Err(LlmError::Blocked("SAFETY".into()))
        );
        let cut = json!({"candidates": [{"finishReason": "MAX_TOKENS", "content": {"parts": [{"text": "{"}]}}]});
        assert!(matches!(parse_response(&cut), Err(LlmError::BadOutput(_))));
        let junk = json!({"candidates": [{"content": {"parts": [{"text": "Sure! Here you go"}]}}]});
        assert!(matches!(parse_response(&junk), Err(LlmError::BadOutput(_))));
    }
}
