//! Typed errors raised by the tonia Pass client.

use serde_json::{Map, Value};

use crate::limits::header_get;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Tonia,
    Authentication,
    Billing,
    Entitlement,
    InvalidRequest,
    ByokKeyMissing,
    PolicyBlock,
    AgentBlock,
    TenantUpstreamBlocked,
    ManagedCredentialUnavailable,
    RateLimit,
    Api,
    PathNotAllowed,
}

#[derive(Debug, Clone, Default)]
struct ErrorExtras {
    body: Option<Value>,
    headers: Vec<(String, String)>,
    retry_after_seconds: Option<u64>,
    request_id: Option<String>,
    entitlement_block: Option<Value>,
    scope: Option<String>,
    policy_block: Option<Value>,
    agent_block: Option<Value>,
    capability: Option<String>,
    provider: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ToniaError {
    pub message: String,
    pub kind: ErrorKind,
    pub r#type: String,
    pub code: Option<String>,
    pub retryable: Option<bool>,
    pub status: Option<u16>,
    extras: Box<ErrorExtras>,
}

pub fn parse_retry_after_seconds(headers: &[(String, String)]) -> Option<u64> {
    let raw = header_get(headers, "retry-after")?.trim();
    if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

pub fn parse_request_id(headers: &[(String, String)]) -> Option<String> {
    let raw = header_get(headers, "x-tonia-request-id")?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

impl ToniaError {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        message: impl Into<String>,
        kind: ErrorKind,
        r#type: impl Into<String>,
        code: Option<String>,
        retryable: Option<bool>,
        status: Option<u16>,
        body: Option<Value>,
        headers: Vec<(String, String)>,
    ) -> Self {
        let retry_after_seconds = parse_retry_after_seconds(&headers);
        let request_id = parse_request_id(&headers);
        Self {
            message: message.into(),
            kind,
            r#type: r#type.into(),
            code,
            retryable,
            status,
            extras: Box::new(ErrorExtras {
                body,
                headers,
                retry_after_seconds,
                request_id,
                ..ErrorExtras::default()
            }),
        }
    }

    pub fn path_not_allowed(path: &str) -> Self {
        Self::new(
            format!("Path is not supported by this SDK: {path}"),
            ErrorKind::PathNotAllowed,
            "client_error",
            Some("path_not_allowed".to_string()),
            Some(false),
            None,
            None,
            Vec::new(),
        )
    }

    pub fn invalid_request(message: impl Into<String>, code: &str) -> Self {
        Self::new(
            message,
            ErrorKind::InvalidRequest,
            "invalid_request_error",
            Some(code.to_string()),
            Some(false),
            None,
            None,
            Vec::new(),
        )
    }

    pub fn missing_bearer() -> Self {
        Self::new(
            "Missing TONIA_API_KEY",
            ErrorKind::Authentication,
            "authentication_error",
            Some("missing_bearer".to_string()),
            Some(false),
            None,
            None,
            Vec::new(),
        )
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        header_get(&self.extras.headers, name)
    }

    pub fn body(&self) -> Option<&Value> {
        self.extras.body.as_ref()
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.extras.headers
    }

    pub fn retry_after_seconds(&self) -> Option<u64> {
        self.extras.retry_after_seconds
    }

    pub fn request_id(&self) -> Option<&str> {
        self.extras.request_id.as_deref()
    }

    pub fn entitlement_block(&self) -> Option<&Value> {
        self.extras.entitlement_block.as_ref()
    }

    pub fn scope(&self) -> Option<&str> {
        self.extras.scope.as_deref()
    }

    pub fn policy_block(&self) -> Option<&Value> {
        self.extras.policy_block.as_ref()
    }

    pub fn agent_block(&self) -> Option<&Value> {
        self.extras.agent_block.as_ref()
    }

    pub fn capability(&self) -> Option<&str> {
        self.extras.capability.as_deref()
    }

    pub fn provider(&self) -> Option<&str> {
        self.extras.provider.as_deref()
    }

    pub fn reason(&self) -> Option<&str> {
        self.extras.reason.as_deref()
    }

    fn with_scope(mut self, scope: Option<String>) -> Self {
        self.extras.scope = scope;
        self
    }

    fn with_capability(mut self, capability: Option<String>) -> Self {
        self.extras.capability = capability;
        self
    }

    fn with_provider(mut self, provider: Option<String>) -> Self {
        self.extras.provider = provider;
        self
    }

    fn with_reason(mut self, reason: Option<String>) -> Self {
        self.extras.reason = reason;
        self
    }

    fn with_policy_block(mut self, block: Value) -> Self {
        self.extras.policy_block = Some(block);
        self
    }

    fn with_agent_block(mut self, block: Value) -> Self {
        self.extras.agent_block = Some(block);
        self
    }

    fn with_entitlement_block(mut self, block: Value) -> Self {
        self.extras.entitlement_block = Some(block);
        self
    }
}

fn string_field(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_string)
}

fn json_code(carrier: &Value) -> Option<String> {
    let code = carrier.as_object()?.get("code")?;
    if code.is_null() {
        return None;
    }
    match code {
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(false) => None,
        other => Some(other.to_string()),
    }
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn carrier_retryable(carrier: &Value, default: bool) -> bool {
    let Some(obj) = carrier.as_object() else {
        return default;
    };
    match obj.get("retryable") {
        None | Some(Value::Null) => default,
        Some(value) => json_truthy(value),
    }
}

fn string_from_value(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|value| match value {
        Value::String(s) => Some(s.clone()),
        _ => None,
    })
}

pub fn error_from_structured(
    err: &Map<String, Value>,
    status: Option<u16>,
    body: Option<Value>,
    headers: Vec<(String, String)>,
) -> ToniaError {
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| err.get("code").and_then(Value::as_str))
        .or_else(|| err.get("type").and_then(Value::as_str))
        .unwrap_or("tonia API error")
        .to_string();
    let code = string_field(err, "code");
    let retryable = err.get("retryable").and_then(Value::as_bool);
    let error_type = err
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    match error_type.as_str() {
        "authentication_error" => ToniaError::new(
            message,
            ErrorKind::Authentication,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        "billing_error" => ToniaError::new(
            message,
            ErrorKind::Billing,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        "entitlement_error" => ToniaError::new(
            message,
            ErrorKind::Entitlement,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        )
        .with_scope(string_from_value(err, "scope")),
        "invalid_request_error" => ToniaError::new(
            message,
            ErrorKind::InvalidRequest,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        "byok_key_missing" => ToniaError::new(
            message,
            ErrorKind::ByokKeyMissing,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        "policy_block" => ToniaError::new(
            message,
            ErrorKind::PolicyBlock,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        "agent_block" => ToniaError::new(
            message,
            ErrorKind::AgentBlock,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        )
        .with_capability(string_from_value(err, "capability")),
        "tenant_upstream_blocked" => ToniaError::new(
            message,
            ErrorKind::TenantUpstreamBlocked,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        "managed_credential_unavailable" => ToniaError::new(
            message,
            ErrorKind::ManagedCredentialUnavailable,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        )
        .with_provider(string_from_value(err, "provider")),
        "rate_limit_error" => ToniaError::new(
            message,
            ErrorKind::RateLimit,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        )
        .with_reason(string_from_value(err, "reason"))
        .with_scope(string_from_value(err, "scope")),
        "api_error" => ToniaError::new(
            message,
            ErrorKind::Api,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
        _ => ToniaError::new(
            message,
            ErrorKind::Tonia,
            error_type,
            code,
            retryable,
            status,
            body,
            headers,
        ),
    }
}

pub fn error_from_http_fallback(
    status: u16,
    body: Option<Value>,
    headers: Vec<(String, String)>,
) -> ToniaError {
    let message = format!("HTTP {status}");
    if status == 429 {
        return ToniaError::new(
            message,
            ErrorKind::RateLimit,
            "rate_limit_error",
            None,
            Some(true),
            Some(status),
            body,
            headers,
        );
    }
    if status == 502 || status == 503 {
        return ToniaError::new(
            message,
            ErrorKind::Api,
            "api_error",
            None,
            Some(true),
            Some(status),
            body,
            headers,
        );
    }
    ToniaError::new(
        message,
        ErrorKind::Tonia,
        "invalid_request_error",
        None,
        Some(false),
        Some(status),
        body,
        headers,
    )
}

pub fn raise_from_stream_headers(headers: &[(String, String)]) -> Result<(), ToniaError> {
    if let Some(policy) = header_get(headers, "x-tonia-policy-block") {
        return Err(ToniaError::new(
            "tonia policy block",
            ErrorKind::PolicyBlock,
            "policy_block",
            Some(policy.to_string()),
            Some(false),
            Some(200),
            None,
            headers.to_vec(),
        )
        .with_policy_block(serde_json::json!({ "code": policy })));
    }
    if let Some(entitlement) = header_get(headers, "x-tonia-entitlement-block") {
        return Err(ToniaError::new(
            "tonia entitlement block",
            ErrorKind::Entitlement,
            "entitlement_error",
            Some(entitlement.to_string()),
            Some(true),
            Some(200),
            None,
            headers.to_vec(),
        )
        .with_entitlement_block(serde_json::json!({ "code": entitlement })));
    }
    if let Some(agent) = header_get(headers, "x-tonia-agent-block") {
        let capability = header_get(headers, "x-tonia-agent-capability").map(str::to_string);
        return Err(ToniaError::new(
            "tonia agent block",
            ErrorKind::AgentBlock,
            "agent_block",
            Some(agent.to_string()),
            Some(false),
            Some(200),
            None,
            headers.to_vec(),
        )
        .with_capability(capability.clone())
        .with_agent_block(serde_json::json!({
            "code": agent,
            "capability": capability,
        })));
    }
    Ok(())
}

pub fn raise_from_response_body(
    body: &Value,
    status: u16,
    headers: &[(String, String)],
) -> Result<(), ToniaError> {
    let Some(obj) = body.as_object() else {
        return Ok(());
    };

    if status == 200 {
        if let Some(carrier) = obj.get("_tonia_policy_block") {
            return Err(ToniaError::new(
                "tonia policy block",
                ErrorKind::PolicyBlock,
                "policy_block",
                json_code(carrier),
                Some(carrier_retryable(carrier, false)),
                Some(200),
                Some(body.clone()),
                headers.to_vec(),
            )
            .with_policy_block(carrier.clone()));
        }
        if let Some(carrier) = obj.get("_tonia_agent_block") {
            return Err(ToniaError::new(
                "tonia agent block",
                ErrorKind::AgentBlock,
                "agent_block",
                json_code(carrier),
                Some(carrier_retryable(carrier, false)),
                Some(200),
                Some(body.clone()),
                headers.to_vec(),
            )
            .with_agent_block(carrier.clone())
            .with_capability(
                carrier
                    .as_object()
                    .and_then(|m| string_from_value(m, "capability")),
            ));
        }
        if let Some(carrier) = obj.get("_tonia_entitlement_block") {
            return Err(ToniaError::new(
                "tonia entitlement block",
                ErrorKind::Entitlement,
                "entitlement_error",
                json_code(carrier),
                Some(carrier_retryable(carrier, true)),
                Some(200),
                Some(body.clone()),
                headers.to_vec(),
            )
            .with_entitlement_block(carrier.clone())
            .with_scope(
                carrier
                    .as_object()
                    .and_then(|m| string_from_value(m, "scope")),
            ));
        }
        return Ok(());
    }

    if status == 404 && obj.get("error").and_then(Value::as_str) == Some("not_found") {
        return Err(ToniaError::new(
            "not_found",
            ErrorKind::InvalidRequest,
            "invalid_request_error",
            Some("not_found".to_string()),
            Some(false),
            Some(404),
            Some(body.clone()),
            headers.to_vec(),
        ));
    }

    match obj.get("error") {
        Some(Value::Object(envelope)) => Err(error_from_structured(
            envelope,
            Some(status),
            Some(body.clone()),
            headers.to_vec(),
        )),
        Some(Value::String(envelope)) => Err(ToniaError::new(
            envelope.clone(),
            ErrorKind::Tonia,
            "invalid_request_error",
            None,
            None,
            Some(status),
            Some(body.clone()),
            headers.to_vec(),
        )),
        _ => Ok(()),
    }
}
