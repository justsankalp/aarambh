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
    /// Send the schema as JSON Schema (`responseJsonSchema`) rather than
    /// Gemini's OpenAPI subset (`responseSchema`). Flipped, and kept, if
    /// Gemini refuses the first form.
    json_schema: std::sync::atomic::AtomicBool,
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
            json_schema: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn body(req: &Request<'_>, json_schema: bool) -> Value {
        let mut config = json!({
            "responseMimeType": "application/json",
            "maxOutputTokens": req.max_output_tokens,
        });
        if json_schema {
            config["responseJsonSchema"] = req.schema.clone();
        } else {
            config["responseSchema"] = to_gemini_schema(req.schema);
        }
        json!({
            "systemInstruction": { "parts": [{ "text": req.system }] },
            "contents": [{ "role": "user", "parts": [{ "text": req.user }] }],
            "generationConfig": config,
        })
    }

    async fn send(&self, url: &str, body: &Value) -> Result<Value, LlmError> {
        let mut delay = self.backoff;
        let mut last = LlmError::Unavailable("no attempt made".into());
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            let resp = match self
                .http
                .post(url)
                .header("x-goog-api-key", &self.key)
                .json(body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last = LlmError::Unavailable(error_chain(&e));
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
                    let body = resp.text().await.unwrap_or_default();
                    let msg = if body.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", error_message(&body))
                    };
                    LlmError::Unavailable(format!("HTTP {status}{msg}"))
                };
                continue;
            }
            let text = resp
                .text()
                .await
                .map_err(|e| LlmError::Unavailable(error_chain(&e)))?;
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

/// Removes a key everywhere in a schema (for the probe's variants).
fn without(v: &Value, key: &str) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| *k != key)
                .map(|(k, v)| (k.clone(), without(v, key)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| without(v, key)).collect()),
        other => other.clone(),
    }
}

impl Gemini {
    async fn send_once(&self, url: &str, body: &Value) -> String {
        match self
            .http
            .post(url)
            .header("x-goog-api-key", &self.key)
            .json(body)
            .send()
            .await
        {
            Err(e) => format!("no response: {}", error_chain(&e)),
            Ok(r) => {
                let status = r.status().as_u16();
                let text = r.text().await.unwrap_or_default();
                if status == 200 {
                    "ok".into()
                } else {
                    format!("HTTP {status}: {}", error_message(&text))
                }
            }
        }
    }

    /// Sends small requests that add one part of a real request at a time,
    /// and lists the models this key can use, to show what Gemini refuses.
    pub async fn probe(&self, model: &str, schema: &Value) -> Vec<(String, String)> {
        let url = format!("{}/v1beta/models/{model}:generateContent", self.base_url);
        let contents =
            json!([{ "role": "user", "parts": [{ "text": "Reply with a JSON object." }] }]);
        let small = json!({ "type": "OBJECT", "properties": { "ok": { "type": "BOOLEAN" } }, "required": ["ok"] });
        let digest = to_gemini_schema(schema);
        let variants: Vec<(&str, Value)> = vec![
            ("plain text", json!({ "contents": contents })),
            (
                "+ JSON output",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json" } }),
            ),
            (
                "+ small schema",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": small } }),
            ),
            (
                "+ system prompt",
                json!({ "systemInstruction": { "parts": [{ "text": "Be brief." }] }, "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": small } }),
            ),
            (
                "+ maxOutputTokens 8192",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": small, "maxOutputTokens": 8192 } }),
            ),
            (
                "digest schema (responseSchema)",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": digest } }),
            ),
            (
                "digest schema (responseJsonSchema)",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseJsonSchema": schema } }),
            ),
            (
                "digest schema without maxItems",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": without(&digest, "maxItems") } }),
            ),
            (
                "digest schema without propertyOrdering",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": without(&digest, "propertyOrdering") } }),
            ),
            (
                "digest schema without enums",
                json!({ "contents": contents, "generationConfig": { "responseMimeType": "application/json", "responseSchema": without(&without(&digest, "enum"), "format") } }),
            ),
        ];
        let mut out = Vec::new();
        for (name, body) in variants {
            out.push((name.to_owned(), self.send_once(&url, &body).await));
        }
        let models = match self
            .http
            .get(format!("{}/v1beta/models?pageSize=200", self.base_url))
            .header("x-goog-api-key", &self.key)
            .send()
            .await
        {
            Ok(r) => {
                let v: Value = r.json().await.unwrap_or(Value::Null);
                let names: Vec<String> = v["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| {
                        m["name"]
                            .as_str()?
                            .strip_prefix("models/")
                            .map(str::to_owned)
                    })
                    .filter(|n| n.contains("flash"))
                    .collect();
                format!(
                    "{} flash models, {} listed: {}",
                    names.len(),
                    if names.iter().any(|n| n == model) {
                        model
                    } else {
                        "NOT"
                    },
                    names.join(", ")
                )
            }
            Err(e) => format!("no response: {}", error_chain(&e)),
        };
        out.push(("models this key can use".into(), models));
        out
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

/// A reqwest error with its causes ("error sending request" alone says
/// nothing about DNS, TLS or timeouts).
fn error_chain(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        msg.push_str(&format!(": {s}"));
        source = s.source();
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
        use std::sync::atomic::Ordering;
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.base_url, req.model
        );
        let form = self.json_schema.load(Ordering::Relaxed);
        let name = |json: bool| {
            if json {
                "responseJsonSchema"
            } else {
                "responseSchema"
            }
        };
        match self.send(&url, &Self::body(req, form)).await {
            // Refused: try the other way of giving the schema once.
            Err(LlmError::Rejected {
                status: 400,
                message,
            }) => match self.send(&url, &Self::body(req, !form)).await {
                Ok(v) => {
                    eprintln!(
                        "note: Gemini refused {} ({message}); {} worked",
                        name(form),
                        name(!form)
                    );
                    self.json_schema.store(!form, Ordering::Relaxed);
                    Ok(v)
                }
                Err(LlmError::Rejected {
                    status,
                    message: second,
                }) => Err(LlmError::Rejected {
                    status,
                    message: format!(
                        "with {}: {message}; with {}: {second}",
                        name(form),
                        name(!form)
                    ),
                }),
                Err(e) => Err(e),
            },
            other => other,
        }
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
