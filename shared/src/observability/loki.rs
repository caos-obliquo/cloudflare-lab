//! Loki log exporter — sends structured logs to Loki via HTTP JSON API.
//!
//! Batches buffered log events and POSTs them as JSON to Loki's
//! `/loki/api/v1/push` endpoint. Non-blocking via `cx.wait_until()`.
//! Follows the same retry + buffer pattern as the OTel exporter.
//!
//! # Config
//!
//! - `LOKI_ENDPOINT` — URL of the Loki HTTP endpoint (default: http://loki:3100)
//! - `LOKI_TENANT_ID` — optional Loki tenant ID (`X-Scope-OrgID` header)

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use serde::Serialize;
use wasm_bindgen::JsCast;
use worker::Date;

use crate::observability::structured_log::{LogEvent, LogLevel};

/// Maximum log events to hold in the pre-push buffer.
const MAX_BUFFERED_EVENTS: usize = 100;

/// Maximum failed push payloads to keep for later retry.
const MAX_BUFFERED_PUSHES: usize = 10;

/// Log events waiting to be pushed to Loki.
static EVENT_BUFFER: Mutex<Vec<LogEvent>> = Mutex::new(Vec::new());

/// A failed push payload waiting for retry on the next successful push.
struct PendingPush {
    url: String,
    body: Vec<u8>,
    tenant_id: Option<String>,
}

/// Buffer of failed push payloads.
static FAILED_BUFFER: Mutex<VecDeque<PendingPush>> = Mutex::new(VecDeque::new());

/// A single Loki stream — one label set with multiple timestamped values.
#[derive(Serialize)]
struct LokiStream {
    stream: HashMap<String, String>,
    values: Vec<Vec<String>>,
}

/// The Loki push request body.
#[derive(Serialize)]
struct LokiPayload {
    streams: Vec<LokiStream>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Add a log event to the export buffer.
///
/// Events accumulate here until the next [`push_logs`] call drains them.
/// If the buffer is full, the oldest event is dropped.
pub fn buffer_event(event: LogEvent) {
    if let Ok(mut buf) = EVENT_BUFFER.lock() {
        if buf.len() >= MAX_BUFFERED_EVENTS {
            buf.remove(0);
        }
        buf.push(event);
    }
}

/// Push buffered log events to Loki.
///
/// Drains the event buffer, groups events by service and log level into
/// separate Loki streams, and POSTs the payload. Retries up to 3 times
/// with backoff [`0ms, 100ms, 300ms`]. On success, flushes any previously
/// failed pushes. On failure, buffers the payload for retry on the next call.
///
/// Best-effort and non-blocking — intended for use inside `cx.wait_until()`.
pub async fn push_logs(loki_endpoint: &str, tenant_id: Option<&str>) {
    let events = {
        let mut buf = match EVENT_BUFFER.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if buf.is_empty() {
            return;
        }
        buf.drain(..).collect::<Vec<_>>()
    };

    let payload = build_loki_payload(&events);
    let body = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => {
            worker::console_log!("loki serialize error: {}", e);
            return;
        }
    };

    let url = format!("{}/loki/api/v1/push", loki_endpoint.trim_end_matches('/'));

    match send_with_retry(&url, &body, tenant_id).await {
        Ok(()) => {
            // On success, flush any previously failed pushes.
            flush_failed().await;
        }
        Err(e) => {
            worker::console_log!("loki push failed (will retry later): {}", e);
            buffer_failed(url, body, tenant_id.map(|s| s.to_string()));
        }
    }
}

// ---------------------------------------------------------------------------
// Loki payload construction
// ---------------------------------------------------------------------------

/// Build the Loki push payload from a batch of log events.
///
/// Groups events by `(service, level)` so each unique combination becomes
/// a separate Loki stream with labels `worker` and `level`.
fn build_loki_payload(events: &[LogEvent]) -> LokiPayload {
    // Group events by (service, level_label) into a BTreeMap so output is
    // deterministic (stable stream ordering across serializations).
    let mut grouped: std::collections::BTreeMap<(String, String), Vec<Vec<String>>> = std::collections::BTreeMap::new();

    for event in events {
        let level_label = match event.level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };

        // Nanosecond timestamp from worker Date (ms precision × 1_000_000).
        let ts_ns = Date::now().as_millis() * 1_000_000;
        let ts_str = ts_ns.to_string();

        // Serialize the full LogEvent as the log line value.
        let log_json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());

        grouped
            .entry((event.service.clone(), level_label.to_string()))
            .or_default()
            .push(vec![ts_str, log_json]);
    }

    let streams = grouped
        .into_iter()
        .map(|((service, level), values)| {
            let mut stream = HashMap::new();
            stream.insert("worker".to_string(), service);
            stream.insert("level".to_string(), level);
            LokiStream { stream, values }
        })
        .collect();

    LokiPayload { streams }
}

// ---------------------------------------------------------------------------
// HTTP transport
// ---------------------------------------------------------------------------

/// Single HTTP POST of JSON-encoded payload to the Loki endpoint.
async fn send_post(url: &str, body: &[u8], tenant_id: Option<&str>) -> Result<(), String> {
    let mut init = worker::RequestInit::new();
    init.method = worker::Method::Post;
    init.headers = {
        let h = worker::Headers::new();
        h.set("Content-Type", "application/json").ok();
        if let Some(tid) = tenant_id {
            h.set("X-Scope-OrgID", tid).ok();
        }
        h
    };

    let js_array = js_sys::Uint8Array::from(body);
    init.body = Some(wasm_bindgen::JsValue::from(js_array));

    match worker::Fetch::Request(worker::Request::new_with_init(url, &init).map_err(|e| format!("build req: {:?}", e))?)
        .send()
        .await
    {
        Ok(r) => {
            let code = r.status_code();
            if (200..300).contains(&code) {
                Ok(())
            } else {
                Err(format!("loki returned {}", code))
            }
        }
        Err(e) => Err(format!("loki post: {:?}", e)),
    }
}

/// POST with retry: up to 3 attempts, backoff [`0ms`, `100ms`, `300ms`].
async fn send_with_retry(url: &str, body: &[u8], tenant_id: Option<&str>) -> Result<(), String> {
    let backoffs = [0u64, 100, 300];
    let mut last_err = String::from("all 3 attempts failed");

    for (i, delay) in backoffs.iter().enumerate() {
        if *delay > 0 {
            sleep_ms(*delay).await;
        }

        match send_post(url, body, tenant_id).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                worker::console_log!("loki push attempt {}/3 failed: {}", i + 1, e);
                last_err = e;
            }
        }
    }

    Err(last_err)
}

// ---------------------------------------------------------------------------
// Failed-push buffer
// ---------------------------------------------------------------------------

/// Flush all failed pushes, best-effort (silently drops on individual failure).
async fn flush_failed() {
    let pushes: Vec<PendingPush> = {
        let mut buf = match FAILED_BUFFER.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        buf.drain(..).collect()
    };

    for push in pushes {
        if let Err(e) = send_post(&push.url, &push.body, push.tenant_id.as_deref()).await {
            worker::console_log!("loki flush error: {}", e);
        }
    }
}

/// Buffer a failed push payload for retry on the next successful push.
fn buffer_failed(url: String, body: Vec<u8>, tenant_id: Option<String>) {
    if let Ok(mut buf) = FAILED_BUFFER.lock() {
        if buf.len() >= MAX_BUFFERED_PUSHES {
            buf.pop_front();
        }
        buf.push_back(PendingPush { url, body, tenant_id });
    }
}

// ---------------------------------------------------------------------------
// Async sleep (JS setTimeout bridge)
// ---------------------------------------------------------------------------

/// Async delay using JS setTimeout. Yields to the event loop while waiting.
async fn sleep_ms(ms: u64) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let global = js_sys::global();
        if let Ok(set_timeout) = js_sys::Reflect::get(&global, &wasm_bindgen::JsValue::from("setTimeout")) {
            if let Some(f) = set_timeout.dyn_ref::<js_sys::Function>() {
                let _ = f.call2(
                    &wasm_bindgen::JsValue::null(),
                    &resolve,
                    &wasm_bindgen::JsValue::from(ms as f64),
                );
            }
        }
    });
    let _ = worker::wasm_bindgen_futures::JsFuture::from(promise).await;
}
