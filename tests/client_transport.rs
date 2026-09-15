use std::collections::HashMap;
use std::time::Duration;

use serde_json::json;
use tonia_sdk::transport::{build_headers, join_url, AuthStyle};
use tonia_sdk::{
    ErrorKind, RequestOptions, Tonia, DEFAULT_BASE_URL, DEFAULT_TIMEOUT, IMAGE_TIMEOUT,
    SDK_USER_AGENT,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    // `TONIA_BASE_URL` is intentionally ignored even if present in the process.
    let client = Tonia::builder()
        .base_url("https://pass.tonia.ca:8443")
        .build();
    assert_eq!(client.base_url(), DEFAULT_BASE_URL);
    assert_eq!(Tonia::new().base_url(), DEFAULT_BASE_URL);
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
