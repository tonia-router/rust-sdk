//! Soft-limit warning headers on successful runtime responses.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitInfo {
    pub warning: Option<String>,
    pub scope: Option<String>,
    pub kind: Option<String>,
    pub used: Option<String>,
    pub value: Option<String>,
    pub remaining: Option<String>,
    pub period_ends_at: Option<String>,
}

const LIMIT_KEYS: &[(&str, &str)] = &[
    ("warning", "x-tonia-limit-warning"),
    ("scope", "x-tonia-limit-scope"),
    ("kind", "x-tonia-limit-kind"),
    ("used", "x-tonia-limit-used"),
    ("value", "x-tonia-limit-value"),
    ("remaining", "x-tonia-limit-remaining"),
    ("period_ends_at", "x-tonia-limit-period-ends-at"),
];

pub fn header_get<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, value)| key.eq_ignore_ascii_case(name) && !value.is_empty())
        .map(|(_, value)| value.as_str())
}

pub fn limits_from_headers(headers: &[(String, String)]) -> Option<LimitInfo> {
    if headers.is_empty() {
        return None;
    }
    let mut info = LimitInfo {
        warning: None,
        scope: None,
        kind: None,
        used: None,
        value: None,
        remaining: None,
        period_ends_at: None,
    };
    let mut any_set = false;
    for (field, header) in LIMIT_KEYS {
        let value = header_get(headers, header).map(str::to_string);
        if value.as_ref().is_some_and(|v| !v.is_empty()) {
            any_set = true;
        }
        match *field {
            "warning" => info.warning = value,
            "scope" => info.scope = value,
            "kind" => info.kind = value,
            "used" => info.used = value,
            "value" => info.value = value,
            "remaining" => info.remaining = value,
            "period_ends_at" => info.period_ends_at = value,
            _ => {}
        }
    }
    any_set.then_some(info)
}
