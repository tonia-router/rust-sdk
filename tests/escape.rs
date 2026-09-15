use tonia_sdk::{assert_path_allowed, normalize_path, ErrorKind};

#[test]
fn normalize_path_strips_query_collapses_slashes_and_resolves_dots() {
    assert_eq!(normalize_path("v1/models?x=1"), "/v1/models");
    assert_eq!(
        normalize_path("//v1//chat//completions"),
        "/v1/chat/completions"
    );
    assert_eq!(
        normalize_path("/v1/models/../chat/completions"),
        "/v1/chat/completions"
    );
}

#[test]
fn supported_prefixes_pass() {
    for path in [
        "/v1/models",
        "/v1/chat/completions",
        "/v1/public/catalogue",
        "/v1/status",
        "/v1/interactions",
        "/v1/audio/speech",
        "/v1/audio/transcriptions",
    ] {
        assert_eq!(assert_path_allowed(path).unwrap(), path);
    }
}

#[test]
fn dot_segment_traversal_to_supported_prefix_is_allowed() {
    assert_eq!(
        assert_path_allowed("/v1/models/../chat/completions").unwrap(),
        "/v1/chat/completions"
    );
}

#[test]
fn unsupported_paths_rejected_before_send() {
    for path in [
        "/v1/billing/checkout",
        "/v1/conversations",
        "/v1/conversations/export",
        "/healthz",
        "/v1/models/../billing/checkout",
        "/v1/realtime",
    ] {
        let err = assert_path_allowed(path).unwrap_err();
        assert_eq!(err.kind, ErrorKind::PathNotAllowed);
        assert_eq!(err.r#type, "client_error");
        assert_eq!(err.code.as_deref(), Some("path_not_allowed"));
        assert_eq!(err.retryable, Some(false));
    }
}
