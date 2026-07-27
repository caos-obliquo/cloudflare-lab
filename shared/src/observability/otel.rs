//! OTLP span exporter — sends spans to SigNoz OTel collector via HTTP/protobuf.
//!
//! The collector's HTTP endpoint (`:4318/v1/traces`) only accepts
//! `application/x-protobuf`. This module uses the `prost` crate to encode
//! the OTLP `ExportTraceServiceRequest` as protobuf binary.

use crate::observability::trace_context::TraceContext;
use prost::Message;

use super::otlp_proto::{
    kv_int, kv_str, ExportTraceServiceRequest, Resource, ResourceSpans, Scope, ScopeSpans,
    Span, Status,
};

#[allow(clippy::too_many_arguments)]
/// Export a single span to the OTel collector via HTTP/protobuf.
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

    // POST to collector.
    let url = format!("{}/v1/traces", collector_url.trim_end_matches('/'));

    let mut init = worker::RequestInit::new();
    init.method = worker::Method::Post;
    init.headers = {
        let h = worker::Headers::new();
        h.set("Content-Type", "application/x-protobuf").ok();
        h
    };

    // Convert Vec<u8> to JsValue via js_sys::Uint8Array.
    let js_array = js_sys::Uint8Array::from(&proto_bytes[..]);
    init.body = Some(wasm_bindgen::JsValue::from(js_array));

    match worker::Fetch::Request(
        worker::Request::new_with_init(&url, &init).map_err(|e| format!("build req: {:?}", e))?
    )
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

/// Decode a hex string into raw bytes.
fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::otlp_proto::any_value;
    use prost::Message;

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
