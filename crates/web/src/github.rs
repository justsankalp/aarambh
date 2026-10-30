//! GitHub REST Contents API client. The token is sent only to
//! api.github.com, and only in the Authorization header.

use aarambh_core::sync::{MAX_RETRIES, RepoRef, backoff_ms, commit_message};
use aarambh_core::{Day, Event, append_to_day_file};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use leptos::prelude::window;
use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestCache, RequestInit, RequestMode, Response};

const API_VERSION: &str = "2026-03-10";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhError {
    /// 401: the token is wrong, expired or revoked.
    Unauthorized,
    /// 403 that isn't rate limiting: the token lacks a permission.
    Forbidden,
    /// 404 on the repo itself (also what GitHub says when the token can't see it).
    NotFound,
    RateLimited,
    /// 409/422 on a write: someone else wrote the file first.
    Conflict,
    /// No response at all: offline, DNS, CORS.
    Network,
    Http(u16),
    /// The response or the file in the repo couldn't be read.
    BadData(String),
}

impl GhError {
    pub fn message(&self) -> String {
        match self {
            GhError::Unauthorized => "GitHub rejected the token. Check it hasn't expired.".into(),
            GhError::Forbidden => {
                "The token can't do this. It needs Contents and Actions read/write on this repo."
                    .into()
            }
            GhError::NotFound => "Repo not found, or the token doesn't have access to it.".into(),
            GhError::RateLimited => "GitHub rate limit reached. Try again in a few minutes.".into(),
            GhError::Conflict => "Someone else saved at the same moment. Try again.".into(),
            GhError::Network => "Can't reach GitHub. You may be offline.".into(),
            GhError::Http(code) => format!("GitHub returned an error ({code})."),
            GhError::BadData(why) => format!("Couldn't read data from GitHub: {why}"),
        }
    }

    /// Errors worth retrying later without the person doing anything.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            GhError::Network | GhError::RateLimited | GhError::Conflict | GhError::Http(500..)
        )
    }
}

#[derive(Clone, Debug)]
pub struct Gh {
    repo: RepoRef,
    token: String,
}

struct Resp {
    status: u16,
    etag: Option<String>,
    rate_limit_left: Option<String>,
    body: String,
}

/// A file read from the repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoFile {
    pub text: String,
    pub sha: String,
    pub etag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched<T> {
    /// 304: unchanged since the ETag we sent.
    NotModified,
    Missing,
    Found(T),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Deserialize)]
struct ContentResp {
    sha: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    encoding: String,
}

#[derive(Deserialize)]
struct PutResp {
    content: PutContent,
}

#[derive(Deserialize)]
struct PutContent {
    sha: String,
}

#[derive(Deserialize)]
pub struct RepoInfo {
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
}

impl Gh {
    pub fn new(repo: RepoRef, token: String) -> Self {
        Gh { repo, token }
    }

    async fn send(
        &self,
        method: &str,
        url: &str,
        body: Option<&str>,
        etag: Option<&str>,
    ) -> Result<Resp, GhError> {
        let headers = Headers::new().map_err(|_| GhError::Network)?;
        let set = |k: &str, v: &str| {
            headers
                .set(k, v)
                .map_err(|_| GhError::BadData(format!("bad header {k}")))
        };
        set("Accept", "application/vnd.github+json")?;
        set("Authorization", &format!("Bearer {}", self.token.trim()))?;
        set("X-GitHub-Api-Version", API_VERSION)?;
        if let Some(e) = etag {
            set("If-None-Match", e)?;
        }
        let init = RequestInit::new();
        init.set_method(method);
        init.set_mode(RequestMode::Cors);
        // We keep our own ETags; the HTTP cache would hide fresh writes for 60 s.
        init.set_cache(RequestCache::NoStore);
        if let Some(b) = body {
            set("Content-Type", "application/json")?;
            init.set_body(&JsValue::from_str(b));
        }
        init.set_headers(&headers);
        let req = Request::new_with_str_and_init(url, &init).map_err(|_| GhError::Network)?;
        let resp: Response = JsFuture::from(window().fetch_with_request(&req))
            .await
            .map_err(|_| GhError::Network)?
            .unchecked_into();
        let h = resp.headers();
        let text_promise = resp.text().map_err(|_| GhError::Network)?;
        let body = JsFuture::from(text_promise)
            .await
            .map_err(|_| GhError::Network)?
            .as_string()
            .unwrap_or_default();
        Ok(Resp {
            status: resp.status(),
            etag: h.get("etag").ok().flatten(),
            rate_limit_left: h.get("x-ratelimit-remaining").ok().flatten(),
            body,
        })
    }

    fn error_for(r: &Resp) -> GhError {
        match r.status {
            401 => GhError::Unauthorized,
            403 | 429 if r.status == 429 || r.rate_limit_left.as_deref() == Some("0") => {
                GhError::RateLimited
            }
            403 => GhError::Forbidden,
            404 => GhError::NotFound,
            409 | 422 => GhError::Conflict,
            s => GhError::Http(s),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/contents/{path}", self.repo.api_base())
    }

    /// Checks the repo exists and the token can see it.
    pub async fn check_repo(&self) -> Result<RepoInfo, GhError> {
        let r = self.send("GET", &self.repo.api_base(), None, None).await?;
        if r.status != 200 {
            return Err(Self::error_for(&r));
        }
        serde_json::from_str(&r.body).map_err(|e| GhError::BadData(e.to_string()))
    }

    pub async fn get_file(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> Result<Fetched<RepoFile>, GhError> {
        let r = self.send("GET", &self.url(path), None, etag).await?;
        match r.status {
            304 => return Ok(Fetched::NotModified),
            404 => return Ok(Fetched::Missing),
            200 => {}
            _ => return Err(Self::error_for(&r)),
        }
        let c: ContentResp =
            serde_json::from_str(&r.body).map_err(|e| GhError::BadData(e.to_string()))?;
        if c.encoding != "base64" {
            // Files over 1 MB come back without content.
            return Err(GhError::BadData(format!("{path} is too large to read")));
        }
        let packed: String = c.content.split_ascii_whitespace().collect();
        let bytes = B64
            .decode(packed)
            .map_err(|e| GhError::BadData(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|e| GhError::BadData(e.to_string()))?;
        Ok(Fetched::Found(RepoFile {
            text,
            sha: c.sha,
            etag: r.etag,
        }))
    }

    /// Lists a directory. Returns the ETag so unchanged listings cost a 304.
    pub async fn list_dir(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> Result<Fetched<(Vec<DirEntry>, Option<String>)>, GhError> {
        let r = self.send("GET", &self.url(path), None, etag).await?;
        match r.status {
            304 => Ok(Fetched::NotModified),
            404 => Ok(Fetched::Missing),
            200 => {
                let entries =
                    serde_json::from_str(&r.body).map_err(|e| GhError::BadData(e.to_string()))?;
                Ok(Fetched::Found((entries, r.etag)))
            }
            _ => Err(Self::error_for(&r)),
        }
    }

    /// Creates or updates a file. `sha` must be the current blob sha when the
    /// file exists; a stale sha gives `Conflict`. Returns the new sha.
    pub async fn put_file(
        &self,
        path: &str,
        text: &str,
        sha: Option<&str>,
        message: &str,
    ) -> Result<String, GhError> {
        let mut body = serde_json::json!({ "message": message, "content": B64.encode(text) });
        if let Some(s) = sha {
            body["sha"] = s.into();
        }
        let r = self
            .send("PUT", &self.url(path), Some(&body.to_string()), None)
            .await?;
        if r.status != 200 && r.status != 201 {
            return Err(Self::error_for(&r));
        }
        let p: PutResp =
            serde_json::from_str(&r.body).map_err(|e| GhError::BadData(e.to_string()))?;
        Ok(p.content.sha)
    }

    /// Appends events to their day file: read, append, write with the sha we
    /// read. On a conflict, re-read and try again (up to `MAX_RETRIES` times,
    /// with backoff). Existing events are never rewritten, and events already
    /// in the file are skipped. Returns the file as it now is in the repo.
    pub async fn append_events(
        &self,
        day: Day,
        author: &str,
        events: &[Event],
    ) -> Result<RepoFile, GhError> {
        let path = day.events_path();
        let mut retry = 0;
        loop {
            let (existing, sha) = match self.get_file(&path, None).await? {
                Fetched::Found(f) => (Some(f.text), Some(f.sha)),
                Fetched::Missing | Fetched::NotModified => (None, None),
            };
            let out = append_to_day_file(existing.as_deref(), events)
                .map_err(|e| GhError::BadData(format!("{path} is not a JSON array ({e})")))?;
            if out.added == 0 {
                return Ok(RepoFile {
                    text: existing.unwrap_or_default(),
                    sha: sha.unwrap_or_default(),
                    etag: None,
                });
            }
            let message = commit_message(author, day, out.added);
            match self
                .put_file(&path, &out.text, sha.as_deref(), &message)
                .await
            {
                Ok(new_sha) => {
                    return Ok(RepoFile {
                        text: out.text,
                        sha: new_sha,
                        etag: None,
                    });
                }
                Err(GhError::Conflict) if retry < MAX_RETRIES => {
                    retry += 1;
                    sleep_ms(backoff_ms(retry, js_sys::Math::random())).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

pub async fn sleep_ms(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
    });
    let _ = JsFuture::from(promise).await;
}
