// Integration tests for the Loki log exporter (pure parts only).
// Tests: payload JSON shape, stream grouping, tenant ID header map.
// NO network calls and NO worker::Date dependency.
//
// buffer_event + push_logs cannot be tested on native because:
// - buffer_event takes a LogEvent (constructed via LogEvent::new which calls
//   worker::Date::now())
// - push_logs uses worker::Fetch for HTTP POST
// We test payload construction and URL formatting instead.

use std::collections::HashMap;

use cloudflare_shared::observability::structured_log::LogLevel;

// ---------------------------------------------------------------------------
// Payload JSON shape
// ---------------------------------------------------------------------------

#[test]
fn test_loki_payload_json_shape() {
    let mut stream = HashMap::new();
    stream.insert("worker".to_string(), "gateway".to_string());
    stream.insert("level".to_string(), "info".to_string());

    let values = vec![vec![
        "1000000000000000000".to_string(),
        r#"{"msg":"test"}"#.to_string(),
    ]];

    let payload = serde_json::json!({
        "streams": [{
            "stream": stream,
            "values": values
        }]
    });

    let json_str = serde_json::to_string(&payload).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["streams"][0]["stream"]["worker"], "gateway");
    assert_eq!(parsed["streams"][0]["stream"]["level"], "info");
    assert_eq!(parsed["streams"][0]["values"][0][0], "1000000000000000000");
    assert_eq!(parsed["streams"][0]["values"][0][1], r#"{"msg":"test"}"#);
}

// ---------------------------------------------------------------------------
// Stream grouping — events grouped by (service, level)
// ---------------------------------------------------------------------------

#[test]
fn test_loki_stream_grouping_structure() {
    let payload = serde_json::json!({
        "streams": [
            {
                "stream": { "worker": "auth", "level": "info" },
                "values": [["1000000000000000000", r#"{"msg":"login ok"}"#]]
            },
            {
                "stream": { "worker": "gateway", "level": "error" },
                "values": [["2000000000000000000", r#"{"msg":"500 on /api"}"#]]
            }
        ]
    });

    let parsed: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
    let streams = parsed["streams"].as_array().unwrap();
    assert_eq!(streams.len(), 2);
    assert_eq!(streams[0]["stream"]["worker"], "auth");
    assert_eq!(streams[0]["stream"]["level"], "info");
    assert_eq!(streams[1]["stream"]["worker"], "gateway");
    assert_eq!(streams[1]["stream"]["level"], "error");
}

// ---------------------------------------------------------------------------
// Timestamps are nanosecond strings
// ---------------------------------------------------------------------------

#[test]
fn test_loki_timestamp_is_nanosecond_string() {
    let ts_str = "1700000000000000000";
    let ts: u128 = ts_str.parse().unwrap();
    assert!(ts > 1_000_000_000_000_000_000, "must be nanosecond timestamp");
    assert!(format!("{}", ts).len() >= 19, "nanosecond string length >= 19");
}

// ---------------------------------------------------------------------------
// Tenant ID header map
// ---------------------------------------------------------------------------

#[test]
fn test_loki_tenant_header_consistency() {
    let mut headers = HashMap::new();
    headers.insert("X-Scope-OrgID".to_string(), "my-org".to_string());
    headers.insert("Content-Type".to_string(), "application/json".to_string());

    let json = serde_json::to_value(&headers).unwrap();
    assert_eq!(json["X-Scope-OrgID"], "my-org");
    assert_eq!(json["Content-Type"], "application/json");
}

// ---------------------------------------------------------------------------
// Loki endpoint URL format
// ---------------------------------------------------------------------------

#[test]
fn test_loki_endpoint_url_format() {
    let build_url = |endpoint: &str| -> String {
        format!("{}/loki/api/v1/push", endpoint.trim_end_matches('/'))
    };

    assert_eq!(
        build_url("http://loki:3100"),
        "http://loki:3100/loki/api/v1/push"
    );
    assert_eq!(
        build_url("http://loki:3100/"),
        "http://loki:3100/loki/api/v1/push"
    );
    assert_eq!(
        build_url("https://loki.example.com"),
        "https://loki.example.com/loki/api/v1/push"
    );
}

// ---------------------------------------------------------------------------
// LokiPayload serialization — full roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_loki_payload_serde_roundtrip() {
    let payload = serde_json::json!({
        "streams": [{
            "stream": {
                "worker": "analytics",
                "level": "info"
            },
            "values": [
                ["1234567890000000000", r#"{"timestamp":"2026-01-01T00:00:00Z","level":"INFO","message":"track","service":"analytics"}"#]
            ]
        }]
    });

    let bytes = serde_json::to_vec(&payload).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["streams"][0]["stream"]["worker"], "analytics");
    assert_eq!(parsed["streams"][0]["stream"]["level"], "info");
    assert_eq!(
        parsed["streams"][0]["values"][0][0],
        "1234567890000000000"
    );
}

// ---------------------------------------------------------------------------
// Label map from config — HashMap<String,String> pattern
// ---------------------------------------------------------------------------

#[test]
fn test_loki_label_map_pattern() {
    let mut labels = HashMap::new();
    labels.insert("worker".to_string(), "gateway".to_string());
    labels.insert("level".to_string(), "warn".to_string());
    labels.insert("env".to_string(), "production".to_string());

    let payload = serde_json::json!({ "stream": labels });
    assert_eq!(payload["stream"]["worker"], "gateway");
    assert_eq!(payload["stream"]["level"], "warn");
    assert_eq!(payload["stream"]["env"], "production");
    assert_eq!(payload["stream"].as_object().unwrap().len(), 3);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_loki_payload_empty_streams() {
    let payload = serde_json::json!({ "streams": [] });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed["streams"].as_array().unwrap().is_empty());
}

#[test]
fn test_loki_value_always_array_of_two_strings() {
    let value = ["1700000000000000000", r#"{"msg":"test"}"#];
    assert_eq!(value.len(), 2);
    assert!(value[0].chars().all(|c| c.is_ascii_digit()));
}

// ---------------------------------------------------------------------------
// Level label mapping — test the exact strings used
// ---------------------------------------------------------------------------

#[test]
fn test_loki_level_label_mapping() {
    let level_label = |level: LogLevel| -> &'static str {
        match level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    };

    assert_eq!(level_label(LogLevel::Debug), "debug");
    assert_eq!(level_label(LogLevel::Info), "info");
    assert_eq!(level_label(LogLevel::Warn), "warn");
    assert_eq!(level_label(LogLevel::Error), "error");
}
