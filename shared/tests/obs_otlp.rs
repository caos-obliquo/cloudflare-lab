// Integration tests for OTLP protobuf encoding/decoding.
// Pure protobuf roundtrip — no WASM dependencies, no network calls.

use cloudflare_shared::observability::otlp_proto::{
    any_value, kv_int, kv_str, ExportTraceServiceRequest, Resource, ResourceSpans, Scope, ScopeSpans, Span, Status,
};
use prost::Message;

/// Build a fully populated ExportTraceServiceRequest and verify all 12 span
/// fields survive a protobuf encode→decode roundtrip.
#[test]
fn test_span_full_roundtrip() {
    // Known test data
    let trace_id: Vec<u8> = (0u8..16).collect(); // 0,1,2,...,15 (16 bytes)
    let span_id: Vec<u8> = (0xAAu8..0xAA + 8).collect(); // 8 bytes
    let parent_span_id: Vec<u8> = (0xBBu8..0xBB + 8).collect();
    let name = "test-span".to_string();
    let kind = 2i32; // SpanKind::Server
    let start_ns = 1_700_000_000_000_000_000u64;
    let end_ns = 1_700_000_000_123_456_789u64;
    let status_code = 1i32; // Ok
    let status_msg = String::new();

    // Build attributes with all three value types
    let attrs = vec![
        kv_str("http.method", "GET"),
        kv_str("http.target", "/api"),
        kv_int("http.status_code", 200),
    ];

    let span = Span {
        trace_id: trace_id.clone(),
        span_id: span_id.clone(),
        trace_state: String::new(),
        parent_span_id: parent_span_id.clone(),
        name: name.clone(),
        kind,
        start_time_unix_nano: start_ns,
        end_time_unix_nano: end_ns,
        attributes: attrs,
        dropped_attributes_count: 0,
        status: Some(Status {
            code: status_code,
            message: status_msg.clone(),
        }),
    };

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![
                    kv_str("service.name", "test-service"),
                    kv_str("telemetry.sdk.name", "cloudflare-lab"),
                ],
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "test-scope".to_string(),
                    version: "1.0.0".to_string(),
                }),
                spans: vec![span],
            }],
        }],
    };

    // Encode to protobuf
    let encoded = request.encode_to_vec();
    assert!(!encoded.is_empty(), "encoded bytes must not be empty");

    // Decode back
    let decoded = ExportTraceServiceRequest::decode(&encoded[..]).expect("should decode successfully");

    // Verify structure
    assert_eq!(decoded.resource_spans.len(), 1);

    let rs = &decoded.resource_spans[0];

    // Resource
    let resource = rs.resource.as_ref().expect("resource should be present");
    assert_eq!(resource.attributes.len(), 2);

    // Scope
    assert_eq!(rs.scope_spans.len(), 1);
    let ss = &rs.scope_spans[0];
    let scope = ss.scope.as_ref().expect("scope should be present");
    assert_eq!(scope.name, "test-scope");
    assert_eq!(scope.version, "1.0.0");

    // Span
    assert_eq!(ss.spans.len(), 1);
    let sp = &ss.spans[0];

    // 12 field assertions
    // 1. trace_id: 16 bytes
    assert_eq!(sp.trace_id.len(), 16, "trace_id must be 16 bytes");
    assert_eq!(sp.trace_id, trace_id, "trace_id content");

    // 2. span_id: 8 bytes
    assert_eq!(sp.span_id.len(), 8, "span_id must be 8 bytes");
    assert_eq!(sp.span_id, span_id, "span_id content");

    // 3. name
    assert_eq!(sp.name, "test-span");

    // 4. kind == 2 (SERVER)
    assert_eq!(sp.kind, 2, "kind must be SpanKind::Server");

    // 5. start_time_unix_nano
    assert_eq!(sp.start_time_unix_nano, start_ns);

    // 6. end_time_unix_nano
    assert_eq!(sp.end_time_unix_nano, end_ns);

    // 7-9. attributes (string/int/bool variants)
    assert_eq!(sp.attributes.len(), 3);
    // first attr: http.method = "GET" (string)
    assert_eq!(sp.attributes[0].key, "http.method");
    match &sp.attributes[0].value.as_ref().unwrap().value {
        Some(any_value::Value::StringValue(s)) => assert_eq!(s, "GET"),
        _ => panic!("expected string value for http.method"),
    }
    // second attr: http.target = "/api" (string)
    assert_eq!(sp.attributes[1].key, "http.target");
    match &sp.attributes[1].value.as_ref().unwrap().value {
        Some(any_value::Value::StringValue(s)) => assert_eq!(s, "/api"),
        _ => panic!("expected string value for http.target"),
    }
    // third attr: http.status_code = 200 (int)
    assert_eq!(sp.attributes[2].key, "http.status_code");
    match &sp.attributes[2].value.as_ref().unwrap().value {
        Some(any_value::Value::IntValue(n)) => assert_eq!(*n, 200),
        _ => panic!("expected int value for http.status_code"),
    }

    // 10. parent_span_id
    assert_eq!(sp.parent_span_id, parent_span_id, "parent_span_id");

    // 11-12. status code + message
    let status = sp.status.as_ref().expect("status should be present");
    assert_eq!(status.code, 1, "status code should be OK(1)");
    assert!(status.message.is_empty(), "status message should be empty");
}

#[test]
fn test_span_with_error_status() {
    let span = Span {
        trace_id: vec![0u8; 16],
        span_id: vec![0u8; 8],
        trace_state: String::new(),
        parent_span_id: vec![],
        name: "error-span".into(),
        kind: 2,
        start_time_unix_nano: 0,
        end_time_unix_nano: 0,
        attributes: vec![],
        dropped_attributes_count: 0,
        status: Some(Status {
            code: 2, // Error
            message: "timeout".to_string(),
        }),
    };

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource { attributes: vec![] }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "test".into(),
                    version: "1.0".into(),
                }),
                spans: vec![span],
            }],
        }],
    };

    let encoded = request.encode_to_vec();
    let decoded = ExportTraceServiceRequest::decode(&encoded[..]).unwrap();
    let sp = &decoded.resource_spans[0].scope_spans[0].spans[0];
    let st = sp.status.as_ref().unwrap();
    assert_eq!(st.code, 2, "error status code should be 2");
    assert_eq!(st.message, "timeout");
}

#[test]
fn test_bool_attribute_variant() {
    use cloudflare_shared::observability::otlp_proto::KeyValue;
    // Build a KeyValue with a bool value manually
    let kv = KeyValue {
        key: "debug".to_string(),
        value: Some(cloudflare_shared::observability::otlp_proto::AnyValue {
            value: Some(any_value::Value::BoolValue(true)),
        }),
    };

    let span = Span {
        trace_id: vec![0u8; 16],
        span_id: vec![0u8; 8],
        trace_state: String::new(),
        parent_span_id: vec![],
        name: "bool-test".into(),
        kind: 1,
        start_time_unix_nano: 0,
        end_time_unix_nano: 0,
        attributes: vec![kv],
        dropped_attributes_count: 0,
        status: None,
    };

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource { attributes: vec![] }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "test".into(),
                    version: "1.0".into(),
                }),
                spans: vec![span],
            }],
        }],
    };

    let encoded = request.encode_to_vec();
    let decoded = ExportTraceServiceRequest::decode(&encoded[..]).unwrap();
    let sp = &decoded.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(sp.attributes.len(), 1);
    match &sp.attributes[0].value.as_ref().unwrap().value {
        Some(any_value::Value::BoolValue(b)) => assert!(*b),
        _ => panic!("expected bool value"),
    }
}

#[test]
fn test_double_attribute_variant() {
    use cloudflare_shared::observability::otlp_proto::KeyValue;
    let kv = KeyValue {
        key: "temperature".to_string(),
        value: Some(cloudflare_shared::observability::otlp_proto::AnyValue {
            value: Some(any_value::Value::DoubleValue(36.6)),
        }),
    };

    let span = Span {
        trace_id: vec![0u8; 16],
        span_id: vec![0u8; 8],
        trace_state: String::new(),
        parent_span_id: vec![],
        name: "double-test".into(),
        kind: 1,
        start_time_unix_nano: 0,
        end_time_unix_nano: 0,
        attributes: vec![kv],
        dropped_attributes_count: 0,
        status: None,
    };

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource { attributes: vec![] }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "test".into(),
                    version: "1.0".into(),
                }),
                spans: vec![span],
            }],
        }],
    };

    let encoded = request.encode_to_vec();
    let decoded = ExportTraceServiceRequest::decode(&encoded[..]).unwrap();
    let sp = &decoded.resource_spans[0].scope_spans[0].spans[0];
    match &sp.attributes[0].value.as_ref().unwrap().value {
        Some(any_value::Value::DoubleValue(d)) => assert!((*d - 36.6).abs() < f64::EPSILON),
        _ => panic!("expected double value"),
    }
}

#[test]
fn test_span_root_no_parent() {
    // Root span: parent_span_id is empty vec
    let span = Span {
        trace_id: vec![0x01u8; 16],
        span_id: vec![0x02u8; 8],
        trace_state: "".into(),
        parent_span_id: vec![],
        name: "root".into(),
        kind: 2,
        start_time_unix_nano: 1000,
        end_time_unix_nano: 2000,
        attributes: vec![],
        dropped_attributes_count: 0,
        status: Some(Status {
            code: 1,
            message: "".into(),
        }),
    };

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource { attributes: vec![] }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "test".into(),
                    version: "1.0".into(),
                }),
                spans: vec![span],
            }],
        }],
    };

    let encoded = request.encode_to_vec();
    let decoded = ExportTraceServiceRequest::decode(&encoded[..]).unwrap();
    let sp = &decoded.resource_spans[0].scope_spans[0].spans[0];
    assert!(sp.parent_span_id.is_empty(), "root span has no parent");
    assert_eq!(sp.trace_state, "");
    assert_eq!(sp.dropped_attributes_count, 0);
}

#[test]
fn test_empty_request() {
    let request = ExportTraceServiceRequest { resource_spans: vec![] };
    let encoded = request.encode_to_vec();
    let decoded = ExportTraceServiceRequest::decode(&encoded[..]).unwrap();
    assert!(decoded.resource_spans.is_empty());
}
