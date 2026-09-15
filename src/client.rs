//! Official Rust client for tonia Pass (async).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::errors::{
    error_from_http_fallback, raise_from_response_body, raise_from_stream_headers, ToniaError,
};
use crate::escape::assert_path_allowed;
use crate::limits::{limits_from_headers, LimitInfo};
use crate::stream::{feed_sse, raise_if_stream_carrier, SseEvent};
use crate::transport::{
    build_headers, encode_path_segment, headers_from_reqwest, join_url, AuthStyle,
    DEFAULT_BASE_URL, DEFAULT_TIMEOUT, IMAGE_TIMEOUT, SDK_USER_AGENT,
};

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub auth: AuthStyle,
    pub headers: HashMap<String, String>,
    pub timeout: Option<Duration>,
}

struct Inner {
    http: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    default_headers: HashMap<String, String>,
    timeout: Option<Duration>,
    realtime_url: Option<String>,
    last_limits: Mutex<Option<LimitInfo>>,
}

#[derive(Clone)]
pub struct Tonia {
    inner: Arc<Inner>,
    pub catalogue: Catalogue,
    pub public_models: PublicModels,
    pub public_model_categories: PublicModelCategories,
    pub status: Status,
    pub models: Models,
    pub chat: Chat,
    pub messages: Messages,
    pub embeddings: Embeddings,
    pub images: Images,
    pub audio: Audio,
    pub responses: Responses,
    pub rerank: Rerank,
    pub interactions: Interactions,
}

#[derive(Debug, Default)]
pub struct ToniaBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    default_headers: HashMap<String, String>,
    timeout: Option<Duration>,
    realtime_url: Option<String>,
}

impl ToniaBuilder {
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn default_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.default_headers = headers;
        self
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.default_headers.insert(key.into(), value.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn realtime_url(mut self, realtime_url: impl Into<String>) -> Self {
        self.realtime_url = Some(realtime_url.into());
        self
    }

    pub fn build(self) -> Tonia {
        let base_url = self
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let api_key = resolve_api_key(self.api_key);
        let realtime_url = resolve_realtime_url(self.realtime_url);
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(self.timeout.unwrap_or(DEFAULT_TIMEOUT))
            .user_agent(SDK_USER_AGENT)
            .build()
            .expect("reqwest rustls client");
        let inner = Arc::new(Inner {
            http,
            api_key,
            base_url,
            default_headers: self.default_headers,
            timeout: self.timeout,
            realtime_url,
            last_limits: Mutex::new(None),
        });
        Tonia::from_inner(inner)
    }
}

impl Tonia {
    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn builder() -> ToniaBuilder {
        ToniaBuilder::default()
    }

    fn from_inner(inner: Arc<Inner>) -> Self {
        Self {
            catalogue: Catalogue {
                inner: Arc::clone(&inner),
            },
            public_models: PublicModels {
                inner: Arc::clone(&inner),
            },
            public_model_categories: PublicModelCategories {
                inner: Arc::clone(&inner),
            },
            status: Status {
                inner: Arc::clone(&inner),
            },
            models: Models {
                inner: Arc::clone(&inner),
            },
            chat: Chat {
                completions: ChatCompletions {
                    inner: Arc::clone(&inner),
                },
            },
            messages: Messages {
                inner: Arc::clone(&inner),
            },
            embeddings: Embeddings {
                inner: Arc::clone(&inner),
            },
            images: Images {
                inner: Arc::clone(&inner),
            },
            audio: Audio {
                speech: AudioSpeech {
                    inner: Arc::clone(&inner),
                },
                transcriptions: AudioTranscriptions {
                    inner: Arc::clone(&inner),
                },
            },
            responses: Responses {
                inner: Arc::clone(&inner),
            },
            rerank: Rerank {
                inner: Arc::clone(&inner),
            },
            interactions: Interactions {
                inner: Arc::clone(&inner),
            },
            inner,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.inner.timeout
    }

    pub fn realtime_url(&self) -> Option<&str> {
        self.inner.realtime_url.as_deref()
    }

    pub fn last_limits(&self) -> Option<LimitInfo> {
        self.inner.last_limits.lock().expect("last_limits").clone()
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        opts: RequestOptions,
    ) -> Result<Value, ToniaError> {
        let path = assert_path_allowed(path)?;
        send_json(
            &self.inner,
            method,
            &path,
            body,
            opts.auth,
            Some(opts.headers),
            opts.timeout,
        )
        .await
    }

    pub async fn stream(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        opts: RequestOptions,
    ) -> Result<Vec<SseEvent>, ToniaError> {
        let path = assert_path_allowed(path)?;
        stream_sse(
            &self.inner,
            method,
            &path,
            body,
            opts.auth,
            Some(opts.headers),
            opts.timeout,
        )
        .await
    }
}

impl Default for Tonia {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            auth: AuthStyle::Bearer,
            headers: HashMap::new(),
            timeout: None,
        }
    }
}

fn resolve_api_key(explicit: Option<String>) -> Option<String> {
    explicit.filter(|value| !value.is_empty()).or_else(|| {
        std::env::var("TONIA_API_KEY")
            .ok()
            .filter(|value| !value.is_empty())
    })
}

fn resolve_realtime_url(explicit: Option<String>) -> Option<String> {
    explicit
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("TONIA_REALTIME_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .map(|value| value.trim().to_string())
}

fn image_timeout(inner: &Inner) -> Duration {
    inner.timeout.unwrap_or(IMAGE_TIMEOUT)
}

fn wants_stream(body: &Value) -> bool {
    body.get("stream").and_then(Value::as_bool) == Some(true)
}

async fn send_json(
    inner: &Inner,
    method: &str,
    path: &str,
    body: Option<Value>,
    auth: AuthStyle,
    headers: Option<HashMap<String, String>>,
    timeout: Option<Duration>,
) -> Result<Value, ToniaError> {
    let path = assert_path_allowed(path)?;
    let hdrs = build_headers(
        inner.api_key.as_deref(),
        &inner.default_headers,
        auth,
        headers.as_ref(),
        "application/json",
    )?;
    let url = join_url(&inner.base_url, &path);
    let mut req = inner.http.request(
        method
            .parse::<reqwest::Method>()
            .unwrap_or(reqwest::Method::GET),
        &url,
    );
    for (key, value) in hdrs {
        req = req.header(key, value);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    if let Some(timeout) = timeout {
        req = req.timeout(timeout);
    }
    let res = req.send().await.map_err(ToniaError::transport)?;
    let status = res.status().as_u16();
    let header_pairs = headers_from_reqwest(res.headers());
    let bytes = res.bytes().await.map_err(ToniaError::transport)?;
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    raise_from_response_body(&parsed, status, &header_pairs)?;
    if !(200..300).contains(&status) {
        return Err(error_from_http_fallback(status, Some(parsed), header_pairs));
    }
    *inner.last_limits.lock().expect("last_limits") = limits_from_headers(&header_pairs);
    Ok(parsed)
}

async fn stream_sse(
    inner: &Inner,
    method: &str,
    path: &str,
    body: Option<Value>,
    auth: AuthStyle,
    headers: Option<HashMap<String, String>>,
    timeout: Option<Duration>,
) -> Result<Vec<SseEvent>, ToniaError> {
    let path = assert_path_allowed(path)?;
    let payload = match body {
        Some(Value::Object(mut map)) => {
            map.insert("stream".to_string(), json!(true));
            Value::Object(map)
        }
        None => json!({ "stream": true }),
        Some(other) => other,
    };
    let hdrs = build_headers(
        inner.api_key.as_deref(),
        &inner.default_headers,
        auth,
        headers.as_ref(),
        "text/event-stream",
    )?;
    let url = join_url(&inner.base_url, &path);
    let mut req = inner.http.request(
        method
            .parse::<reqwest::Method>()
            .unwrap_or(reqwest::Method::POST),
        &url,
    );
    for (key, value) in hdrs {
        req = req.header(key, value);
    }
    if payload.is_object() {
        req = req.json(&payload);
    }
    if let Some(timeout) = timeout {
        req = req.timeout(timeout);
    }
    let res = req.send().await.map_err(ToniaError::transport)?;
    let status = res.status().as_u16();
    let header_pairs = headers_from_reqwest(res.headers());
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    *inner.last_limits.lock().expect("last_limits") = limits_from_headers(&header_pairs);
    if !(200..300).contains(&status) || !content_type.contains("text/event-stream") {
        let bytes = res.bytes().await.map_err(ToniaError::transport)?;
        let parsed = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        raise_from_response_body(&parsed, status, &header_pairs)?;
        if !(200..300).contains(&status) {
            return Err(error_from_http_fallback(status, Some(parsed), header_pairs));
        }
        return Ok(Vec::new());
    }
    raise_from_stream_headers(&header_pairs)?;
    let mut buffer = String::new();
    let mut events = Vec::new();
    let mut byte_stream = res.bytes_stream();
    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk.map_err(ToniaError::transport)?;
        let text = String::from_utf8_lossy(&chunk);
        let (parsed_events, rest) = feed_sse(&buffer, &text);
        buffer = rest;
        for event in parsed_events {
            if let Some(json) = &event.json {
                raise_if_stream_carrier(json)?;
            }
            events.push(event);
        }
    }
    if !buffer.trim().is_empty() {
        let (parsed_events, _) = feed_sse(&buffer, "\n\n");
        for event in parsed_events {
            if let Some(json) = &event.json {
                raise_if_stream_carrier(json)?;
            }
            events.push(event);
        }
    }
    Ok(events)
}

macro_rules! resource {
    ($name:ident) => {
        #[derive(Clone)]
        pub struct $name {
            inner: Arc<Inner>,
        }
    };
}

resource!(Catalogue);
resource!(PublicModels);
resource!(PublicModelCategories);
resource!(Status);
resource!(Models);
resource!(ChatCompletions);
resource!(Messages);
resource!(Embeddings);
resource!(Images);
resource!(AudioSpeech);
resource!(AudioTranscriptions);
resource!(Responses);
resource!(Rerank);
resource!(Interactions);

#[derive(Clone)]
pub struct Chat {
    pub completions: ChatCompletions,
}

#[derive(Clone)]
pub struct Audio {
    pub speech: AudioSpeech,
    pub transcriptions: AudioTranscriptions,
}

impl Catalogue {
    pub async fn list(&self) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            "/v1/public/catalogue",
            None,
            AuthStyle::None,
            None,
            None,
        )
        .await
    }
}

impl PublicModels {
    pub async fn list(&self) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            "/v1/public/models",
            None,
            AuthStyle::None,
            None,
            None,
        )
        .await
    }

    pub async fn get(&self, model_id: &str) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            &format!("/v1/public/models/{}", encode_path_segment(model_id)),
            None,
            AuthStyle::None,
            None,
            None,
        )
        .await
    }
}

impl PublicModelCategories {
    pub async fn list(&self) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            "/v1/public/model-categories",
            None,
            AuthStyle::None,
            None,
            None,
        )
        .await
    }
}

impl Status {
    pub async fn get(&self) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            "/v1/status",
            None,
            AuthStyle::None,
            None,
            None,
        )
        .await
    }
}

impl Models {
    pub async fn list(&self) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            "/v1/models",
            None,
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }

    pub async fn get(&self, model_id: &str) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "GET",
            &format!("/v1/models/{}", encode_path_segment(model_id)),
            None,
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }
}

impl ChatCompletions {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        if wants_stream(&body) {
            let _ = self.stream(body).await?;
            return Ok(Value::Null);
        }
        send_json(
            &self.inner,
            "POST",
            "/v1/chat/completions",
            Some(body),
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }

    pub async fn stream(&self, body: Value) -> Result<Vec<SseEvent>, ToniaError> {
        stream_sse(
            &self.inner,
            "POST",
            "/v1/chat/completions",
            Some(body),
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }
}

impl Messages {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        if wants_stream(&body) {
            let _ = self.stream(body).await?;
            return Ok(Value::Null);
        }
        send_json(
            &self.inner,
            "POST",
            "/v1/messages",
            Some(body),
            AuthStyle::ApiKey,
            None,
            None,
        )
        .await
    }

    pub async fn stream(&self, body: Value) -> Result<Vec<SseEvent>, ToniaError> {
        stream_sse(
            &self.inner,
            "POST",
            "/v1/messages",
            Some(body),
            AuthStyle::ApiKey,
            None,
            None,
        )
        .await
    }
}

impl Embeddings {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "POST",
            "/v1/embeddings",
            Some(body),
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }
}

impl Images {
    pub async fn generate(&self, body: Value) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "POST",
            "/v1/images/generations",
            Some(body),
            AuthStyle::Bearer,
            None,
            Some(image_timeout(&self.inner)),
        )
        .await
    }

    pub async fn edit(&self, body: Value) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "POST",
            "/v1/images/edits",
            Some(body),
            AuthStyle::Bearer,
            None,
            Some(image_timeout(&self.inner)),
        )
        .await
    }
}

impl AudioSpeech {
    pub async fn create(&self, body: Value) -> Result<bytes::Bytes, ToniaError> {
        let mut headers = HashMap::new();
        headers.insert(
            "Accept".to_string(),
            "application/octet-stream, audio/*, application/json".to_string(),
        );
        let value = send_json(
            &self.inner,
            "POST",
            "/v1/audio/speech",
            Some(body),
            AuthStyle::Bearer,
            Some(headers),
            Some(image_timeout(&self.inner)),
        )
        .await?;
        match value {
            Value::String(text) => Ok(bytes::Bytes::from(text)),
            other => Ok(bytes::Bytes::from(other.to_string())),
        }
    }
}

impl AudioTranscriptions {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "POST",
            "/v1/audio/transcriptions",
            Some(body),
            AuthStyle::Bearer,
            None,
            Some(image_timeout(&self.inner)),
        )
        .await
    }
}

impl Responses {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        if wants_stream(&body) {
            let _ = self.stream(body).await?;
            return Ok(Value::Null);
        }
        send_json(
            &self.inner,
            "POST",
            "/v1/responses",
            Some(body),
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }

    pub async fn stream(&self, body: Value) -> Result<Vec<SseEvent>, ToniaError> {
        stream_sse(
            &self.inner,
            "POST",
            "/v1/responses",
            Some(body),
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }
}

impl Rerank {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        send_json(
            &self.inner,
            "POST",
            "/v1/rerank",
            Some(body),
            AuthStyle::Bearer,
            None,
            None,
        )
        .await
    }
}

impl Interactions {
    pub async fn create(&self, body: Value) -> Result<Value, ToniaError> {
        if wants_stream(&body) {
            let _ = self.stream(body).await?;
            return Ok(Value::Null);
        }
        send_json(
            &self.inner,
            "POST",
            "/v1/interactions",
            Some(body),
            AuthStyle::Bearer,
            None,
            Some(image_timeout(&self.inner)),
        )
        .await
    }

    pub async fn stream(&self, body: Value) -> Result<Vec<SseEvent>, ToniaError> {
        stream_sse(
            &self.inner,
            "POST",
            "/v1/interactions",
            Some(body),
            AuthStyle::Bearer,
            None,
            Some(image_timeout(&self.inner)),
        )
        .await
    }
}
