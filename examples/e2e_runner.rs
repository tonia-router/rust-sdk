//! Live e2e runner for WEEKLY `sdk_rust_*` cells.
//!
//! Prints one JSON object `{ "language": "rust", "results": [...] }` to stdout.
//! Invoked by `sdk_live_e2e_sweep.py`.

use std::env;
use std::time::Instant;

use serde_json::{json, Value};
use tonia_sdk::{ErrorKind, RequestOptions, Tonia, ToniaError};

enum CellErr {
    Tonia(ToniaError),
    Assert(&'static str),
}

type CellResult = Result<Value, CellErr>;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let results = run().await;
    println!(
        "{}",
        json!({
            "language": "rust",
            "results": results,
        })
    );
}

async fn run() -> Vec<Value> {
    let mut results = Vec::new();
    let base_url = env::var("TONIA_BASE_URL").unwrap_or_else(|_| {
        env::var("PASS_BASE_URL").unwrap_or_else(|_| "https://pass.tonia.ca:8443".into())
    });
    let public = Tonia::builder().base_url(&base_url).build();

    push_cell(
        &mut results,
        "sdk_rust_public_catalogue",
        "/v1/public/catalogue",
        object_ok(public.catalogue.list().await, "catalogue_body_missing"),
    );
    push_cell(
        &mut results,
        "sdk_rust_public_models",
        "/v1/public/models",
        object_ok(
            public.public_models.list().await,
            "public_models_body_missing",
        ),
    );
    push_cell(
        &mut results,
        "sdk_rust_status",
        "/v1/status",
        object_ok(public.status.get().await, "status_body_missing"),
    );
    results.push(path_reject(&public).await);

    let mode = env::var("SDK_E2E_MODE").unwrap_or_else(|_| "thin".into());
    if mode == "public" {
        return results;
    }
    let Some(api_key) = env::var("TONIA_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        for (id, route) in [
            ("sdk_rust_runtime_models", "/v1/models"),
            ("sdk_rust_systemone", "/v1/systemone"),
        ] {
            results.push(skip(id, route, "requires_TONIA_API_KEY"));
        }
        return results;
    };

    let client = Tonia::builder()
        .api_key(api_key)
        .base_url(&base_url)
        .build();
    let listed = match client.models.list().await {
        Ok(body) => {
            push_cell(
                &mut results,
                "sdk_rust_runtime_models",
                "/v1/models",
                Ok(json!({ "body_object": true, "count": model_ids(&body).len() })),
            );
            body
        }
        Err(err) => {
            push_cell(
                &mut results,
                "sdk_rust_runtime_models",
                "/v1/models",
                Err(CellErr::Tonia(err)),
            );
            return results;
        }
    };

    if mode == "light" {
        return results;
    }

    let preferred = env::var("SDK_E2E_SYSTEMONE_MODEL").ok();
    match pick_systemone(&listed, preferred.as_deref()) {
        None => results.push(skip(
            "sdk_rust_systemone",
            "/v1/systemone",
            "runtime_sku_missing",
        )),
        Some(model) => push_cell(
            &mut results,
            "sdk_rust_systemone",
            "/v1/systemone",
            systemone(&client, &model).await,
        ),
    }
    results
}

async fn systemone(client: &Tonia, model: &str) -> CellResult {
    let body = client
        .request(
            "POST",
            "/v1/systemone",
            Some(json!({
                "model": model,
                "state": "The sky is blue.",
                "questions": {
                    "color": {"type": "noul", "instructions": "Is the sky blue?"},
                    "kind": {
                        "type": "choice",
                        "criteria": {
                            "sky": {"description": "open air"},
                            "sea": {"description": "water"}
                        }
                    },
                    "confidence": {
                        "type": "score",
                        "instructions": "How sure is this public fixture?",
                        "criteria": ["Low", "Medium", "High"]
                    }
                }
            })),
            RequestOptions::default(),
        )
        .await
        .map_err(CellErr::Tonia)?;
    let answers = body
        .get("answers")
        .and_then(Value::as_object)
        .filter(|map| !map.is_empty())
        .ok_or(CellErr::Assert("systemone_answers_missing"))?;
    let input_tokens = body
        .pointer("/usage/input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if input_tokens <= 0 {
        return Err(CellErr::Assert("systemone_usage_missing"));
    }
    Ok(json!({
        "model": model,
        "answer_count": answers.len(),
        "input_tokens": input_tokens,
        "usage_present": true
    }))
}

fn pick_systemone(listed: &Value, preferred: Option<&str>) -> Option<String> {
    let ids = model_ids(listed);
    if let Some(chosen) = preferred.map(str::trim).filter(|value| !value.is_empty()) {
        return ids.into_iter().find(|id| id == chosen);
    }
    listed
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|item| {
            let family = item
                .pointer("/surface/family")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let has_cap = item
                .get("capabilities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|cap| cap == "systemone");
            if family == "systemone" || has_cap {
                item.get("id").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        })
        .or_else(|| {
            ids.into_iter()
                .find(|id| id.ends_with("jev-latest") || id.ends_with("/jev-latest"))
        })
}

fn model_ids(listed: &Value) -> Vec<String> {
    listed
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn object_ok(result: Result<Value, ToniaError>, missing: &'static str) -> CellResult {
    let body = result.map_err(CellErr::Tonia)?;
    if body.as_object().is_none() {
        return Err(CellErr::Assert(missing));
    }
    Ok(json!({ "body_object": true }))
}

async fn path_reject(client: &Tonia) -> Value {
    let started = Instant::now();
    match client
        .request("GET", "/healthz", None, RequestOptions::default())
        .await
    {
        Err(err) if err.kind == ErrorKind::PathNotAllowed => json!({
            "cell_id": "sdk_rust_path_reject",
            "language": "rust",
            "route": "/healthz",
            "status": "passed",
            "detail": "ok",
            "duration_ms": started.elapsed().as_millis() as u64,
            "assertions": { "rejected_before_send": true }
        }),
        Err(err) => failed_tonia("sdk_rust_path_reject", "/healthz", started, &err),
        Ok(_) => json!({
            "cell_id": "sdk_rust_path_reject",
            "language": "rust",
            "route": "/healthz",
            "status": "failed",
            "detail": "path_not_rejected",
            "duration_ms": started.elapsed().as_millis() as u64,
        }),
    }
}

fn push_cell(results: &mut Vec<Value>, id: &str, route: &str, outcome: CellResult) {
    let started = Instant::now();
    results.push(match outcome {
        Ok(assertions) => json!({
            "cell_id": id,
            "language": "rust",
            "route": route,
            "status": "passed",
            "detail": "ok",
            "duration_ms": started.elapsed().as_millis() as u64,
            "assertions": assertions,
        }),
        Err(CellErr::Tonia(err)) => failed_tonia(id, route, started, &err),
        Err(CellErr::Assert(detail)) => json!({
            "cell_id": id,
            "language": "rust",
            "route": route,
            "status": "failed",
            "detail": detail,
            "duration_ms": started.elapsed().as_millis() as u64,
            "error_class": "AssertionError",
            "error_code": detail,
        }),
    });
}

fn failed_tonia(id: &str, route: &str, started: Instant, err: &ToniaError) -> Value {
    json!({
        "cell_id": id,
        "language": "rust",
        "route": route,
        "status": "failed",
        "detail": err.message,
        "duration_ms": started.elapsed().as_millis() as u64,
        "error_class": "ToniaError",
        "error_code": err.code,
        "status_code": err.status,
    })
}

fn skip(id: &str, route: &str, detail: &str) -> Value {
    json!({
        "cell_id": id,
        "language": "rust",
        "route": route,
        "status": "skipped",
        "detail": detail,
        "duration_ms": 0,
    })
}
