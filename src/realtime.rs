//! Mediated `/v1/realtime` WebSocket. Not an HTTP escape-hatch path.

use std::time::Duration;

use serde_json::Value;
use url::form_urlencoded::Serializer;
use url::Url;

use crate::errors::{raise_from_response_body, ToniaError};

pub const FIRST_HOP_PROVIDER: &str = "openai";
pub const FIRST_HOP_MODEL: &str = "gpt-live-1";
pub const REALTIME_PATH: &str = "/v1/realtime";
pub const LOCAL_DATA_PORT: u16 = 8444;
pub const LOCAL_REALTIME_PORT: u16 = 8448;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub const TURN_TYPES: &[&str] = &[
    "output_text.done",
    "output_audio.done",
    "transcript.final",
    "response.function_call",
];

pub const TERMINAL_TURN_TYPES: &[&str] = &[
    "output_text.done",
    "output_audio.done",
    "response.function_call",
];

pub fn realtime_ws_url(base_url: &str, realtime_url: Option<&str>) -> String {
    let override_url = realtime_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("TONIA_REALTIME_URL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    if let Some(url) = override_url {
        return url.trim_end_matches('/').to_string();
    }

    let parsed = Url::parse(base_url.trim_end_matches('/')).ok();
    let Some(parsed) = parsed else {
        return format!("ws://{}{REALTIME_PATH}", base_url.trim_end_matches('/'));
    };
    let scheme = if parsed.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    let hostname = parsed.host_str().unwrap_or("");
    let port = parsed.port();
    let netloc = if matches!(hostname, "127.0.0.1" | "localhost")
        && (port.is_none() || port == Some(LOCAL_DATA_PORT))
    {
        format!("{hostname}:{LOCAL_REALTIME_PORT}")
    } else if let Some(port) = port {
        format!("{hostname}:{port}")
    } else {
        hostname.to_string()
    };
    format!("{scheme}://{netloc}{REALTIME_PATH}")
}

#[derive(Debug, Clone)]
pub struct RealtimeConnectParams<'a> {
    pub base_url: &'a str,
    pub realtime_url: Option<&'a str>,
    pub provider: &'a str,
    pub model: &'a str,
    pub mode: &'a str,
    pub chat_model: Option<&'a str>,
    pub transcripts: bool,
    pub webrtc: bool,
    pub transport: Option<&'a str>,
}

impl<'a> RealtimeConnectParams<'a> {
    pub fn new(base_url: &'a str) -> Self {
        Self {
            base_url,
            realtime_url: None,
            provider: FIRST_HOP_PROVIDER,
            model: FIRST_HOP_MODEL,
            mode: "cascaded",
            chat_model: None,
            transcripts: false,
            webrtc: false,
            transport: None,
        }
    }
}

/// Session `connect()` options. WebRTC is rejected on the URL helper only.
#[derive(Debug, Clone, Default)]
pub struct RealtimeConnect {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub chat_model: Option<String>,
    pub transcripts: bool,
}

pub fn realtime_connect_url(params: RealtimeConnectParams<'_>) -> Result<String, ToniaError> {
    let transport = params.transport.unwrap_or("").trim().to_ascii_lowercase();
    if params.webrtc || matches!(transport.as_str(), "webrtc" | "rtc") {
        return Err(ToniaError::invalid_request(
            "Managed WebRTC to the lab is forbidden",
            "realtime_webrtc_forbidden",
        ));
    }
    let mode_norm = if params.mode.trim().is_empty() {
        "cascaded".to_string()
    } else {
        params.mode.trim().to_ascii_lowercase()
    };
    if !matches!(mode_norm.as_str(), "cascaded" | "native") {
        return Err(ToniaError::invalid_request(
            "realtime mode must be cascaded or native",
            "realtime_mode_invalid",
        ));
    }
    if mode_norm == "native" && !params.transcripts {
        return Err(ToniaError::invalid_request(
            "native Live needs transcripts=1",
            "realtime_native_transcripts_required",
        ));
    }

    let provider = {
        let trimmed = params.provider.trim();
        if trimmed.is_empty() {
            FIRST_HOP_PROVIDER
        } else {
            trimmed
        }
    };
    let model = {
        let trimmed = params.model.trim();
        if trimmed.is_empty() {
            FIRST_HOP_MODEL
        } else {
            trimmed
        }
    };

    let mut serializer = Serializer::new(String::new());
    serializer.append_pair("provider", provider);
    serializer.append_pair("model", model);
    serializer.append_pair("mode", &mode_norm);
    if let Some(chat_model) = params
        .chat_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        serializer.append_pair("chat_model", chat_model);
    }
    if params.transcripts {
        serializer.append_pair("transcripts", "1");
    }
    let query = serializer.finish();
    Ok(format!(
        "{}?{query}",
        realtime_ws_url(params.base_url, params.realtime_url)
    ))
}

pub fn raise_if_realtime_error(payload: &Value) -> Result<(), ToniaError> {
    if !payload.is_object() {
        return Ok(());
    }
    if payload.get("type").and_then(Value::as_str) == Some("error")
        || payload.get("error").is_some_and(Value::is_object)
    {
        raise_from_response_body(payload, 400, &[])?;
    }
    Ok(())
}

#[cfg(feature = "realtime")]
fn probe_timeout() -> ToniaError {
    ToniaError::invalid_request("realtime recv timeout", "realtime_probe_timeout")
}

#[cfg(feature = "realtime")]
fn handshake_invalid(created: &Value) -> ToniaError {
    let code = created
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("realtime_handshake_invalid");
    let mut err = ToniaError::invalid_request("realtime handshake failed", code);
    err = err_with_body(err, created.clone());
    err
}

#[cfg(feature = "realtime")]
fn err_with_body(err: ToniaError, body: Value) -> ToniaError {
    ToniaError::new(
        err.message.clone(),
        err.kind,
        err.r#type.clone(),
        err.code.clone(),
        err.retryable,
        err.status,
        Some(body),
        err.headers().to_vec(),
    )
}

#[cfg(feature = "realtime")]
mod live {
    use std::time::{Duration, Instant};

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

    use super::{
        err_with_body, handshake_invalid, probe_timeout, raise_if_realtime_error, RealtimeConnect,
        RealtimeConnectParams, FIRST_HOP_MODEL, FIRST_HOP_PROVIDER, HANDSHAKE_TIMEOUT,
        TERMINAL_TURN_TYPES,
    };
    use crate::errors::ToniaError;

    type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

    pub struct RealtimeSession {
        ws: WsStream,
        pub session_id: String,
        pub mode: String,
        pub model: String,
        pub reconnect: String,
        pub created: Value,
    }

    impl std::fmt::Debug for RealtimeSession {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RealtimeSession")
                .field("session_id", &self.session_id)
                .field("mode", &self.mode)
                .field("model", &self.model)
                .field("reconnect", &self.reconnect)
                .field("created", &self.created)
                .finish_non_exhaustive()
        }
    }

    impl RealtimeSession {
        fn from_created(ws: WsStream, created: Value) -> Result<Self, ToniaError> {
            raise_if_realtime_error(&created)?;
            if created.get("type").and_then(Value::as_str) != Some("session.created") {
                return Err(handshake_invalid(&created));
            }
            Ok(Self {
                session_id: created
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                mode: created
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                model: created
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or(FIRST_HOP_MODEL)
                    .to_string(),
                reconnect: created
                    .get("reconnect")
                    .and_then(Value::as_str)
                    .unwrap_or("new_session")
                    .to_string(),
                created,
                ws,
            })
        }

        async fn send_json(&mut self, payload: Value) -> Result<(), ToniaError> {
            self.ws
                .send(Message::Text(payload.to_string().into()))
                .await
                .map_err(ws_err)
        }

        pub async fn send_text(&mut self, text: impl Into<String>) -> Result<(), ToniaError> {
            self.send_json(json!({ "type": "input_text", "text": text.into() }))
                .await
        }

        pub async fn send_audio_append(
            &mut self,
            audio: &[u8],
            mime: Option<&str>,
        ) -> Result<(), ToniaError> {
            self.send_json(json!({
                "type": "input_audio.append",
                "audio": STANDARD.encode(audio),
                "mime": mime.unwrap_or("audio/wav"),
            }))
            .await
        }

        pub async fn send_audio_commit(
            &mut self,
            audio: Option<&[u8]>,
            mime: Option<&str>,
        ) -> Result<(), ToniaError> {
            let mut payload = json!({
                "type": "input_audio.commit",
                "mime": mime.unwrap_or("audio/wav"),
            });
            if let Some(audio) = audio {
                payload["audio"] = json!(STANDARD.encode(audio));
            }
            self.send_json(payload).await
        }

        pub async fn send_delegation_result(
            &mut self,
            delegation_id: impl Into<String>,
            content: impl Into<String>,
            kind: Option<&str>,
        ) -> Result<(), ToniaError> {
            self.send_json(json!({
                "type": "delegation.result",
                "delegation_id": delegation_id.into(),
                "kind": kind.unwrap_or("commentary"),
                "content": content.into(),
            }))
            .await
        }

        pub async fn recv(&mut self, timeout: Option<Duration>) -> Result<Value, ToniaError> {
            let wait = timeout.unwrap_or(Duration::from_secs(60));
            let raw = tokio::time::timeout(wait, next_text(&mut self.ws))
                .await
                .map_err(|_| probe_timeout())??;
            let payload: Value = serde_json::from_str(&raw).map_err(|_| {
                ToniaError::invalid_request(
                    "realtime event is not an object",
                    "realtime_turn_invalid",
                )
            })?;
            if !payload.is_object() {
                return Err(ToniaError::invalid_request(
                    "realtime event is not an object",
                    "realtime_turn_invalid",
                ));
            }
            raise_if_realtime_error(&payload)?;
            Ok(payload)
        }

        pub async fn wait_turn(&mut self, timeout: Duration) -> Result<Value, ToniaError> {
            let deadline = Instant::now() + timeout;
            let mut last = None;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match self.recv(Some(remaining)).await {
                    Ok(event) => {
                        let terminal = event
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| TERMINAL_TURN_TYPES.contains(&kind));
                        if terminal {
                            return Ok(event);
                        }
                        last = Some(event);
                    }
                    Err(err) if err.code.as_deref() == Some("realtime_probe_timeout") => break,
                    Err(err) => return Err(err),
                }
            }
            last.ok_or_else(probe_timeout)
        }

        pub async fn close(&mut self) -> Result<(), ToniaError> {
            self.ws.close(None).await.map_err(ws_err)
        }
    }

    async fn next_text(ws: &mut WsStream) -> Result<String, ToniaError> {
        loop {
            match ws.next().await {
                None => {
                    return Err(ToniaError::invalid_request(
                        "realtime socket closed",
                        "realtime_turn_invalid",
                    ))
                }
                Some(Err(err)) => return Err(ws_err(err)),
                Some(Ok(Message::Text(text))) => return Ok(text.to_string()),
                Some(Ok(Message::Binary(bytes))) => {
                    return String::from_utf8(bytes.to_vec()).map_err(|_| {
                        ToniaError::invalid_request(
                            "realtime event is not an object",
                            "realtime_turn_invalid",
                        )
                    })
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = ws.send(Message::Pong(payload)).await;
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                Some(Ok(Message::Close(_))) => {
                    return Err(ToniaError::invalid_request(
                        "realtime socket closed",
                        "realtime_turn_invalid",
                    ))
                }
            }
        }
    }

    fn ws_err(err: tokio_tungstenite::tungstenite::Error) -> ToniaError {
        ToniaError::new(
            err.to_string(),
            crate::errors::ErrorKind::Tonia,
            "client_error",
            Some("transport".to_string()),
            Some(true),
            None,
            None,
            Vec::new(),
        )
    }

    pub async fn connect_realtime(
        api_key: Option<&str>,
        base_url: &str,
        realtime_url: Option<&str>,
        timeout: Duration,
        opts: RealtimeConnect,
    ) -> Result<RealtimeSession, ToniaError> {
        let key = api_key
            .filter(|value| !value.is_empty())
            .ok_or_else(ToniaError::missing_bearer)?;
        let provider = opts.provider.as_deref().unwrap_or(FIRST_HOP_PROVIDER);
        let model = opts.model.as_deref().unwrap_or(FIRST_HOP_MODEL);
        let mode = opts.mode.as_deref().unwrap_or("cascaded");
        let url = super::realtime_connect_url(RealtimeConnectParams {
            base_url,
            realtime_url,
            provider,
            model,
            mode,
            chat_model: opts.chat_model.as_deref(),
            transcripts: opts.transcripts,
            webrtc: false,
            transport: None,
        })?;
        let mut request = url.into_client_request().map_err(ws_err)?;
        request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {key}")
                .parse()
                .map_err(|_| ToniaError::missing_bearer())?,
        );
        let (mut ws, _) = tokio::time::timeout(timeout, connect_async(request))
            .await
            .map_err(|_| probe_timeout())?
            .map_err(ws_err)?;
        let handshake = timeout.min(HANDSHAKE_TIMEOUT);
        let raw = tokio::time::timeout(handshake, next_text(&mut ws))
            .await
            .map_err(|_| {
                let _ = ws;
                probe_timeout()
            })??;
        let created: Value = serde_json::from_str(&raw).map_err(|_| {
            ToniaError::invalid_request("realtime handshake failed", "realtime_handshake_invalid")
        })?;
        if !created.is_object() {
            return Err(err_with_body(
                ToniaError::invalid_request(
                    "realtime handshake failed",
                    "realtime_handshake_invalid",
                ),
                created,
            ));
        }
        RealtimeSession::from_created(ws, created)
    }
}

#[cfg(feature = "realtime")]
pub use live::{connect_realtime, RealtimeSession};

#[cfg(not(feature = "realtime"))]
pub async fn connect_realtime(
    _api_key: Option<&str>,
    _base_url: &str,
    _realtime_url: Option<&str>,
    _timeout: Duration,
    _opts: RealtimeConnect,
) -> Result<(), ToniaError> {
    Err(ToniaError::invalid_request(
        "crate built without the realtime feature",
        "realtime_feature_disabled",
    ))
}
