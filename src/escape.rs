//! Supported path prefixes for [`crate::Tonia::request`].

use crate::errors::ToniaError;

pub const ESCAPE_HATCH_PREFIXES: &[&str] = &[
    "/v1/public/",
    "/v1/status",
    "/v1/models",
    "/v1/chat/",
    "/v1/messages",
    "/v1/embeddings",
    "/v1/images/",
    "/v1/audio/",
    "/v1/responses",
    "/v1/rerank",
    "/v1/interactions",
];

pub fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let with_slash = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    let no_query = with_slash
        .split_once('?')
        .map(|(head, _)| head)
        .unwrap_or(with_slash.as_str());
    let no_query = no_query
        .split_once('#')
        .map(|(head, _)| head)
        .unwrap_or(no_query);
    let mut collapsed = no_query.to_string();
    while collapsed.contains("//") {
        collapsed = collapsed.replace("//", "/");
    }
    let mut resolved: Vec<&str> = Vec::new();
    for part in collapsed.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            resolved.pop();
            continue;
        }
        resolved.push(part);
    }
    format!("/{}", resolved.join("/"))
}

pub fn assert_path_allowed(path: &str) -> Result<String, ToniaError> {
    let normalized = normalize_path(path);
    for prefix in ESCAPE_HATCH_PREFIXES {
        let allowed = if let Some(bare) = prefix.strip_suffix('/') {
            normalized == bare || normalized.starts_with(prefix)
        } else {
            normalized == *prefix || normalized.starts_with(&format!("{prefix}/"))
        };
        if allowed {
            return Ok(normalized);
        }
    }
    Err(ToniaError::path_not_allowed(&normalized))
}
