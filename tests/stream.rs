use serde_json::json;
use tonia_sdk::{feed_sse, limits_from_headers, raise_if_stream_carrier, ErrorKind};

#[test]
fn feed_sse_parses_and_keeps_remainder() {
    let (events, rest) = feed_sse("", "data: {\"a\":1}\n\ndata: {\"b\"");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].json, Some(json!({ "a": 1 })));
    assert_eq!(rest, "data: {\"b\"");
}

#[test]
fn feed_sse_yields_done() {
    let (events, rest) = feed_sse("", "data: [DONE]\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "[DONE]");
    assert_eq!(events[0].json, None);
    assert_eq!(rest, "");
}

#[test]
fn feed_sse_splits_crlf_frames() {
    let (events, rest) = feed_sse("", "data: {\"a\":1}\r\n\r\ndata: {\"b\":2}\r\n\r\n");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].json, Some(json!({ "a": 1 })));
    assert_eq!(events[1].json, Some(json!({ "b": 2 })));
    assert_eq!(rest, "");
}

#[test]
fn stream_carrier_nested_anthropic_message_start() {
    let err = raise_if_stream_carrier(&json!({
        "type": "message_start",
        "message": {
            "_tonia_policy_block": { "code": "regulated_content_detected" }
        }
    }))
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PolicyBlock);
}

#[test]
fn stream_carrier() {
    let policy = raise_if_stream_carrier(&json!({
        "_tonia_policy_block": { "code": "regulated_content_detected" }
    }))
    .unwrap_err();
    assert_eq!(policy.kind, ErrorKind::PolicyBlock);

    let entitlement = raise_if_stream_carrier(&json!({
        "_tonia_entitlement_block": { "code": "managed_budget_exhausted" }
    }))
    .unwrap_err();
    assert_eq!(entitlement.kind, ErrorKind::Entitlement);
}

#[test]
fn limits_from_headers_reads_seven_keys() {
    let limits = limits_from_headers(&[
        (
            "x-tonia-limit-warning".to_string(),
            "usage_limit_80_percent".to_string(),
        ),
        ("x-tonia-limit-remaining".to_string(), "17".to_string()),
    ])
    .expect("limits");
    assert_eq!(limits.warning.as_deref(), Some("usage_limit_80_percent"));
    assert_eq!(limits.remaining.as_deref(), Some("17"));
}
