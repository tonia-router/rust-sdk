use tonia_sdk::{
    realtime_connect_url, realtime_ws_url, ErrorKind, RealtimeConnectParams, FIRST_HOP_MODEL,
    FIRST_HOP_PROVIDER, REALTIME_PATH,
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
