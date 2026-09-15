use serde_json::{json, Map, Value};
use tonia_sdk::{
    error_from_http_fallback, error_from_structured, raise_from_response_body,
    raise_from_stream_headers, ErrorKind,
};

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn structured(error_type: &str) -> Map<String, Value> {
    json!({
        "type": error_type,
        "code": "synthetic_code",
        "message": "Synthetic",
        "retryable": true,
    })
    .as_object()
    .cloned()
    .expect("object")
}

#[test]
fn http_200_policy_carrier() {
    let err = raise_from_response_body(
        &json!({ "_tonia_policy_block": { "code": "regulated_content_detected" } }),
        200,
        &[],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PolicyBlock);
    assert_eq!(
        err.policy_block().unwrap()["code"],
        "regulated_content_detected"
    );
}

#[test]
fn http_200_agent_carrier() {
    let err = raise_from_response_body(
        &json!({
            "_tonia_agent_block": {
                "code": "tool_calls_disabled_by_profile",
                "capability": "tool_calls",
                "retryable": false,
            }
        }),
        200,
        &[],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::AgentBlock);
    assert_eq!(
        err.agent_block().unwrap()["code"],
        "tool_calls_disabled_by_profile"
    );
    assert_eq!(err.code.as_deref(), Some("tool_calls_disabled_by_profile"));
    assert_eq!(err.capability(), Some("tool_calls"));
    assert_eq!(err.retryable, Some(false));
}

#[test]
fn http_200_entitlement_carrier() {
    let err = raise_from_response_body(
        &json!({
            "_tonia_entitlement_block": {
                "code": "managed_budget_exhausted",
                "retryable": false,
            }
        }),
        200,
        &[],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Entitlement);
    assert_eq!(
        err.entitlement_block().unwrap()["code"],
        "managed_budget_exhausted"
    );
    assert_eq!(err.code.as_deref(), Some("managed_budget_exhausted"));
    assert_eq!(err.retryable, Some(false));
}

#[test]
fn public_model_flat_404_is_typed() {
    let err = raise_from_response_body(&json!({ "error": "not_found" }), 404, &[]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code.as_deref(), Some("not_found"));
    assert_eq!(err.retryable, Some(false));
}

#[test]
fn structured_error_taxonomy_maps_to_specific_kind() {
    let cases = [
        ("authentication_error", ErrorKind::Authentication),
        ("billing_error", ErrorKind::Billing),
        ("entitlement_error", ErrorKind::Entitlement),
        ("invalid_request_error", ErrorKind::InvalidRequest),
        ("byok_key_missing", ErrorKind::ByokKeyMissing),
        ("policy_block", ErrorKind::PolicyBlock),
        ("agent_block", ErrorKind::AgentBlock),
        ("tenant_upstream_blocked", ErrorKind::TenantUpstreamBlocked),
        (
            "managed_credential_unavailable",
            ErrorKind::ManagedCredentialUnavailable,
        ),
        ("rate_limit_error", ErrorKind::RateLimit),
        ("api_error", ErrorKind::Api),
    ];
    for (error_type, expected) in cases {
        let error = error_from_structured(
            &structured(error_type),
            Some(503),
            None,
            headers(&[("retry-after", "60")]),
        );
        assert_eq!(error.kind, expected, "{error_type}");
        assert_eq!(error.code.as_deref(), Some("synthetic_code"));
        assert_eq!(error.retryable, Some(true));
        assert_eq!(error.status, Some(503));
        assert_eq!(error.header("retry-after"), Some("60"));
        assert_eq!(error.retry_after_seconds(), Some(60));
    }
}

#[test]
fn unknown_structured_type_stays_generic() {
    let error = error_from_structured(&structured("not_a_real_type"), Some(500), None, Vec::new());
    assert_eq!(error.kind, ErrorKind::Tonia);
    assert_eq!(error.r#type, "not_a_real_type");
}

#[test]
fn stream_policy_header_raises() {
    let err = raise_from_stream_headers(&headers(&[(
        "x-tonia-policy-block",
        "regulated_content_detected",
    )]))
    .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("regulated_content_detected"));
    assert_eq!(err.retryable, Some(false));
    assert_eq!(
        err.policy_block().unwrap()["code"],
        "regulated_content_detected"
    );
}

#[test]
fn stream_agent_header_raises() {
    let err = raise_from_stream_headers(&headers(&[
        ("x-tonia-agent-block", "tool_calls_disabled_by_profile"),
        ("x-tonia-agent-capability", "tool_calls"),
    ]))
    .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("tool_calls_disabled_by_profile"));
    assert_eq!(err.capability(), Some("tool_calls"));
    assert_eq!(err.retryable, Some(false));
    assert_eq!(
        err.agent_block().unwrap()["code"],
        "tool_calls_disabled_by_profile"
    );
}

#[test]
fn stream_entitlement_header_raises() {
    let err = raise_from_stream_headers(&headers(&[(
        "x-tonia-entitlement-block",
        "managed_budget_exhausted",
    )]))
    .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("managed_budget_exhausted"));
    assert_eq!(err.retryable, Some(true));
}

#[test]
fn stream_headers_without_block_are_noop() {
    raise_from_stream_headers(&headers(&[("content-type", "text/event-stream")])).unwrap();
}

#[test]
fn provider_requires_surface_is_invalid_request() {
    let err = raise_from_response_body(
        &json!({
            "error": {
                "type": "invalid_request_error",
                "code": "provider_requires_surface",
                "required_surface": "interactions",
                "retryable": false,
            }
        }),
        400,
        &[],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.code.as_deref(), Some("provider_requires_surface"));
    assert_eq!(err.retryable, Some(false));
    assert_eq!(
        err.body().unwrap()["error"]["required_surface"],
        "interactions"
    );
}

#[test]
fn admission_429_lifts_reason_scope_and_retry_after() {
    let err = raise_from_response_body(
        &json!({
            "error": {
                "type": "rate_limit_error",
                "code": "admission_rate_limited",
                "reason": "rpm_per_key",
                "scope": "key",
                "retryable": true,
            }
        }),
        429,
        &headers(&[("Retry-After", "47")]),
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::RateLimit);
    assert_eq!(err.code.as_deref(), Some("admission_rate_limited"));
    assert_eq!(err.reason(), Some("rpm_per_key"));
    assert_eq!(err.scope(), Some("key"));
    assert_eq!(err.retryable, Some(true));
    assert_eq!(err.retry_after_seconds(), Some(47));
}

#[test]
fn quota_429_is_entitlement_not_rate_limit() {
    let err = raise_from_response_body(
        &json!({
            "error": {
                "type": "entitlement_error",
                "code": "request_quota_exhausted",
                "scope": "organization",
                "retryable": true,
            }
        }),
        429,
        &headers(&[("Retry-After", "86400")]),
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Entitlement);
    assert_eq!(err.code.as_deref(), Some("request_quota_exhausted"));
    assert_eq!(err.scope(), Some("organization"));
    assert_eq!(err.retryable, Some(true));
    assert_eq!(err.retry_after_seconds(), Some(86400));
}

#[test]
fn http_200_entitlement_carrier_lifts_retry_after_and_scope() {
    let err = raise_from_response_body(
        &json!({
            "_tonia_entitlement_block": {
                "type": "entitlement_block",
                "code": "request_quota_exhausted",
                "scope": "organization",
                "retryable": true,
            }
        }),
        200,
        &headers(&[("Retry-After", "3600")]),
    )
    .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("request_quota_exhausted"));
    assert_eq!(err.scope(), Some("organization"));
    assert_eq!(err.retryable, Some(true));
    assert_eq!(err.retry_after_seconds(), Some(3600));
}

#[test]
fn audit_tip_contention_parses_retry_after_and_request_id() {
    let err = raise_from_response_body(
        &json!({
            "error": {
                "type": "api_error",
                "code": "audit_tip_contention",
                "retryable": true,
            }
        }),
        503,
        &headers(&[
            ("Retry-After", "1"),
            ("x-tonia-request-id", "tonia_req_abc"),
        ]),
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Api);
    assert_eq!(err.code.as_deref(), Some("audit_tip_contention"));
    assert_eq!(err.retryable, Some(true));
    assert_eq!(err.retry_after_seconds(), Some(1));
    assert_eq!(err.request_id(), Some("tonia_req_abc"));
}

#[test]
fn http_date_retry_after_is_ignored() {
    let error = error_from_structured(
        json!({
            "type": "rate_limit_error",
            "code": "admission_rate_limited",
            "retryable": true,
        })
        .as_object()
        .unwrap(),
        Some(429),
        None,
        headers(&[("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")]),
    );
    assert_eq!(error.kind, ErrorKind::RateLimit);
    assert_eq!(error.retry_after_seconds(), None);
}

#[test]
fn bare_429_fallback_is_retryable_rate_limit() {
    let error = error_from_http_fallback(429, None, headers(&[("Retry-After", "2")]));
    assert_eq!(error.kind, ErrorKind::RateLimit);
    assert_eq!(error.retryable, Some(true));
    assert_eq!(error.retry_after_seconds(), Some(2));
    assert_eq!(error.reason(), None);
}

#[test]
fn bare_503_fallback_is_retryable_api_error() {
    let error = error_from_http_fallback(503, None, headers(&[("Retry-After", "60")]));
    assert_eq!(error.kind, ErrorKind::Api);
    assert_eq!(error.retryable, Some(true));
    assert_eq!(error.retry_after_seconds(), Some(60));
}

#[test]
fn bare_400_fallback_stays_non_retryable() {
    let error = error_from_http_fallback(400, None, Vec::new());
    assert_eq!(error.r#type, "invalid_request_error");
    assert_eq!(error.kind, ErrorKind::Tonia);
    assert_eq!(error.retryable, Some(false));
}

#[test]
fn bare_502_fallback_is_retryable_api_error() {
    let error = error_from_http_fallback(502, None, Vec::new());
    assert_eq!(error.kind, ErrorKind::Api);
    assert_eq!(error.retryable, Some(true));
}

#[test]
fn string_error_envelope_leaves_retryable_none() {
    let err =
        raise_from_response_body(&json!({ "error": "upstream_said_no" }), 400, &[]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Tonia);
    assert_eq!(err.r#type, "invalid_request_error");
    assert_eq!(err.retryable, None);
    assert_eq!(err.message, "upstream_said_no");
}
