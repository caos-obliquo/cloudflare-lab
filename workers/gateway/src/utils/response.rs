//! JSON response builder (no CORS — handled at route level in `routes.rs`).

use worker::*;

// JSON HTTP response. CORS headers are applied by the router in `routes.rs`
// where the request `Origin` header is available for safe reflection.
pub fn json_response(status: u16, data: &serde_json::Value) -> Result<Response> {
    let resp = Response::from_json(data)?;
    Ok(resp.with_status(status))
}
