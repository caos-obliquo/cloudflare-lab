use worker::*;
use serde_json::json;

// JSON error response: {"status":"error","error":"<msg>","code":<status>,"request_id":"<id>"}
// CORS * on all responses. Production: restrict to known origins.
pub fn json_error_response(status: u16, message: &str, _request_id: &str) -> Result<Response> {
    let resp = Response::from_json(&json!({
        "status": "error",
        "error": message,
        "code": status,
        "request_id": _request_id,
    }))?;
    let resp = resp.with_status(status);
    resp.headers().set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}

// JSON success response with CORS. Status allows 200, 201, etc.
pub fn json_ok_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;
    let resp = resp.with_status(status);
    resp.headers().set("Access-Control-Allow-Origin", "*")?;
    Ok(resp)
}
