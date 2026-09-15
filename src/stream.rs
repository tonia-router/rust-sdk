//! SSE helpers for streaming Pass responses.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
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

type ByteChunks = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin>>;
type OpenFuture = Pin<Box<dyn Future<Output = Result<Option<ByteChunks>, ToniaError>> + Send>>;

/// Incremental SSE. First poll opens the HTTP body; events are not buffered.
pub struct SseStream {
    state: SseStreamState,
}

enum SseStreamState {
    Connecting(OpenFuture),
    Open {
        buffer: String,
        chunks: ByteChunks,
        pending: VecDeque<SseEvent>,
        ended: bool,
    },
    Failed,
    Done,
}

impl SseStream {
    pub(crate) fn connecting<F>(future: F) -> Self
    where
        F: Future<Output = Result<Option<ByteChunks>, ToniaError>> + Send + 'static,
    {
        Self {
            state: SseStreamState::Connecting(Box::pin(future)),
        }
    }
}

impl Stream for SseStream {
    type Item = Result<SseEvent, ToniaError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                SseStreamState::Connecting(future) => match future.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(err)) => {
                        this.state = SseStreamState::Failed;
                        return Poll::Ready(Some(Err(err)));
                    }
                    Poll::Ready(Ok(None)) => {
                        this.state = SseStreamState::Done;
                        return Poll::Ready(None);
                    }
                    Poll::Ready(Ok(Some(chunks))) => {
                        this.state = SseStreamState::Open {
                            buffer: String::new(),
                            chunks,
                            pending: VecDeque::new(),
                            ended: false,
                        };
                    }
                },
                SseStreamState::Open {
                    buffer,
                    chunks,
                    pending,
                    ended,
                } => {
                    if let Some(event) = pending.pop_front() {
                        if let Some(json) = &event.json {
                            if let Err(err) = raise_if_stream_carrier(json) {
                                this.state = SseStreamState::Failed;
                                return Poll::Ready(Some(Err(err)));
                            }
                        }
                        return Poll::Ready(Some(Ok(event)));
                    }
                    if *ended {
                        this.state = SseStreamState::Done;
                        return Poll::Ready(None);
                    }
                    match Pin::new(chunks).poll_next(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(None) => {
                            if !buffer.trim().is_empty() {
                                let (events, _) = feed_sse(buffer, "\n\n");
                                pending.extend(events);
                            }
                            *ended = true;
                        }
                        Poll::Ready(Some(Err(err))) => {
                            this.state = SseStreamState::Failed;
                            return Poll::Ready(Some(Err(ToniaError::transport(err))));
                        }
                        Poll::Ready(Some(Ok(chunk))) => {
                            let text = String::from_utf8_lossy(&chunk);
                            let (events, rest) = feed_sse(buffer, &text);
                            *buffer = rest;
                            pending.extend(events);
                        }
                    }
                }
                SseStreamState::Failed | SseStreamState::Done => return Poll::Ready(None),
            }
        }
    }
}
