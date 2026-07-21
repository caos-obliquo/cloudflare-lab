use crate::utils::response::json_response;
use cloudflare_shared::bindings::EnvBindings;
use worker::*;

// GET /kv: puts then gets a KV value. Demonstrates Workers KV read/write.
// KV = global key-value store, eventually consistent (seconds for replication).
// Best for config, session data, cached responses.
pub async fn handler(env: &Env) -> Result<Response> {
    let bindings = EnvBindings::from_env(env)?;

    bindings
        .kv
        .put("greeting", "Hello from Rust KV!")?
        .execute()
        .await?;

    let val = bindings.kv.get("greeting").text().await?;

    json_response(
        200,
        &serde_json::json!({
            "status": "ok",
            "endpoint": "/kv",
            "value": val
        }),
    )
}
