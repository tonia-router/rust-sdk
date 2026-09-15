//! Official Rust client for tonia Pass.

pub mod client;
pub mod errors;
pub mod escape;
pub mod limits;
pub mod realtime;
pub mod stream;
pub mod transport;

#[cfg(feature = "blocking")]
pub mod blocking;

pub use client::{RequestOptions, Tonia, ToniaBuilder};
pub use errors::{
    error_from_http_fallback, error_from_structured, raise_from_response_body,
    raise_from_stream_headers, ErrorKind, ToniaError,
};
pub use escape::{assert_path_allowed, normalize_path, ESCAPE_HATCH_PREFIXES};
pub use limits::{limits_from_headers, LimitInfo};
#[cfg(feature = "realtime")]
pub use realtime::{connect_realtime, RealtimeSession};
pub use realtime::{
    raise_if_realtime_error, realtime_connect_url, realtime_ws_url, RealtimeConnect,
    RealtimeConnectParams, FIRST_HOP_MODEL, FIRST_HOP_PROVIDER, REALTIME_PATH, TERMINAL_TURN_TYPES,
    TURN_TYPES,
};
pub use stream::{feed_sse, raise_if_stream_carrier, SseEvent, SseStream};
pub use transport::{
    audio_mime_for_name, encode_path_segment, reject_data_uri, AuthStyle, InvalidTranscriptionFile,
    TranscriptionError, TranscriptionFile, DEFAULT_BASE_URL, DEFAULT_TIMEOUT, IMAGE_TIMEOUT,
    SDK_USER_AGENT, SDK_VERSION,
};

pub const VERSION: &str = SDK_VERSION;
