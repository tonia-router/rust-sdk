//! SSE helpers for streaming Pass responses.

use serde_json::Value;

use crate::errors::{raise_from_response_body, ToniaError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub data: String,
    pub raw: String,
    pub event: Option<String>,
    pub json: Option<Value>,
}

pub fn feed_sse(buffer: &str, chunk: &str) -> (Vec<SseEvent>, String) {
    let combined = format!("{buffer}{chunk}");
    let mut parts: Vec<&str> = split_sse_frames(&combined);
    let rest = parts.pop().unwrap_or("").to_string();
    let mut events = Vec::new();
    for block in parts {
        if block.trim().is_empty() || block.starts_with(':') {
            continue;
        }
        let mut event_name = None;
        let mut data_lines = Vec::new();
        for line in block.lines() {
            if let Some(name) = line.strip_prefix("event:") {
                event_name = Some(name.trim().to_string());
            } else if let Some(data) = line.strip_prefix("data:") {
                data_lines.push(data.trim_start().to_string());
            }
        }
        if data_lines.is_empty() {
            continue;
        }
        let data = data_lines.join("\n");
        let parsed = if !data.is_empty() && data != "[DONE]" {
            serde_json::from_str(&data).ok()
        } else {
            None
        };
        events.push(SseEvent {
            event: event_name,
            data,
            raw: block.to_string(),
            json: parsed,
        });
    }
    (events, rest)
}

fn split_sse_frames(combined: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = combined.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r'
            && i + 3 < bytes.len()
            && bytes[i + 1] == b'\n'
            && bytes[i + 2] == b'\r'
            && bytes[i + 3] == b'\n'
        {
            parts.push(&combined[start..i]);
            start = i + 4;
            i = start;
            continue;
        }
        if bytes[i] == b'\n' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            parts.push(&combined[start..i]);
            start = i + 2;
            i = start;
            continue;
        }
        i += 1;
    }
    parts.push(&combined[start..]);
    parts
}

pub fn raise_if_stream_carrier(payload: &Value) -> Result<(), ToniaError> {
    raise_from_response_body(payload, 200, &[])?;
    if let Some(nested) = payload.get("message") {
        if nested.is_object() {
            raise_from_response_body(nested, 200, &[])?;
        }
    }
    Ok(())
}
