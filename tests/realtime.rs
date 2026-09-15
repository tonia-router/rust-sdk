use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, accept_hdr_async};
use tonia_sdk::{
    raise_if_realtime_error, realtime_connect_url, realtime_ws_url, ErrorKind, RealtimeConnect,
    RealtimeConnectParams, RequestOptions, Tonia, FIRST_HOP_MODEL, FIRST_HOP_PROVIDER,
    REALTIME_PATH,
};

#[test]
fn hosted_pass_keeps_8443() {
    assert_eq!(
        realtime_ws_url("https://pass.tonia.ca:8443", None),
        format!("wss://pass.tonia.ca:8443{REALTIME_PATH}")
    );
}

#[test]
fn local_8444_maps_to_8448() {
    assert_eq!(
        realtime_ws_url("http://127.0.0.1:8444", None),
        format!("ws://127.0.0.1:8448{REALTIME_PATH}")
    );
    assert_eq!(
        realtime_ws_url("http://localhost:8444", None),
        format!("ws://localhost:8448{REALTIME_PATH}")
    );
}

#[test]
fn local_missing_port_maps_to_8448() {
    assert_eq!(
        realtime_ws_url("http://127.0.0.1", None),
        format!("ws://127.0.0.1:8448{REALTIME_PATH}")
    );
}

#[test]
fn local_already_8448_stays_8448() {
    assert_eq!(
        realtime_ws_url("http://127.0.0.1:8448", None),
        format!("ws://127.0.0.1:8448{REALTIME_PATH}")
    );
}

#[test]
fn other_hosts_keep_8444() {
    assert_eq!(
        realtime_ws_url("https://pass-dev.example:8444", None),
        format!("wss://pass-dev.example:8444{REALTIME_PATH}")
    );
}

#[test]
fn override_does_not_append_path() {
    assert_eq!(
        realtime_ws_url(
            "https://pass.tonia.ca:8443",
            Some("wss://live.example/custom")
        ),
        "wss://live.example/custom"
    );
}

#[test]
fn webrtc_is_forbidden() {
    let err = realtime_connect_url(RealtimeConnectParams {
        webrtc: true,
        ..RealtimeConnectParams::new("https://pass.tonia.ca:8443")
    })
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code.as_deref(), Some("realtime_webrtc_forbidden"));
}

#[test]
fn transport_rtc_is_forbidden() {
    let err = realtime_connect_url(RealtimeConnectParams {
        transport: Some("rtc"),
        ..RealtimeConnectParams::new("https://pass.tonia.ca:8443")
    })
    .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("realtime_webrtc_forbidden"));
}

#[test]
fn native_without_transcripts_is_invalid() {
    let err = realtime_connect_url(RealtimeConnectParams {
        mode: "native",
        ..RealtimeConnectParams::new("https://pass.tonia.ca:8443")
    })
    .unwrap_err();
    assert_eq!(
        err.code.as_deref(),
        Some("realtime_native_transcripts_required")
    );
}

#[test]
fn connect_url_includes_query() {
    let url = realtime_connect_url(RealtimeConnectParams {
        provider: FIRST_HOP_PROVIDER,
        model: FIRST_HOP_MODEL,
        chat_model: Some("gpt-5.4"),
        transcripts: true,
        ..RealtimeConnectParams::new("https://pass.tonia.ca:8443")
    })
    .unwrap();
    assert!(url.starts_with("wss://pass.tonia.ca:8443/v1/realtime?"));
    assert!(url.contains("provider=openai"));
    assert!(url.contains("model=gpt-live-1"));
    assert!(url.contains("mode=cascaded"));
    assert!(url.contains("chat_model=gpt-5.4"));
    assert!(url.contains("transcripts=1"));
}

#[test]
fn error_envelope_raises() {
    let err = raise_if_realtime_error(&json!({
        "error": {
            "type": "invalid_request_error",
            "code": "realtime_webrtc_forbidden",
            "retryable": false
        }
    }))
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code.as_deref(), Some("realtime_webrtc_forbidden"));
}

#[tokio::test]
async fn http_request_still_blocks_realtime() {
    let tonia = Tonia::builder().api_key("tonia_test").build();
    let err = tonia
        .request("GET", "/v1/realtime", None, RequestOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PathNotAllowed);
}

async fn spawn_scripted_ws(
    script: Vec<Message>,
    require_bearer: bool,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = if require_bearer {
            #[allow(clippy::result_large_err)]
            let callback = |req: &Request, res: Response| {
                let auth = req
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                assert_eq!(auth, "Bearer tonia_test");
                Ok(res)
            };
            accept_hdr_async(stream, callback).await.unwrap()
        } else {
            accept_async(stream).await.unwrap()
        };
        for message in script {
            ws.send(message).await.unwrap();
        }
        while let Some(Ok(_)) = ws.next().await {}
    });
    (format!("ws://{addr}/v1/realtime"), handle)
}

#[tokio::test]
async fn connect_handshake_and_wait_turn() {
    let created = r#"{"type":"session.created","session_id":"s1","mode":"cascaded","model":"gpt-live-1","reconnect":"new_session"}"#;
    let (url, handle) = spawn_scripted_ws(
        vec![
            Message::Text(created.into()),
            Message::Text(r#"{"type":"transcript.final","text":"skip"}"#.into()),
            Message::Text(r#"{"type":"output_text.done","text":"ok"}"#.into()),
        ],
        true,
    )
    .await;

    let mut session = Tonia::builder()
        .api_key("tonia_test")
        .realtime_url(&url)
        .base_url("https://pass.tonia.ca:8443")
        .build()
        .realtime
        .connect(RealtimeConnect {
            model: Some("gpt-live-1".into()),
            ..RealtimeConnect::default()
        })
        .await
        .unwrap();
    assert_eq!(session.session_id, "s1");
    assert_eq!(session.reconnect, "new_session");
    session.send_text("Reply with ok.").await.unwrap();
    let event = session.wait_turn(Duration::from_secs(2)).await.unwrap();
    assert_eq!(event["type"], "output_text.done");
    session.close().await.unwrap();
    let _ = handle.await;
}

#[tokio::test]
async fn wait_turn_returns_last_event_on_timeout() {
    let created = r#"{"type":"session.created","session_id":"s2"}"#;
    let (url, handle) = spawn_scripted_ws(
        vec![
            Message::Text(created.into()),
            Message::Text(r#"{"type":"output_audio.delta","audio":"AA=="}"#.into()),
        ],
        false,
    )
    .await;

    let mut session = Tonia::builder()
        .api_key("tonia_test")
        .realtime_url(&url)
        .timeout(Duration::from_secs(2))
        .build()
        .realtime
        .connect(RealtimeConnect::default())
        .await
        .unwrap();
    let event = session.wait_turn(Duration::from_millis(250)).await.unwrap();
    assert_eq!(event["type"], "output_audio.delta");
    session.close().await.unwrap();
    let _ = handle.await;
}

#[tokio::test]
async fn recv_timeout_with_no_event_is_probe_timeout() {
    let created = r#"{"type":"session.created","session_id":"s3"}"#;
    let (url, handle) = spawn_scripted_ws(vec![Message::Text(created.into())], false).await;
    let mut session = Tonia::builder()
        .api_key("tonia_test")
        .realtime_url(&url)
        .build()
        .realtime
        .connect(RealtimeConnect::default())
        .await
        .unwrap();
    let err = session
        .recv(Some(Duration::from_millis(80)))
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("realtime_probe_timeout"));
    session.close().await.unwrap();
    let _ = handle.await;
}

#[tokio::test]
async fn handshake_error_envelope() {
    let (url, handle) = spawn_scripted_ws(
        vec![Message::Text(
            r#"{"type":"error","error":{"type":"invalid_request_error","code":"realtime_webrtc_forbidden"}}"#
                .into(),
        )],
        false,
    )
    .await;
    let err = Tonia::builder()
        .api_key("tonia_test")
        .realtime_url(&url)
        .build()
        .realtime
        .connect(RealtimeConnect::default())
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("realtime_webrtc_forbidden"));
    let _ = handle.await;
}
