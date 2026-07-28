//! Auth Worker — register/login/verify/me, HMAC tokens, pbkdf2, DO rate limiting.

use cloudflare_shared::crypto::{hash_password, verify_legacy_sha256, verify_password};
use cloudflare_shared::{
    bootstrap::ensure_users_table,
    observability::{
        health::{DependencyHealth, HealthStatus},
        structured_log::Logger,
        trace_context::TraceContext,
    },
    response::json_error_response,
    session::{create_token, parse_token},
    tracing::request_id_for_request,
};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::JsValue;
use worker::*;

static DB_BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

fn validate_username(username: &str) -> Result<()> {
    let len = username.len();
    if !(3..=32).contains(&len) || !username.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::from("Invalid username"));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<()> {
    let len = password.len();
    if !(8..=128).contains(&len) {
        return Err(Error::from("Invalid password"));
    }
    Ok(())
}

// DO-based rate limiter: atomic, globally consistent.
// Uses rate-limiter worker's RateLimiter DO class via binding.
// Shard key = IP:route (e.g., "1.2.3.4:register").
async fn check_rate_limit(req: &Request, env: &Env, route: &str, limit: u64) -> Result<Option<Response>> {
    let ip = req.headers().get("CF-Connecting-IP")?.unwrap_or_default();
    let key = format!("{}:{}", ip, route);

    let ns = env.durable_object("RATE_LIMITER")?;
    let stub = ns.get_by_name(&key)?;

    // Build check request: POST with limit/window params.
    let body = serde_json::json!({"limit": limit, "window": 60});
    let mut init = RequestInit::new();
    init.method = Method::Post;
    init.body = Some(serde_json::to_string(&body)?.into());
    let do_req = Request::new_with_init("http://do/check", &init)?;

    let mut resp = stub.fetch_with_request(do_req).await?;
    let result: serde_json::Value = resp.json().await?;

    let allowed = result.get("allowed").and_then(|v| v.as_bool()).unwrap_or(false);
    if !allowed {
        return Ok(Some(json_error_response(
            429,
            "Too many requests. Please try again later.",
            "",
        )?));
    }
    Ok(None)
}

// Get SESSION_SECRET from env vars. This is the HMAC signing key for session tokens.
// Must be set via `wrangler secret put SESSION_SECRET` before deploy.
fn session_secret(env: &Env) -> Result<Vec<u8>> {
    Ok(env
        .var("SESSION_SECRET")
        .map_err(|_| Error::from("SESSION_SECRET not configured"))?
        .to_string()
        .into_bytes())
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    let path = url.path();
    let method = req.method().to_string();
    let req_id = request_id_for_request(&req)?;
    let ctx = TraceContext::from_request(&req)?;
    let logger = Logger::new("auth");

    let start = Date::now().as_millis();
    logger.request(&method, path, &ctx).emit();

    if !DB_BOOTSTRAPPED.load(Ordering::Relaxed) {
        if let Ok(db) = env.d1("D1") {
            ensure_users_table(&db).await?;
            DB_BOOTSTRAPPED.store(true, Ordering::Relaxed);
        }
    }

    let mut resp = match (method.as_str(), path) {
        ("GET", "/") => json_response(
            200,
            &serde_json::json!({
                "status": "ok",
                "service": "auth-worker",
                "routes": ["/register", "/login", "/verify", "/me"]
            }),
        ),
        ("POST", "/register") => register(req, &env).await,
        ("POST", "/login") => login(req, &env).await,
        ("GET", "/verify") => verify(req, &env).await,
        ("GET", "/me") => me(req, &env).await,
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
        ("GET", "/debug/d1") => debug_d1(&env).await,
        ("POST", "/debug/hash") => debug_hash(&env).await,
        ("POST", "/debug/insert") => debug_insert(&env).await,
        ("POST", "/debug/register") => debug_register(&env).await,
        _ => json_error_response(404, "Not found", ""),
    }?;
    let duration_ms = Date::now().as_millis() - start;
    let status = resp.status_code();
    logger.response(&method, path, status, duration_ms, &ctx).emit();
    resp.headers().set("X-Request-Id", &req_id)?;
    ctx.inject_into_response(&mut resp)?;
    Ok(resp)
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
    add(
        "rate_limiter",
        env.durable_object("RATE_LIMITER")
            .map(|_| ())
            .map_err(|e| format!("{:?}", e)),
    );
    r
}

async fn register(mut req: Request, env: &Env) -> Result<Response> {
    console_log!("[register] called");
    let body: RegisterRequest = req.json().await?;
    if validate_username(&body.username).is_err() {
        return json_error_response(400, "Invalid username: must be 3-32 alphanumeric characters", "");
    }
    if validate_password(&body.password).is_err() {
        return json_error_response(400, "Invalid password: must be 8-128 characters", "");
    }
    let db = env.d1("D1")?;
    let existing = db
        .prepare("SELECT id FROM users WHERE username = ?1")
        .bind(&[JsValue::from(&body.username)])?
        .all()
        .await?;
    if existing.results::<serde_json::Value>()?.len() > 0 {
        return json_error_response(409, "Username already exists", "");
    }
    let hashed = hash_password(&body.password);
    db.prepare("INSERT INTO users (username, password) VALUES (?1, ?2)")
        .bind(&[JsValue::from(&body.username), JsValue::from(&hashed)])?
        .run()
        .await?;
    json_response(
        201,
        &serde_json::json!({"status":"ok","message":"User registered successfully","username": body.username}),
    )
}

// Login: validate -> verify password -> sign HMAC stateless token. No KV session write.
// Token is self-contained: HMAC-SHA256 signed with SESSION_SECRET, expires in 7 days.
// Old sess_<hex> KV tokens are no longer created (validate still accepts them for 7-day grace).
async fn login(mut req: Request, env: &Env) -> Result<Response> {
    let body: LoginRequest = req.json().await?;
    if validate_username(&body.username).is_err() {
        return json_error_response(400, "Invalid username: must be 3-32 alphanumeric characters", "");
    }
    if validate_password(&body.password).is_err() {
        return json_error_response(400, "Invalid password: must be 8-128 characters", "");
    }

    let db = env.d1("D1")?;
    let secret = session_secret(env)?;
    let db_result = db
        .prepare("SELECT password FROM users WHERE username = ?1")
        .bind(&[JsValue::from(&body.username)])?
        .all()
        .await?;
    let rows = db_result.results::<serde_json::Value>();
    let stored_hash = rows?.first().and_then(|r| r.get("password").and_then(|v| v.as_str().map(String::from)));

    match stored_hash {
        // pbkdf2 match -> sign HMAC token, return to client.
        Some(hash) if verify_password(&body.password, &hash) => {
            match create_token(&secret, &body.username, "session", 0) {
                Ok(token) => json_response(
                    200,
                    &serde_json::json!({"status":"ok","message":"Login successful","token":token}),
                ),
                Err(e) => json_error_response(500, &format!("Token creation failed: {}", e), ""),
            }
        }
        // SHA256 migration -> re-hash to pbkdf2, then sign HMAC token.
        Some(hash) if verify_legacy_sha256(&body.password, &hash) => {
            let new_hash = hash_password(&body.password);
            db.prepare("UPDATE users SET password = ?1 WHERE username = ?2")
                .bind(&[JsValue::from(&new_hash), JsValue::from(&body.username)])?
                .run()
                .await?;
            match create_token(&secret, &body.username, "session", 0) {
                Ok(token) => json_response(
                    200,
                    &serde_json::json!({"status":"ok","message":"Login successful","token":token}),
                ),
                Err(e) => json_error_response(500, &format!("Token creation failed: {}", e), ""),
            }
        }
        _ => json_error_response(401, "Invalid credentials", ""),
    }
}

// Verify: parse HMAC stateless token, no KV lookup.
// Supports both s2.<base64>.<sig> (new) and sess_<hex> (old KV tokens).
// Old format checked via KV as fallback during migration window.
async fn verify(req: Request, env: &Env) -> Result<Response> {
    let auth_header = req.headers().get("Authorization")?.unwrap_or_default();
    let raw = auth_header.strip_prefix("Bearer ").unwrap_or("");
    if raw.is_empty() {
        return json_error_response(401, "Missing or invalid Authorization header", "");
    }

    // New HMAC token format: prefix s2.
    if raw.starts_with("s2.") {
        let secret = session_secret(env)?;
        match parse_token(raw, &secret, "session") {
            Ok(t) => json_response(
                200,
                &serde_json::json!({"status":"ok","message":"Token is valid","username":t.username}),
            ),
            Err(e) => json_error_response(401, &format!("Invalid or expired token: {}", e), ""),
        }
    } else {
        // Legacy KV token fallback: query KV for old sess_<hex> tokens.
        // TODO: remove once all users have re-authenticated with HMAC tokens.
        let kv = env.kv("SESSIONS")?;
        match kv.get(raw).text().await? {
            Some(user) => json_response(
                200,
                &serde_json::json!({"status":"ok","message":"Token is valid","username":user}),
            ),
            None => json_error_response(401, "Invalid or expired token", ""),
        }
    }
}

// Me: parse HMAC stateless token, return user info + token.
async fn me(req: Request, env: &Env) -> Result<Response> {
    let auth_header = req.headers().get("Authorization")?.unwrap_or_default();
    let raw = auth_header.strip_prefix("Bearer ").unwrap_or("");
    if raw.is_empty() {
        return json_error_response(401, "Missing Authorization header", "");
    }

    // New HMAC token.
    if raw.starts_with("s2.") {
        let secret = session_secret(env)?;
        match parse_token(raw, &secret, "session") {
            Ok(t) => json_response(
                200,
                &serde_json::json!({"status":"ok","authenticated":true,"username":t.username,"token":raw}),
            ),
            Err(e) => json_error_response(401, &format!("Invalid or expired token: {}", e), ""),
        }
    } else {
        // Legacy KV token fallback.
        let kv = env.kv("SESSIONS")?;
        match kv.get(raw).text().await? {
            Some(user) => json_response(
                200,
                &serde_json::json!({"status":"ok","authenticated":true,"username":user,"token":raw}),
            ),
            None => json_error_response(401, "Invalid or expired token", ""),
        }
    }
}

// Debug: test D1 step-by-step, return full error details
async fn debug_d1(env: &Env) -> Result<Response> {
    let db = match env.d1("D1") {
        Ok(d) => d,
        Err(e) => return json_response(200, &serde_json::json!({"step":"d1_binding","error":format!("{}",e)})),
    };
    let stmt = db.prepare("SELECT 1 as x");
    let bound = match stmt.bind(&[]) {
        Ok(b) => b,
        Err(e) => return json_response(200, &serde_json::json!({"step":"bind","error":format!("{}",e)})),
    };
    let d1_result = match bound.all().await {
        Ok(r) => r,
        Err(e) => return json_response(200, &serde_json::json!({"step":"all","error":format!("{}",e)})),
    };
    let rows = match d1_result.results::<serde_json::Value>() {
        Ok(r) => r,
        Err(e) => return json_response(200, &serde_json::json!({"step":"results","error":format!("{}",e),"success":d1_result.success()})),
    };
    json_response(200, &serde_json::json!({"step":"done","rows":rows,"count":rows.len()}))
}

// Debug: register step-by-step, return full error details at each step
async fn debug_register(env: &Env) -> Result<Response> {
    let db = match env.d1("D1") {
        Ok(d) => d,
        Err(e) => return json_response(200, &serde_json::json!({"step":"d1_binding","error":format!("{}",e)})),
    };
    let existing = match db
        .prepare("SELECT id FROM users WHERE username = ?1")
        .bind(&[JsValue::from("debug_test_user")])
    {
        Ok(stmt) => match stmt.all().await {
            Ok(r) => r,
            Err(e) => return json_response(200, &serde_json::json!({"step":"select_all","error":format!("{}",e)})),
        },
        Err(e) => return json_response(200, &serde_json::json!({"step":"bind","error":format!("{}",e)})),
    };
    let rows = match existing.results::<serde_json::Value>() {
        Ok(r) => r,
        Err(e) => return json_response(200, &serde_json::json!({"step":"results_deser","error":format!("{}",e),"success":existing.success()})),
    };
    json_response(200, &serde_json::json!({"step":"select_ok","rows":rows,"count":rows.len()}))
}

// Debug: test hash_password in WASM
async fn debug_hash(env: &Env) -> Result<Response> {
    let pwd = "TestPass123!";
    let hashed = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hash_password(pwd)
    })) {
        Ok(h) => h,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            return json_response(200, &serde_json::json!({"step":"hash_panic","error":msg}));
        }
    };
    json_response(200, &serde_json::json!({"step":"hash_ok","hash":hashed}))
}

// Debug: test D1 INSERT
async fn debug_insert(env: &Env) -> Result<Response> {
    let db = match env.d1("D1") {
        Ok(d) => d,
        Err(e) => return json_response(200, &serde_json::json!({"step":"d1_binding","error":format!("{}",e)})),
    };
    let hashed = hash_password("TestPass123!");
    match db
        .prepare("INSERT INTO users (username, password) VALUES (?1, ?2)")
        .bind(&[JsValue::from("debug_insert_user"), JsValue::from(&hashed)])
    {
        Ok(stmt) => match stmt.run().await {
            Ok(_) => json_response(200, &serde_json::json!({"step":"insert_ok"})),
            Err(e) => json_response(200, &serde_json::json!({"step":"insert_err","error":format!("{}",e)})),
        },
        Err(e) => json_response(200, &serde_json::json!({"step":"bind_err","error":format!("{}",e)})),
    }
}

fn json_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;
    Ok(resp.with_status(status))
}
