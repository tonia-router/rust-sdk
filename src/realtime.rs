//! Mediated `/v1/realtime` WebSocket. Not an HTTP escape-hatch path.

use url::form_urlencoded::Serializer;
use url::Url;

use crate::errors::ToniaError;

pub const FIRST_HOP_PROVIDER: &str = "openai";
pub const FIRST_HOP_MODEL: &str = "gpt-live-1";
pub const REALTIME_PATH: &str = "/v1/realtime";
pub const LOCAL_DATA_PORT: u16 = 8444;
pub const LOCAL_REALTIME_PORT: u16 = 8448;

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
