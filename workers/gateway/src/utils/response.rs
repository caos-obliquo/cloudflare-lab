//! CORS-wrapped JSON response builder.

use worker::*;

// JSON HTTP response with CORS headers. Used by all gateway handlers.
// CORS * allows any origin - restrict to known domains in production.
pub fn json_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;

    let headers = Headers::new();
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")?;
    headers.set("Access-Control-Allow-Headers", "Content-Type, Authorization")?;

    Ok(resp.with_status(status).with_headers(headers))
}
