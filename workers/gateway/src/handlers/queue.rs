use cloudflare_shared::bindings::EnvBindings;
use worker::*;

use crate::utils::response::json_response;

// GET /queue — send message to Queues (consumed by #[event(queue)] in lib.rs)
pub async fn handler(env: &Env) -> Result<Response> {
    let bindings = EnvBindings::from_env(env)?;

    bindings.queue.send("hello from rust worker").await?;

    json_response(
        200,
        &serde_json::json!({
            "status": "ok",
            "endpoint": "/queue",
            "message": "Message sent to queue!"
        }),
    )
}
