use crate::utils::response::json_response;
use cloudflare_shared::bindings::EnvBindings;
use worker::*;

// GET /queue: sends a message to Cloudflare Queues.
// Consumer (lib.rs #[event(queue)]) receives it async and inserts into D1.
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
