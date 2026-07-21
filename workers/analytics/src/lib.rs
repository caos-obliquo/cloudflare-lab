use cloudflare_shared::session::validate_token;
use cloudflare_shared::tracing::request_id_for_request;
use cloudflare_shared::response::json_error_response;
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::web_sys;
use worker::*;

#[derive(Deserialize)]
struct TrackRequest {
    event_type: String,
    event_data: Option<String>,
}

fn session_secret(env: &Env) -> Result<Vec<u8>> {
    Ok(env.var("SESSION_SECRET")
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

async fn require_auth(req: &Request, env: &Env) -> Result<String> {
    match verify_token(req, env).await? {
        Some(username) => Ok(username),
        None => {
            let resp = json_error_response(401, "Unauthorized", "").unwrap();
            let web_resp: web_sys::Response = resp.into();
            Err(Error::from(JsValue::from(web_resp)))
        }
    }
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    let path = url.path();
    let method = req.method().to_string();
    let req_id = request_id_for_request(&req)?;
    console_log!("[req-{}] handling {} {}", req_id, method, path);

    let resp = match (method.as_str(), path) {
        ("GET", "/") => json_response(200, &serde_json::json!({"status":"ok","service":"analytics-worker","routes":["/track","/events","/summary"]})),
        ("POST", "/track") => track(req, &env).await,
        ("GET", "/events") => events(req, &env).await,
        ("GET", "/summary") => summary(req, &env).await,
        _ => json_error_response(404, "Not found", ""),
    }?;
    resp.headers().set("X-Request-Id", &req_id)?;
    Ok(resp)
}

async fn track(mut req: Request, env: &Env) -> Result<Response> {
    let _user = require_auth(&req, &env).await?;
    let body: TrackRequest = req.json().await?;
    let db = env.d1("D1")?;
    let data = body.event_data.unwrap_or_default();
    db.prepare("INSERT INTO analytics_events (event_type, event_data) VALUES (?1, ?2)")
        .bind(&[JsValue::from(&body.event_type), JsValue::from(&data)])?
        .run()
        .await?;
    json_response(201, &serde_json::json!({"status":"ok","message":"Event tracked","event_type":body.event_type}))
}

async fn events(req: Request, env: &Env) -> Result<Response> {
    let _user = require_auth(&req, &env).await?;
    let db = env.d1("D1")?;
    let query: std::collections::HashMap<String, String> = req.url()?.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let limit: u32 = query.get("limit").and_then(|v| v.parse().ok()).unwrap_or(20).clamp(1, 100);
    let offset: u32 = query.get("cursor").and_then(|v| v.parse().ok()).unwrap_or(0);

    let total = db.prepare("SELECT COUNT(*) as count FROM analytics_events")
        .first::<i64>(Some("count")).await?.unwrap_or(0);
    let result = db.prepare("SELECT id, event_type, event_data, created_at FROM analytics_events ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")
        .bind(&[JsValue::from(limit as i64), JsValue::from(offset as i64)])?
        .all().await?;
    let rows = result.results::<serde_json::Value>()?;

    let next_offset = offset + limit;
    let has_more = (next_offset as i64) < total;
    let next_cursor = if has_more { serde_json::json!(next_offset) } else { serde_json::Value::Null };

    json_response(200, &serde_json::json!({"status":"ok","count":rows.len(),"events":rows,"limit":limit,"cursor":offset,"next_cursor":next_cursor,"total":total}))
}

async fn summary(req: Request, env: &Env) -> Result<Response> {
    let _user = require_auth(&req, &env).await?;
    let db = env.d1("D1")?;
    let total = db.prepare("SELECT COUNT(*) as count FROM analytics_events")
        .first::<i64>(Some("count")).await?.unwrap_or(0);
    let by_type = db.prepare("SELECT event_type, COUNT(*) as count FROM analytics_events GROUP BY event_type ORDER BY count DESC")
        .all().await?;
    let type_rows = by_type.results::<serde_json::Value>()?;
    json_response(200, &serde_json::json!({"status":"ok","total_events":total,"by_type":type_rows}))
}

fn json_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;
    let resp = resp.with_status(status);
    resp.headers().set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}
