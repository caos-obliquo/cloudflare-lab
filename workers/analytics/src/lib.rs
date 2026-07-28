//! Analytics Worker — D1 event tracking, Bearer auth.

use cloudflare_shared::{
    bootstrap::ensure_analytics_events_table,
    observability::{
        health::{DependencyHealth, HealthStatus},
        structured_log::Logger,
        trace_context::TraceContext,
    },
    response::json_error_response,
    session::validate_token,
    tracing::request_id_for_request,
};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::JsValue;
use worker::*;

static DB_BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
struct TrackRequest {
    event_type: String,
    event_data: Option<String>,
}

fn session_secret(env: &Env) -> Result<Vec<u8>> {
    Ok(env
        .var("SESSION_SECRET")
        .map_err(|_| Error::from("SESSION_SECRET not configured"))?
        .to_string()
        .into_bytes())
}

// Verify Bearer token via HMAC (new) or KV fallback (legacy sess_ tokens).
async fn verify_token(req: &Request, env: &Env) -> Result<Option<String>> {
    let auth_header = req.headers().get("Authorization")?.unwrap_or_default();
    let raw = auth_header.strip_prefix("Bearer ").unwrap_or("");
    if raw.is_empty() {
        return Ok(None);
    }

    // HMAC stateless token.
    if raw.starts_with("s2.") {
        let secret = session_secret(env)?;
        return Ok(validate_token(raw, &secret, "session"));
    }

    // Legacy KV token fallback.
    let kv = env.kv("SESSIONS")?;
    Ok(kv.get(raw).text().await?)
}

async fn require_auth(req: &Request, env: &Env) -> Result<Option<String>> {
    verify_token(req, env).await
}

fn check_bindings(env: &Env) -> Vec<DependencyHealth> {
    let mut r = Vec::new();
    let mut add = |name: &str, res: Result<(), String>| {
        let s = Date::now().as_millis() as f64;
        match res {
            Ok(()) => r.push(DependencyHealth::healthy(
                name,
                (Date::now().as_millis() as f64 - s) as u64,
            )),
            Err(e) => r.push(DependencyHealth::unhealthy(name, &e)),
        }
    };
    add("d1", env.d1("D1").map(|_| ()).map_err(|e| format!("{:?}", e)));
    add(
        "kv",
        env.kv("SESSIONS").map(|_| ()).map_err(|e| format!("{:?}", e)),
    );
    r
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    let path = url.path();
    let method = req.method().to_string();
    let req_id = request_id_for_request(&req)?;
    let ctx = TraceContext::from_request(&req)?;
    let logger = Logger::new("analytics");

    let start = Date::now().as_millis();
    logger.request(&method, path, &ctx).emit();

    if !DB_BOOTSTRAPPED.load(Ordering::Relaxed) {
        if let Ok(db) = env.d1("D1") {
            ensure_analytics_events_table(&db).await?;
            DB_BOOTSTRAPPED.store(true, Ordering::Relaxed);
        }
    }

    // Capture origin before req is moved into handlers.
    let req_origin = req.headers().get("Origin")?.unwrap_or_default();

    let mut resp = match (method.as_str(), path) {
        ("GET", "/") => json_response(
            200,
            &serde_json::json!({"status":"ok","service":"analytics-worker","routes":["/track","/events","/summary"]}),
        ),
        ("POST", "/track") => track(req, &env).await,
        ("GET", "/events") => events(req, &env).await,
        ("GET", "/summary") => summary(req, &env).await,
        ("GET", "/health") | ("GET", "/readyz") => {
            let results = check_bindings(&env);
            let all_ok = results.iter().all(|d| d.status == HealthStatus::Healthy);
            let overall = if all_ok {
                "healthy"
            } else if results.iter().any(|d| d.status == HealthStatus::Unhealthy) {
                "unhealthy"
            } else {
                "degraded"
            };
            json_response(
                if all_ok { 200 } else { 503 },
                &serde_json::json!({"status":overall,"checks":results}),
            )
        }
        ("GET", "/livez") => json_response(200, &serde_json::json!({"status":"alive"})),
        _ => json_error_response(404, "Not found", ""),
    }?;

    // CORS: reflect origin on all responses.
    if req_origin.is_empty() {
        resp.headers().set("Access-Control-Allow-Origin", "*")?;
    } else {
        resp.headers().set("Access-Control-Allow-Origin", &req_origin)?;
    }
    resp.headers()
        .set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")?;
    resp.headers()
        .set("Access-Control-Allow-Headers", "Content-Type, Authorization, traceparent, x-request-id")?;

    let duration_ms = Date::now().as_millis() - start;
    let status = resp.status_code();
    logger.response(&method, path, status, duration_ms, &ctx).emit();
    resp.headers().set("X-Request-Id", &req_id)?;
    ctx.inject_into_response(&mut resp)?;
    Ok(resp)
}

async fn track(mut req: Request, env: &Env) -> Result<Response> {
    if require_auth(&req, env).await?.is_none() {
        return json_error_response(401, "Unauthorized", "");
    }
    let body: TrackRequest = req.json().await?;
    let db = env.d1("D1")?;
    let data = body.event_data.unwrap_or_default();
    db.prepare("INSERT INTO analytics_events (event_type, event_data) VALUES (?1, ?2)")
        .bind(&[JsValue::from(&body.event_type), JsValue::from(&data)])?
        .run()
        .await?;
    json_response(
        201,
        &serde_json::json!({"status":"ok","message":"Event tracked","event_type":body.event_type}),
    )
}

async fn events(req: Request, env: &Env) -> Result<Response> {
    if require_auth(&req, env).await?.is_none() {
        return json_error_response(401, "Unauthorized", "");
    }
    let db = env.d1("D1")?;
    let query: std::collections::HashMap<String, String> = req
        .url()?
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let limit: u32 = query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    let offset: u32 = query.get("cursor").and_then(|v| v.parse().ok()).unwrap_or(0);

    let total = db
        .prepare("SELECT COUNT(*) as count FROM analytics_events")
        .first::<i64>(Some("count"))
        .await?
        .unwrap_or(0);
    let result = db.prepare("SELECT id, event_type, event_data, created_at FROM analytics_events ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")
        .bind(&[JsValue::from(limit as i64), JsValue::from(offset as i64)])?
        .all().await?;
    let rows = result.results::<serde_json::Value>()?;

    let next_offset = offset + limit;
    let has_more = (next_offset as i64) < total;
    let next_cursor = if has_more {
        serde_json::json!(next_offset)
    } else {
        serde_json::Value::Null
    };

    json_response(
        200,
        &serde_json::json!({"status":"ok","count":rows.len(),"events":rows,"limit":limit,"cursor":offset,"next_cursor":next_cursor,"total":total}),
    )
}

async fn summary(req: Request, env: &Env) -> Result<Response> {
    if require_auth(&req, env).await?.is_none() {
        return json_error_response(401, "Unauthorized", "");
    }
    let db = env.d1("D1")?;
    let total = db
        .prepare("SELECT COUNT(*) as count FROM analytics_events")
        .first::<i64>(Some("count"))
        .await?
        .unwrap_or(0);
    let by_type = db
        .prepare("SELECT event_type, COUNT(*) as count FROM analytics_events GROUP BY event_type ORDER BY count DESC")
        .all()
        .await?;
    let type_rows = by_type.results::<serde_json::Value>()?;
    json_response(
        200,
        &serde_json::json!({"status":"ok","total_events":total,"by_type":type_rows}),
    )
}

fn json_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;
    Ok(resp.with_status(status))
}
