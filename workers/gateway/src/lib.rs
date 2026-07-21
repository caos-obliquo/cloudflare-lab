mod aws_sigv4;
mod handlers;
mod routes;
mod utils;

use wasm_bindgen::JsValue;
use worker::*;

// HTTP fetch handler -> routes::Router::route() for hand-rolled dispatch.
#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    routes::Router::route(req, env).await
}

// Queue consumer. Per-message ack/retry. Malformed JSON ack'd and dropped (no dead-letter yet).
#[event(queue)]
async fn queue(message_batch: MessageBatch<String>, env: Env, _ctx: Context) -> Result<()> {
    let messages = message_batch.messages()?;
    console_log!("Received {} messages from queue", messages.len());
    let db = env.d1("D1")?;

    for msg in messages {
        let body: String = msg.body().to_string();
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(val) => {
                let event_type = val.get("event_type").and_then(|v| v.as_str()).unwrap_or("queue_event");
                match db.prepare("INSERT INTO analytics_events (event_type, event_data) VALUES (?1, ?2)")
                    .bind(&[JsValue::from(event_type), JsValue::from(&body)])?
                    .run().await
                {
                    Ok(_) => { console_log!("[queue] processed message: {}", event_type); msg.ack(); }
                    Err(e) => { console_log!("[queue] ERROR inserting message: {:?}", e); msg.retry(); }
                }
            }
            Err(e) => { console_log!("[queue] ERROR parsing message: {:?}, body: {}", e, body); msg.ack(); }
        }
    }
    Ok(())
}
