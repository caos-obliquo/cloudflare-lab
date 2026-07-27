//! JSON response helpers with CORS headers.

use serde_json::json;
use worker::*;

// JSON error response: {"status":"error","error":"<msg>","code":<status>,"request_id":"<id>"}
// CORS wildcard: no request context available in these helpers.
// TODO: make origin configurable (e.g., via thread-local or parameter) for production.
// Callers with request access should override with reflected origin after calling.
pub fn json_error_response(status: u16, message: &str, _request_id: &str) -> Result<Response> {
    let resp = Response::from_json(&json!({
        "status": "error",
        "error": message,
        "code": status,
        "request_id": _request_id,
    }))?;
    let resp = resp.with_status(status);
    // SAFE: Fallback wildcard — callers may override with specific origin.
    resp.headers().set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}

// JSON success response with CORS. Status allows 200, 201, etc.
// Uses wildcard origin (no request context). Override at route level if possible.
pub fn json_ok_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;
    let resp = resp.with_status(status);
    // SAFE: Fallback wildcard — callers may override with specific origin.
    resp.headers().set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}
