use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::json;
use tonia_sdk::transport::{build_headers, join_url, AuthStyle};
use tonia_sdk::{
    audio_mime_for_name, encode_path_segment, ErrorKind, InvalidTranscriptionFile, RequestOptions,
    Tonia, TranscriptionError, TranscriptionFile, DEFAULT_BASE_URL, DEFAULT_TIMEOUT, IMAGE_TIMEOUT,
    SDK_USER_AGENT,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn default_base_url_is_tonia_pass_only() {
    let client = Tonia::new();
    assert_eq!(client.base_url(), DEFAULT_BASE_URL);
    assert_eq!(client.timeout(), None);
}

#[test]
fn ctor_timeout_is_stored() {
    let client = Tonia::builder()
        .timeout(Duration::from_millis(1250))
        .build();
    assert_eq!(client.timeout(), Some(Duration::from_millis(1250)));
}

#[test]
fn ctor_does_not_read_tonia_base_url() {
    let _guard = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("TONIA_BASE_URL").ok();
    std::env::set_var("TONIA_BASE_URL", "https://evil.example");
    let client = Tonia::new();
    assert_eq!(client.base_url(), DEFAULT_BASE_URL);
    restore_env("TONIA_BASE_URL", prev);
}

#[test]
fn build_headers_sets_ua_bearer_and_missing_key() {
    let err = build_headers(
        None,
        &HashMap::new(),
        AuthStyle::Bearer,
        None,
        "application/json",
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Authentication);
    assert_eq!(err.code.as_deref(), Some("missing_bearer"));

    let hdrs = build_headers(
        Some("tonia_test"),
        &HashMap::new(),
        AuthStyle::Bearer,
        None,
        "application/json",
    )
    .unwrap();
    assert_eq!(
        hdrs.get("Authorization").map(String::as_str),
        Some("Bearer tonia_test")
    );
    assert_eq!(
        hdrs.get("User-Agent").map(String::as_str),
        Some(SDK_USER_AGENT)
    );
    assert!(!hdrs.keys().any(|key| key.eq_ignore_ascii_case("x-api-key")));

    let hdrs = build_headers(
        Some("tonia_test"),
        &HashMap::new(),
        AuthStyle::ApiKey,
        None,
        "application/json",
    )
    .unwrap();
    assert_eq!(
        hdrs.get("x-api-key").map(String::as_str),
        Some("tonia_test")
    );
    assert!(!hdrs
        .keys()
        .any(|key| key.eq_ignore_ascii_case("authorization")));

    let none = build_headers(
        None,
        &HashMap::new(),
        AuthStyle::None,
        None,
        "application/json",
    )
    .unwrap();
    assert!(!none
        .keys()
        .any(|key| key.eq_ignore_ascii_case("authorization")));
    assert!(!none.keys().any(|key| key.eq_ignore_ascii_case("x-api-key")));
}

#[test]
fn join_url_prefixes_slash() {
    assert_eq!(
        join_url("https://pass.example", "/v1/models"),
        "https://pass.example/v1/models"
    );
    assert_eq!(
        join_url("https://pass.example", "v1/models"),
        "https://pass.example/v1/models"
    );
}

#[test]
fn image_timeout_const_is_300s() {
    assert_eq!(IMAGE_TIMEOUT, Duration::from_secs(300));
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(60));
}

fn client(server: &MockServer, api_key: Option<&str>) -> Tonia {
    let mut builder = Tonia::builder().base_url(server.uri());
    if let Some(api_key) = api_key {
        builder = builder.api_key(api_key);
    }
    builder.build()
}

#[tokio::test]
async fn public_route_sends_no_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/public/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": "gpt-test" }]
        })))
        .mount(&server)
        .await;

    let body = client(&server, None).public_models.list().await.unwrap();
    assert_eq!(body["data"][0]["id"], "gpt-test");
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0]
        .headers
        .iter()
        .any(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization")));
}

#[tokio::test]
async fn bearer_default_headers_and_limits_are_preserved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer tonia_test"))
        .and(header("x-tonia-title", "SDK E2E"))
        .and(header("user-agent", SDK_USER_AGENT))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-tonia-limit-warning", "usage_limit_80_percent")
                .insert_header("x-tonia-limit-remaining", "17")
                .set_body_json(json!({ "data": [{ "id": "gpt-test" }] })),
        )
        .mount(&server)
        .await;

    let tonia = Tonia::builder()
        .base_url(server.uri())
        .api_key("tonia_test")
        .header("X-Tonia-Title", "SDK E2E")
        .build();
    tonia.models.list().await.unwrap();
    let limits = tonia.last_limits().expect("limits");
    assert_eq!(limits.warning.as_deref(), Some("usage_limit_80_percent"));
    assert_eq!(limits.remaining.as_deref(), Some("17"));
}

#[tokio::test]
async fn messages_uses_x_api_key_not_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "tonia_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "content": [] })))
        .mount(&server)
        .await;

    client(&server, Some("tonia_test"))
        .messages
        .create(json!({ "model": "claude-test", "messages": [], "max_tokens": 1 }))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0]
        .headers
        .iter()
        .any(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization")));
}

#[tokio::test]
async fn policy_carrier_on_200_does_not_set_last_limits() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-tonia-limit-warning", "usage_limit_80_percent")
                .set_body_json(json!({
                    "_tonia_policy_block": { "code": "regulated_content_detected" }
                })),
        )
        .mount(&server)
        .await;

    let tonia = client(&server, Some("tonia_test"));
    let err = tonia
        .chat
        .completions
        .create(json!({ "model": "gpt-test", "messages": [] }))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PolicyBlock);
    assert!(tonia.last_limits().is_none());
}

#[tokio::test]
async fn no_retry_on_503() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": { "type": "api_error", "code": "upstream", "retryable": true }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server, Some("tonia_test"))
        .models
        .list()
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Api);
}

#[tokio::test]
async fn request_rejects_realtime_http() {
    let tonia = Tonia::builder()
        .base_url("https://pass.example")
        .api_key("tonia_test")
        .build();
    let err = tonia
        .request("GET", "/v1/realtime", None, RequestOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PathNotAllowed);
}

#[tokio::test]
async fn request_strips_query_and_allows_audio_translations() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/translations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "ok" })))
        .mount(&server)
        .await;

    let body = client(&server, Some("tonia_test"))
        .request(
            "POST",
            "/v1/audio/translations?lang=fr",
            Some(json!({ "model": "whisper" })),
            RequestOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(body["text"], "ok");
}

#[tokio::test]
async fn empty_models_list_is_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&server)
        .await;

    let body = client(&server, Some("tonia_test"))
        .models
        .list()
        .await
        .unwrap();
    assert_eq!(body["data"], json!([]));
}

#[tokio::test]
async fn models_get_encodes_slash_and_space() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/vendor%2Fmodel%20id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/public/models/vendor%2Fmodel%20id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let tonia = client(&server, Some("tonia_test"));
    tonia.models.get("vendor/model id").await.unwrap();
    tonia.public_models.get("vendor/model id").await.unwrap();
    assert_eq!(
        encode_path_segment("vendor/model id"),
        "vendor%2Fmodel%20id"
    );
    assert_eq!(encode_path_segment("gpt-4"), "gpt-4");
}

#[tokio::test]
async fn prompt_cache_fields_pass_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usage": { "prompt_tokens_details": { "cached_tokens": 8 } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usage": { "prompt_tokens_details": { "cached_tokens": 8 } }
        })))
        .mount(&server)
        .await;

    let tonia = client(&server, Some("tonia_test"));
    let chat = tonia
        .chat
        .completions
        .create(json!({
            "model": "gpt-5.4-nano",
            "messages": [{ "role": "user", "content": "ok" }],
            "prompt_cache_key": "caller-key"
        }))
        .await
        .unwrap();
    let messages = tonia
        .messages
        .create(json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 8,
            "cache_control": { "type": "ephemeral" },
            "messages": [{ "role": "user", "content": "ok" }]
        }))
        .await
        .unwrap();
    assert_eq!(chat["usage"]["prompt_tokens_details"]["cached_tokens"], 8);
    assert_eq!(
        messages["usage"]["prompt_tokens_details"]["cached_tokens"],
        8
    );
    let requests = server.received_requests().await.unwrap();
    let chat_body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let msg_body: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(chat_body["prompt_cache_key"], "caller-key");
    assert_eq!(msg_body["cache_control"], json!({ "type": "ephemeral" }));
}

#[tokio::test]
async fn bare_429_is_rate_limit_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "2"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server, Some("tonia_test"))
        .responses
        .create(json!({ "model": "gpt-test", "input": "hello" }))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::RateLimit);
    assert_eq!(err.retryable, Some(true));
    assert_eq!(err.retry_after_seconds(), Some(2));
}

#[tokio::test]
async fn named_helpers_hit_locked_paths() {
    let server = MockServer::start().await;
    for route in [
        "/v1/public/catalogue",
        "/v1/public/models/vendor%2Fmodel",
        "/v1/public/model-categories",
        "/v1/status",
        "/v1/embeddings",
        "/v1/images/generations",
        "/v1/images/edits",
        "/v1/audio/speech",
        "/v1/audio/transcriptions",
        "/v1/responses",
        "/v1/rerank",
        "/v1/interactions",
    ] {
        Mock::given(path(route))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({ "data": [{ "id": "ok" }] })),
            )
            .mount(&server)
            .await;
    }

    let tonia = client(&server, Some("tonia_test"));
    tonia.catalogue.list().await.unwrap();
    tonia.public_models.get("vendor/model").await.unwrap();
    tonia.public_model_categories.list().await.unwrap();
    tonia.status.get().await.unwrap();
    tonia
        .embeddings
        .create(json!({ "model": "embed", "input": "hello" }))
        .await
        .unwrap();
    tonia
        .images
        .generate(json!({ "model": "image", "prompt": "hello" }))
        .await
        .unwrap();
    tonia
        .images
        .edit(json!({ "model": "image", "prompt": "hello" }))
        .await
        .unwrap();
    let _ = tonia
        .audio
        .speech
        .create(json!({ "model": "gpt-4o-mini-tts", "input": "hello", "voice": "alloy" }))
        .await
        .unwrap();
    tonia
        .audio
        .transcriptions
        .create("gpt-transcribe", b"RIFF".as_slice(), Some("clip.wav"), &[])
        .await
        .unwrap();
    tonia
        .responses
        .create(json!({ "model": "gpt", "input": "hello" }))
        .await
        .unwrap();
    tonia
        .rerank
        .create(json!({ "model": "rerank", "query": "a", "documents": ["b"] }))
        .await
        .unwrap();
    tonia
        .interactions
        .create(json!({ "model": "gpt", "input": "hello" }))
        .await
        .unwrap();
}

#[tokio::test]
async fn systemone_stays_an_escape_hatch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "answers": {} })))
        .mount(&server)
        .await;

    let body = client(&server, Some("tonia_test"))
        .request(
            "POST",
            "/v1/systemone",
            Some(json!({
                "model": "typesafe/jev-latest",
                "state": "The sky is blue.",
                "questions": { "color": { "type": "noul", "instructions": "Is the sky blue?" } }
            })),
            RequestOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(body["answers"], json!({}));
}

#[tokio::test]
async fn gemini_interactions_body_is_passthrough() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gemini/gemini-2.5-flash-image",
        "stream": false,
        "input": [
            { "type": "text", "text": "Make the fox sit" },
            { "type": "image", "mime_type": "image/png", "data": "AA==" }
        ]
    });
    Mock::given(method("POST"))
        .and(path("/v1/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "interaction": {} })))
        .mount(&server)
        .await;

    client(&server, Some("tonia_test"))
        .interactions
        .create(body.clone())
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent, body);
}

#[tokio::test]
async fn provider_requires_surface_stays_invalid_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "code": "provider_requires_surface",
                "provider": "gemini",
                "required_surface": "interactions",
                "retryable": false
            }
        })))
        .mount(&server)
        .await;

    let err = client(&server, Some("tonia_test"))
        .images
        .generate(json!({ "model": "gemini/gemini-2.5-flash-image", "prompt": "fox" }))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code.as_deref(), Some("provider_requires_surface"));
    assert_eq!(err.status, Some(400));
    assert_eq!(err.retryable, Some(false));
}

#[tokio::test]
async fn audio_speech_returns_raw_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b"ID3\x00fake-mp3".as_slice(), "audio/mpeg"),
        )
        .mount(&server)
        .await;

    let audio = client(&server, Some("tonia_test"))
        .audio
        .speech
        .create(json!({ "model": "gpt-4o-mini-tts", "input": "Bonjour", "voice": "alloy" }))
        .await
        .unwrap();
    assert_eq!(&audio[..], b"ID3\x00fake-mp3");
    let accept = server.received_requests().await.unwrap()[0]
        .headers
        .iter()
        .find(|(k, _)| k.as_str().eq_ignore_ascii_case("accept"))
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap();
    assert!(accept.starts_with("application/octet-stream"));
}

#[tokio::test]
async fn audio_transcriptions_sends_multipart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "bonjour" })))
        .mount(&server)
        .await;

    let out = client(&server, Some("tonia_test"))
        .audio
        .transcriptions
        .create(
            "gpt-transcribe",
            b"RIFF".as_slice(),
            Some("clip.wav"),
            &[("language", "fr")],
        )
        .await
        .unwrap();
    assert_eq!(out["text"], "bonjour");
    let request = &server.received_requests().await.unwrap()[0];
    let content_type = request
        .headers
        .iter()
        .find(|(k, _)| k.as_str().eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap();
    assert!(content_type.contains("multipart/form-data"));
    assert!(!content_type.contains("application/json"));
    let body = &request.body;
    assert!(body.windows(b"clip.wav".len()).any(|w| w == b"clip.wav"));
    assert!(body
        .windows(b"gpt-transcribe".len())
        .any(|w| w == b"gpt-transcribe"));
    assert!(body.windows(b"language".len()).any(|w| w == b"language"));
    assert!(!body.starts_with(b"{"));
}

#[test]
fn audio_transcriptions_rejects_data_uri_before_network() {
    let err = TranscriptionFile::try_from_user("data:audio/wav;base64,AA==").unwrap_err();
    assert!(err.to_string().contains("data URI"));
    assert_eq!(err, InvalidTranscriptionFile::data_uri());
    assert_eq!(
        TranscriptionFile::try_from("data:audio/wav;base64,AA==").unwrap_err(),
        InvalidTranscriptionFile::data_uri()
    );
    assert_eq!(audio_mime_for_name("clip.wav"), "audio/wav");
    assert_eq!(audio_mime_for_name("clip.mpga"), "application/octet-stream");
    assert_eq!(audio_mime_for_name("clip.oga"), "application/octet-stream");
}

#[tokio::test]
async fn audio_transcriptions_create_does_not_wrap_data_uri_as_tonia_error() {
    let err = Tonia::builder()
        .api_key("tonia_test")
        .build()
        .audio
        .transcriptions
        .create(
            "gpt-transcribe",
            TranscriptionFile::Path(std::path::PathBuf::from("data:audio/wav;base64,AA==")),
            None,
            &[],
        )
        .await
        .unwrap_err();
    match err {
        TranscriptionError::InvalidFile(inner) => {
            assert_eq!(inner, InvalidTranscriptionFile::data_uri());
        }
        TranscriptionError::Api(other) => panic!("wrapped as ToniaError: {other:?}"),
    }
}

#[tokio::test]
async fn ctor_timeout_overrides_heavy_helpers() {
    let client = Tonia::builder()
        .timeout(Duration::from_millis(40))
        .api_key("tonia_test")
        .build();
    assert_eq!(client.heavy_timeout(), Duration::from_millis(40));
    assert_eq!(
        Tonia::builder()
            .api_key("tonia_test")
            .build()
            .heavy_timeout(),
        IMAGE_TIMEOUT
    );

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_json(json!({ "data": [] })),
        )
        .mount(&server)
        .await;
    let err = Tonia::builder()
        .base_url(server.uri())
        .api_key("tonia_test")
        .timeout(Duration::from_millis(40))
        .build()
        .images
        .generate(json!({ "model": "image", "prompt": "hello" }))
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("timeout"));
}

#[tokio::test]
async fn interactions_stream_uses_ctor_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/interactions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_raw("data: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;
    let err = Tonia::builder()
        .base_url(server.uri())
        .api_key("tonia_test")
        .timeout(Duration::from_millis(40))
        .build()
        .interactions
        .stream(json!({ "model": "gpt", "input": "hello" }))
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("timeout"));
}

#[tokio::test]
async fn create_stream_true_does_not_drain() {
    let tonia = Tonia::builder().api_key("tonia_test").build();
    let err = tonia
        .chat
        .completions
        .create(json!({ "model": "gpt", "stream": true, "messages": [] }))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code.as_deref(), Some("use_stream_helper"));
}

#[tokio::test]
async fn stream_is_incremental_and_yields_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-tonia-limit-remaining", "17")
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"o\"}}]}\n\n\
                     data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ),
        )
        .mount(&server)
        .await;

    let tonia = client(&server, Some("tonia_test"));
    let mut stream = tonia
        .chat
        .completions
        .stream(json!({ "model": "gpt-test", "messages": [] }));
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.unwrap());
    }
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].data, "[DONE]");
    assert_eq!(
        tonia.last_limits().unwrap().remaining.as_deref(),
        Some("17")
    );
    let request = &server.received_requests().await.unwrap()[0];
    assert!(request
        .headers
        .iter()
        .any(|(k, v)| k.as_str().eq_ignore_ascii_case("accept")
            && v.to_str().unwrap() == "text/event-stream"));
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains("\"stream\":true"));
}

#[tokio::test]
async fn stream_policy_header_yields_zero_events() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-tonia-policy-block", "regulated_content_detected")
                .insert_header("x-tonia-limit-remaining", "3")
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"blocked\"}}]}\n\n",
                    "text/event-stream",
                ),
        )
        .mount(&server)
        .await;

    let tonia = client(&server, Some("tonia_test"));
    let mut stream = tonia
        .chat
        .completions
        .stream(json!({ "model": "gpt-test", "messages": [] }));
    let err = stream.next().await.unwrap().unwrap_err();
    assert_eq!(err.kind, ErrorKind::PolicyBlock);
    assert_eq!(err.code.as_deref(), Some("regulated_content_detected"));
    assert!(stream.next().await.is_none());
    assert_eq!(tonia.last_limits().unwrap().remaining.as_deref(), Some("3"));
}

#[tokio::test]
async fn stream_agent_and_entitlement_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-tonia-agent-block", "tool_calls_disabled_by_profile")
                .insert_header("x-tonia-agent-capability", "tool_calls")
                .set_body_raw("data: {\"choices\":[]}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let tonia = client(&server, Some("tonia_test"));
    let err = tonia
        .chat
        .completions
        .stream(json!({ "model": "gpt-test", "messages": [] }))
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::AgentBlock);
    assert_eq!(err.capability(), Some("tool_calls"));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-tonia-entitlement-block", "managed_budget_exhausted")
                .set_body_raw("data: {\"choices\":[]}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;
    let err = client(&server, Some("tonia_test"))
        .chat
        .completions
        .stream(json!({ "model": "gpt-test", "messages": [] }))
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Entitlement);
    assert_eq!(err.code.as_deref(), Some("managed_budget_exhausted"));
}

#[tokio::test]
async fn stream_200_non_sse_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .mount(&server)
        .await;

    let mut stream = client(&server, Some("tonia_test"))
        .chat
        .completions
        .stream(json!({ "model": "gpt-test", "messages": [] }));
    assert!(stream.next().await.is_none());
}

#[test]
fn env_key_and_realtime_url_are_read() {
    let _guard = ENV_LOCK.lock().unwrap();
    let prev_key = std::env::var("TONIA_API_KEY").ok();
    let prev_rt = std::env::var("TONIA_REALTIME_URL").ok();
    std::env::set_var("TONIA_API_KEY", "from-env-key");
    std::env::set_var("TONIA_REALTIME_URL", "wss://live.example/custom");
    let client = Tonia::builder()
        .base_url("https://pass.tonia.ca:8443")
        .build();
    assert_eq!(client.realtime_url(), Some("wss://live.example/custom"));
    restore_env("TONIA_API_KEY", prev_key);
    restore_env("TONIA_REALTIME_URL", prev_rt);
    let _ = client;
}

#[tokio::test]
async fn env_api_key_is_sent_as_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&server)
        .await;
    let tonia = {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev_key = std::env::var("TONIA_API_KEY").ok();
        std::env::set_var("TONIA_API_KEY", "from-env-key");
        let tonia = Tonia::builder().base_url(server.uri()).build();
        restore_env("TONIA_API_KEY", prev_key);
        tonia
    };
    tonia.models.list().await.unwrap();
    let auth = server.received_requests().await.unwrap()[0]
        .headers
        .iter()
        .find(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap();
    assert_eq!(auth, "Bearer from-env-key");
}

#[tokio::test]
async fn body_field_named_timeout_is_json_not_http_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(80))
                .set_body_json(json!({ "choices": [] })),
        )
        .mount(&server)
        .await;
    let body = json!({
        "model": "gpt-test",
        "messages": [],
        "timeout": 1
    });
    client(&server, Some("tonia_test"))
        .chat
        .completions
        .create(body.clone())
        .await
        .unwrap();
    let sent: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(sent["timeout"], 1);
}

fn restore_env(key: &str, prev: Option<String>) {
    match prev {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

#[tokio::test]
async fn missing_key_is_typed_before_network() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .expect(0)
        .mount(&server)
        .await;
    let tonia = {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev_key = std::env::var("TONIA_API_KEY").ok();
        std::env::remove_var("TONIA_API_KEY");
        let tonia = Tonia::builder().base_url(server.uri()).build();
        restore_env("TONIA_API_KEY", prev_key);
        tonia
    };
    let err = tonia.models.list().await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Authentication);
    assert_eq!(err.code.as_deref(), Some("missing_bearer"));
}

#[tokio::test]
async fn request_auth_none_overrides_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .mount(&server)
        .await;
    client(&server, Some("tonia_test"))
        .request(
            "GET",
            "/v1/status",
            None,
            RequestOptions {
                auth: AuthStyle::None,
                ..RequestOptions::default()
            },
        )
        .await
        .unwrap();
    let request = &server.received_requests().await.unwrap()[0];
    assert!(!request
        .headers
        .iter()
        .any(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization")));
}

#[tokio::test]
async fn responses_nested_tools_body_is_passthrough() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gpt-test",
        "input": [{
            "role": "user",
            "content": [{
                "type": "input_image",
                "image_url": "data:image/png;base64,AA=="
            }]
        }],
        "tools": [{ "type": "web_search_preview" }],
        "tool_choice": "auto",
        "web_search_options": { "search_context_size": "low" }
    });
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "output": [] })))
        .mount(&server)
        .await;
    client(&server, Some("tonia_test"))
        .responses
        .create(body.clone())
        .await
        .unwrap();
    let sent: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(sent, body);
}

#[tokio::test]
async fn images_edit_is_json_not_multipart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/edits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&server)
        .await;
    client(&server, Some("tonia_test"))
        .images
        .edit(json!({ "model": "image", "prompt": "hello" }))
        .await
        .unwrap();
    let request = &server.received_requests().await.unwrap()[0];
    let content_type = request
        .headers
        .iter()
        .find(|(k, _)| k.as_str().eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_str().unwrap().to_string())
        .unwrap();
    assert!(content_type.contains("application/json"));
    assert!(!content_type.contains("multipart"));
    assert!(request.body.starts_with(b"{"));
}

#[tokio::test]
async fn managed_credential_unavailable_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {
                "type": "managed_credential_unavailable",
                "code": "managed_credential_unavailable",
                "message": "Unavailable",
                "retryable": true
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server, Some("tonia_test"))
        .responses
        .create(json!({ "model": "gpt-test", "input": "hello" }))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::ManagedCredentialUnavailable);
    assert_eq!(err.retryable, Some(true));
}
