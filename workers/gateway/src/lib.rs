//! Gateway Worker — central router, observability hub, SigV4 Lambda proxy.

mod aws_sigv4;
mod handlers;
mod routes;
mod utils;

use std::sync::OnceLock;

use cloudflare_shared::observability::{
    metrics::MetricsRegistry,
    structured_log::{LogBuffer, Logger},
};
use wasm_bindgen::JsValue;
use worker::*;

pub fn logger() -> &'static Logger {
    static LOGGER: OnceLock<Logger> = OnceLock::new();
    LOGGER.get_or_init(|| Logger::new("gateway"))
}

pub fn metrics() -> &'static MetricsRegistry {
    static METRICS: OnceLock<MetricsRegistry> = OnceLock::new();
    METRICS.get_or_init(MetricsRegistry::new)
}

pub fn log_buffer() -> &'static LogBuffer {
    static BUF: OnceLock<LogBuffer> = OnceLock::new();
    BUF.get_or_init(|| LogBuffer::new(100))
}

// HTTP fetch handler -> routes::Router::route() for hand-rolled dispatch.
#[event(fetch)]
async fn fetch(req: Request, env: Env, ctx: Context) -> Result<Response> {
    routes::Router::route(req, env, ctx).await
}

// Queue consumer. Per-message ack/retry. Malformed JSON ack'd and dropped.
// TODO: wire dead-letter queue for poison messages after N retries.
#[event(queue)]
async fn queue(message_batch: MessageBatch<String>, env: Env, _ctx: Context) -> Result<()> {
    let messages = message_batch.messages()?;
    logger()
        .info(&format!("Received {} messages from queue", messages.len()))
        .emit();
    let db = env.d1("D1")?;

    for msg in messages {
        let body: String = msg.body().to_string();
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(val) => {
                let event_type = val.get("event_type").and_then(|v| v.as_str()).unwrap_or("queue_event");
                match db
                    .prepare("INSERT INTO analytics_events (event_type, event_data) VALUES (?1, ?2)")
                    .bind(&[JsValue::from(event_type), JsValue::from(&body)])?
                    .run()
                    .await
                {
                    Ok(_) => {
                        logger()
                            .info(&format!("Processed queue message: {}", event_type))
                            .emit();
                        msg.ack();
                    }
                    Err(e) => {
                        logger().error(&format!("Queue insert error: {:?}", e)).emit();
                        msg.retry();
                    }
                }
            }
            Err(e) => {
                logger()
                    .warn(&format!("Queue parse error: {:?}, body: {}", e, body))
                    .emit();
                msg.ack();
            }
        }
    }
    Ok(())
}
