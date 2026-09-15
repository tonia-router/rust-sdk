//! Shared request helpers.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

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

/// Same unreserved set as Python `urllib.parse.quote(..., safe="")`.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub fn encode_path_segment(id: &str) -> String {
    utf8_percent_encode(id, PATH_SEGMENT).to_string()
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

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct InvalidTranscriptionFile(pub String);

impl InvalidTranscriptionFile {
    pub fn data_uri() -> Self {
        Self(
            "file must be audio bytes, a path, or a binary file object — \
             not a data URI. Send the audio file itself."
                .to_string(),
        )
    }
}

/// STT `data:` / unreadable file is not a [`ToniaError`] kind (Python `ValueError`).
#[derive(Debug, thiserror::Error)]
pub enum TranscriptionError {
    #[error(transparent)]
    InvalidFile(#[from] InvalidTranscriptionFile),
    #[error(transparent)]
    Api(#[from] ToniaError),
}

pub fn reject_data_uri(file: &str) -> Result<(), InvalidTranscriptionFile> {
    if file.trim_start().to_ascii_lowercase().starts_with("data:") {
        return Err(InvalidTranscriptionFile::data_uri());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub enum TranscriptionFile {
    Bytes(Bytes),
    Path(std::path::PathBuf),
}

impl From<Vec<u8>> for TranscriptionFile {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(Bytes::from(value))
    }
}

impl From<Bytes> for TranscriptionFile {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for TranscriptionFile {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(Bytes::copy_from_slice(value))
    }
}

impl From<std::path::PathBuf> for TranscriptionFile {
    fn from(value: std::path::PathBuf) -> Self {
        Self::Path(value)
    }
}

impl From<&std::path::Path> for TranscriptionFile {
    fn from(value: &std::path::Path) -> Self {
        Self::Path(value.to_path_buf())
    }
}

impl TryFrom<&str> for TranscriptionFile {
    type Error = InvalidTranscriptionFile;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from_user(value)
    }
}

impl TryFrom<String> for TranscriptionFile {
    type Error = InvalidTranscriptionFile;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from_user(&value)
    }
}

impl TranscriptionFile {
    pub fn try_from_user(file: &str) -> Result<Self, InvalidTranscriptionFile> {
        reject_data_uri(file)?;
        Ok(Self::Path(std::path::PathBuf::from(file)))
    }

    pub fn read(
        &self,
        filename: Option<&str>,
    ) -> Result<(String, Bytes, &'static str), InvalidTranscriptionFile> {
        match self {
            Self::Bytes(bytes) => {
                let name = filename.unwrap_or("audio.wav").to_string();
                let mime = audio_mime_for_name(&name);
                Ok((name, bytes.clone(), mime))
            }
            Self::Path(path) => {
                if let Some(text) = path.to_str() {
                    reject_data_uri(text)?;
                }
                let content = std::fs::read(path).map_err(|err| {
                    InvalidTranscriptionFile(format!("cannot read transcription file: {err}"))
                })?;
                let name = filename
                    .map(str::to_string)
                    .or_else(|| {
                        path.file_name()
                            .and_then(|value| value.to_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "audio.wav".to_string());
                let mime = audio_mime_for_name(&name);
                Ok((name, Bytes::from(content), mime))
            }
        }
    }
}

impl ToniaError {
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
