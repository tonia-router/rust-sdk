# rust-sdk

Official Rust client for [tonia Pass](https://pass.tonia.ca).

**Crate:** `tonia-sdk` **0.4.1** — GitHub `main` / tag `v0.4.1`. Not on
crates.io yet. Do not `cargo add tonia-sdk` from the registry.
**API contract:** [`tonia-api`](https://github.com/tonia-router/tonia-api)

```toml
tonia-sdk = { git = "https://github.com/tonia-router/rust-sdk", tag = "v0.4.1" }
# local Tonia tree:
# tonia-sdk = { path = "../tonia-router/rust-sdk" }
```

```rust
use tonia_sdk::{Tonia, DEFAULT_BASE_URL};

#[tokio::main]
async fn main() -> Result<(), tonia_sdk::ToniaError> {
    let client = Tonia::builder()
        .api_key(std::env::var("TONIA_API_KEY").expect("TONIA_API_KEY"))
        .base_url(DEFAULT_BASE_URL) // hosted DEV must pass base_url; env is not read
        .header("X-Tonia-Title", "example")
        .build();

    let listed = client.models.list().await?;
    let ids = listed["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let _completion = client
        .chat
        .completions
        .create(serde_json::json!({
            "model": ids[0],
            "messages": [{ "role": "user", "content": "Bonjour" }],
        }))
        .await?;
    let _ = client.last_limits();

    use futures_util::StreamExt;
    let mut stream = client.chat.completions.stream(serde_json::json!({
        "model": ids[0],
        "messages": [{ "role": "user", "content": "Compte jusqu’à 3." }],
    }));
    while let Some(event) = stream.next().await {
        let _event = event?;
    }
    Ok(())
}
```

`.stream()` is incremental (`impl Stream`; pull with `futures_util::StreamExt`).
`create` with `"stream": true` returns `use_stream_helper` — call `.stream()`.
Live is `client.realtime.connect`. Speech returns raw bytes; STT is multipart
(`TranscriptionFile`, no `data:` URI).

User-Agent is `tonia-sdk-rs/0.4.1`. The crate reads `TONIA_API_KEY` and
`TONIA_REALTIME_URL` only. It does not read `TONIA_BASE_URL`. It does not
auto-retry — follow `retryable` and `retry_after_seconds` on
[`ToniaError`](src/errors.rs).

Image inputs must contain inline bytes, such as a
`data:image/png;base64,...` URL. Pass returns the non-retryable
`remote_image_url_not_supported` code for remote `http(s)` image links.

Copyright (c) 2026 tonia inc. Apache 2.0 — commercial use allowed. Keep
`NOTICE` if you copy this software.
