use cloudflare_shared::bindings::EnvBindings;
use worker::*;

use crate::utils::response::json_response;

// GET /d1 — D1 SELECT 1 ping
pub async fn handler(env: &Env) -> Result<Response> {
    let bindings = EnvBindings::from_env(env)?;

    let stmt = bindings.d1.prepare("SELECT 1 AS result");
    let result = stmt.first::<i64>(Some("result")).await?;

    json_response(
        200,
        &serde_json::json!({
            "status": "ok",
            "endpoint": "/d1",
            "result": result
        }),
    )
}
