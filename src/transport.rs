//! Shared request helpers.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::errors::ToniaError;

pub const DEFAULT_BASE_URL: &str = "https://pass.tonia.ca:8443";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
pub const IMAGE_TIMEOUT: Duration = Duration::from_secs(300);
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SDK_USER_AGENT: &str = concat!("tonia-sdk-rs/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthStyle {
    #[default]
    Bearer,
    ApiKey,
    None,
}

pub fn build_headers(
    api_key: Option<&str>,
    default_headers: &HashMap<String, String>,
    auth: AuthStyle,
    headers: Option<&HashMap<String, String>>,
    accept: &str,
) -> Result<HashMap<String, String>, ToniaError> {
    let mut hdrs = HashMap::new();
    hdrs.insert("Accept".to_string(), accept.to_string());
    for (key, value) in default_headers {
        hdrs.insert(key.clone(), value.clone());
    }
    if let Some(extra) = headers {
        for (key, value) in extra {
            hdrs.insert(key.clone(), value.clone());
        }
    }
    if !hdrs
        .keys()
        .any(|key| key.eq_ignore_ascii_case("user-agent"))
    {
        hdrs.insert("User-Agent".to_string(), SDK_USER_AGENT.to_string());
    }
    if auth != AuthStyle::None {
        let key = api_key
            .filter(|value| !value.is_empty())
            .ok_or_else(ToniaError::missing_bearer)?;
        match auth {
            AuthStyle::ApiKey => {
                hdrs.insert("x-api-key".to_string(), key.to_string());
            }
            AuthStyle::Bearer => {
                hdrs.insert("Authorization".to_string(), format!("Bearer {key}"));
            }
            AuthStyle::None => {}
        }
    }
    Ok(hdrs)
}

pub fn join_url(base_url: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{base_url}{path}")
    } else {
        format!("{base_url}/{path}")
    }
}

pub fn encode_path_segment(id: &str) -> String {
    utf8_percent_encode(id, NON_ALPHANUMERIC).to_string()
}

pub fn headers_from_reqwest(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(key, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (key.as_str().to_string(), text.to_string()))
        })
        .collect()
}

pub fn is_binary_audio_content_type(content_type: &str) -> bool {
    let lowered = content_type.to_ascii_lowercase();
    lowered.starts_with("audio/") || lowered.starts_with("application/octet-stream")
}

const AUDIO_MIME: &[(&str, &str)] = &[
    (".wav", "audio/wav"),
    (".wave", "audio/wav"),
    (".mp3", "audio/mpeg"),
    (".m4a", "audio/mp4"),
    (".mp4", "audio/mp4"),
    (".webm", "audio/webm"),
    (".ogg", "audio/ogg"),
    (".flac", "audio/flac"),
];

pub fn audio_mime_for_name(name: &str) -> &'static str {
    let suffix = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()));
    suffix
        .as_deref()
        .and_then(|ext| {
            AUDIO_MIME
                .iter()
                .find(|(key, _)| *key == ext)
                .map(|(_, mime)| *mime)
        })
        .unwrap_or("application/octet-stream")
}

pub fn reject_data_uri(file: &str) -> Result<(), ToniaError> {
    if file.trim_start().to_ascii_lowercase().starts_with("data:") {
        return Err(ToniaError::invalid_transcription_file());
    }
    Ok(())
}

impl ToniaError {
    pub(crate) fn invalid_transcription_file() -> Self {
        Self::new(
            "file must be audio bytes, a path, or a binary file object — \
             not a data URI. Send the audio file itself.",
            crate::errors::ErrorKind::InvalidRequest,
            "invalid_request_error",
            Some("invalid_transcription_file".to_string()),
            Some(false),
            None,
            None,
            Vec::new(),
        )
    }

    pub(crate) fn transport(err: reqwest::Error) -> Self {
        let code = if err.is_timeout() {
            "timeout"
        } else {
            "transport"
        };
        Self::new(
            err.to_string(),
            crate::errors::ErrorKind::Tonia,
            "client_error",
            Some(code.to_string()),
            Some(err.is_timeout() || err.is_connect()),
            None,
            None,
            Vec::new(),
        )
    }
}
