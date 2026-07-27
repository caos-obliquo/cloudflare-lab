//! OTLP span exporter — sends spans to SigNoz OTel collector via HTTP/protobuf.
//!
//! The collector's HTTP endpoint (`:4318/v1/traces`) only accepts
//! `application/x-protobuf`. This module uses the `prost` crate to encode
//! the OTLP `ExportTraceServiceRequest` as protobuf binary.

use std::{collections::VecDeque, sync::Mutex};

use prost::Message;
use wasm_bindgen::JsCast;

use super::otlp_proto::{
    kv_int, kv_str, ExportTraceServiceRequest, Resource, ResourceSpans, Scope, ScopeSpans, Span, Status,
};
use crate::observability::trace_context::TraceContext;

const MAX_BUFFERED_SPANS: usize = 100;

struct PendingSpan {
    url: String,
    body: Vec<u8>,
}

static SPAN_BUFFER: Mutex<VecDeque<PendingSpan>> = Mutex::new(VecDeque::new());

#[allow(clippy::too_many_arguments)]
/// Export a single span to the OTel collector via HTTP/protobuf.
///
/// Retries up to 3 times with exponential backoff (100ms, 300ms).
/// On success, also flushes any previously buffered spans.
/// On failure, buffers the span for retry on the next export.
pub async fn export_span(
    collector_url: &str,
    service: &str,
    tc: &TraceContext,
    parent_span_id: Option<&str>,
    name: &str,
    start_ms: f64,
    end_ms: f64,
    method: &str,
    path: &str,
    status: u16,
    error: Option<&str>,
) -> Result<(), String> {
    // Decode hex trace_id (32 hex chars → 16 bytes) and span_id (16 hex → 8 bytes).
    let trace_id = hex_to_bytes(&tc.trace_id);
    let span_id = hex_to_bytes(&tc.span_id);
    let parent = parent_span_id.map(hex_to_bytes);
    if trace_id.len() != 16 {
        return Err(format!("trace_id must be 16 bytes, got {}", trace_id.len()));
    }
    if span_id.len() != 8 {
        return Err(format!("span_id must be 8 bytes, got {}", span_id.len()));
    }

    let start_ns = (start_ms * 1_000_000.0) as u64;
    let end_ns = (end_ms * 1_000_000.0) as u64;

    // Build attributes list.
    let mut attrs = vec![
        kv_str("http.method", method),
        kv_str("http.target", path),
        kv_int("http.status_code", status as i64),
        kv_str("http.route", path),
    ];
    if let Some(err) = error {
        attrs.push(kv_str("error.message", err));
    }

    // Build the OTLP status.
    let otel_status = match error {
        Some(err_msg) => Status {
            code: 2, // Error
            message: err_msg.to_string(),
        },
        None => Status {
            code: 1, // Ok
            message: String::new(),
        },
    };

    // Construct the Span protobuf message.
    let span = Span {
        trace_id,
        span_id,
        trace_state: String::new(),
        parent_span_id: parent.unwrap_or_default(),
        name: name.to_string(),
        kind: 2, // SpanKind::Server
        start_time_unix_nano: start_ns,
        end_time_unix_nano: end_ns,
        attributes: attrs,
        dropped_attributes_count: 0,
        status: Some(otel_status),
    };

    // Build the full ExportTraceServiceRequest.
    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![
                    kv_str("service.name", service),
                    kv_str("telemetry.sdk.name", "cloudflare-lab"),
                    kv_str("telemetry.sdk.language", "rust"),
                ],
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "cloudflare-lab".to_string(),
                    version: "0.1.0".to_string(),
                }),
                spans: vec![span],
            }],
        }],
    };

    // Encode as protobuf binary.
    let proto_bytes = request.encode_to_vec();

    // POST to collector with retry.
    let url = format!("{}/v1/traces", collector_url.trim_end_matches('/'));

    match send_with_retry(&url, &proto_bytes).await {
        Ok(()) => {
            // On success, flush any previously buffered spans.
            flush_buffer().await;
            Ok(())
        }
        Err(e) => {
            // Buffer the failed span for retry on the next export.
            if let Ok(mut buf) = SPAN_BUFFER.lock() {
                if buf.len() >= MAX_BUFFERED_SPANS {
                    buf.pop_front();
                }
                buf.push_back(PendingSpan { url, body: proto_bytes });
            }
            Err(e)
        }
    }
}

/// Decode a hex string into raw bytes.
fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Single HTTP POST of protobuf-encoded span data to the collector.
async fn send_post(url: &str, body: &[u8]) -> Result<(), String> {
    let mut init = worker::RequestInit::new();
    init.method = worker::Method::Post;
    init.headers = {
        let h = worker::Headers::new();
        h.set("Content-Type", "application/x-protobuf").ok();
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
                Err(format!("otel collector returned {}", code))
            }
        }
        Err(e) => Err(format!("otel post: {:?}", e)),
    }
}

/// POST with retry: up to 3 attempts, exponential backoff (100ms, 300ms).
async fn send_with_retry(url: &str, body: &[u8]) -> Result<(), String> {
    let backoffs = [0u64, 100, 300];
    let mut last_err = String::from("all 3 attempts failed");

    for (i, delay) in backoffs.iter().enumerate() {
        if *delay > 0 {
            sleep_ms(*delay).await;
        }

        match send_post(url, body).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                worker::console_log!("otel export attempt {}/3 failed: {}", i + 1, e);
                last_err = e;
            }
        }
    }

    Err(last_err)
}

/// Flush all buffered spans, best-effort (drops on individual failure).
async fn flush_buffer() {
    let spans: Vec<PendingSpan> = {
        let mut buf = match SPAN_BUFFER.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        buf.drain(..).collect()
    };

    for span in spans {
        if let Err(e) = send_post(&span.url, &span.body).await {
            worker::console_log!("otel flush error: {}", e);
        }
    }
}

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

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::*;
    use crate::observability::otlp_proto::any_value;

    #[test]
    fn test_hex_to_bytes() {
        let bytes = hex_to_bytes("00000000000000000000000000000000");
        assert_eq!(bytes.len(), 16);
        assert_eq!(bytes, vec![0u8; 16]);

        let bytes = hex_to_bytes("ff00ff00ff00ff00");
        assert_eq!(bytes, vec![0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0x00]);
    }

    #[test]
    fn test_kv_str() {
        let kv = kv_str("test.key", "test.val");
        assert_eq!(kv.key, "test.key");
        match kv.value.unwrap().value.unwrap() {
            any_value::Value::StringValue(s) => assert_eq!(s, "test.val"),
            _ => panic!("expected string value"),
        }
    }

    #[test]
    fn test_kv_int() {
        let kv = kv_int("status", 200);
        assert_eq!(kv.key, "status");
        match kv.value.unwrap().value.unwrap() {
            any_value::Value::IntValue(n) => assert_eq!(n, 200),
            _ => panic!("expected int value"),
        }
    }

    #[test]
    fn test_proto_roundtrip() {
        // Build a minimal request and verify encoding doesn't crash.
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource { attributes: vec![] }),
                scope_spans: vec![ScopeSpans {
                    scope: Some(Scope {
                        name: "test".into(),
                        version: "1.0".into(),
                    }),
                    spans: vec![Span {
                        trace_id: vec![0u8; 16],
                        span_id: vec![0u8; 8],
                        trace_state: String::new(),
                        parent_span_id: vec![],
                        name: "test-span".into(),
                        kind: 2, // SpanKind::Server
                        start_time_unix_nano: 1_000_000_000,
                        end_time_unix_nano: 2_000_000_000,
                        attributes: vec![],
                        dropped_attributes_count: 0,
                        status: Some(Status {
                            code: 1,
                            message: String::new(),
                        }),
                    }],
                }],
            }],
        };
        let encoded = req.encode_to_vec();
        assert!(!encoded.is_empty(), "encoded protobuf should not be empty");
        // Verify it starts with a valid protobuf field tag
        // Field 1 (resource_spans), wire type 2 (length-delimited) = 0x0a
        assert_eq!(encoded[0], 0x0a, "first byte should be field 1 tag");
    }
}
