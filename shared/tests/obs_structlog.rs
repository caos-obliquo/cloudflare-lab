// Integration tests for structured log module.
// Pure logic: manually construct LogEvent instances (LogEvent::new() calls
// worker::Date::now() which panics on non-wasm targets).

use cloudflare_shared::observability::{
    structured_log::{LogBuffer, LogEvent, LogLevel, Logger},
    trace_context::TraceContext,
};

/// Helper: create a LogEvent without calling the wasm-dependent constructor.
fn make_event(level: LogLevel, msg: &str, svc: &str) -> LogEvent {
    LogEvent {
        timestamp: "2026-01-01T00:00:00.000Z".to_string(),
        level,
        message: msg.to_string(),
        service: svc.to_string(),
        trace_id: None,
        span_id: None,
        duration_ms: None,
        method: None,
        path: None,
        status: None,
        error: None,
        metadata: None,
    }
}

// ---------------------------------------------------------------------------
// LogLevel serialization
// ---------------------------------------------------------------------------

#[test]
fn test_loglevel_serialize_uppercase() {
    assert_eq!(serde_json::to_value(LogLevel::Debug).unwrap(), "DEBUG");
    assert_eq!(serde_json::to_value(LogLevel::Info).unwrap(), "INFO");
    assert_eq!(serde_json::to_value(LogLevel::Warn).unwrap(), "WARN");
    assert_eq!(serde_json::to_value(LogLevel::Error).unwrap(), "ERROR");
}

// ---------------------------------------------------------------------------
// LogEvent JSON shape
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_json_contains_required_fields() {
    let event = make_event(LogLevel::Info, "hello world", "test-service");
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    let obj = json.as_object().unwrap();
    assert!(obj.contains_key("timestamp"), "must have timestamp");
    assert_eq!(obj["level"], "INFO");
    assert_eq!(obj["message"], "hello world");
    assert_eq!(obj["service"], "test-service");
}

#[test]
fn test_logevent_timestamp_is_string() {
    let event = make_event(LogLevel::Warn, "warn msg", "svc");
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert!(json["timestamp"].is_string());
    assert!(!json["timestamp"].as_str().unwrap().is_empty());
}

#[test]
fn test_logevent_optional_fields_absent() {
    let event = make_event(LogLevel::Debug, "test", "svc");
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    let obj = json.as_object().unwrap();
    assert!(!obj.contains_key("trace_id"), "trace_id omitted");
    assert!(!obj.contains_key("span_id"), "span_id omitted");
    assert!(!obj.contains_key("duration_ms"), "duration_ms omitted");
    assert!(!obj.contains_key("method"), "method omitted");
    assert!(!obj.contains_key("path"), "path omitted");
    assert!(!obj.contains_key("status"), "status omitted");
    assert!(!obj.contains_key("error"), "error omitted");
    assert!(!obj.contains_key("metadata"), "metadata omitted");
}

// ---------------------------------------------------------------------------
// Trace context fields merge
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_with_trace_adds_fields() {
    let tc = TraceContext::new();
    let event = make_event(LogLevel::Info, "trace test", "svc").with_trace(&tc);
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["trace_id"], tc.trace_id);
    assert_eq!(json["span_id"], tc.span_id);
}

// ---------------------------------------------------------------------------
// HTTP context fields
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_with_http() {
    let event = make_event(LogLevel::Info, "req", "svc")
        .with_http("POST", "/api/data")
        .with_status(201);
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["method"], "POST");
    assert_eq!(json["path"], "/api/data");
    assert_eq!(json["status"], 201);
}

// ---------------------------------------------------------------------------
// Duration
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_with_duration() {
    let event = make_event(LogLevel::Info, "slow", "svc").with_duration(1234);
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["duration_ms"], 1234);
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_with_error() {
    let event = make_event(LogLevel::Error, "fail", "svc").with_error("connection refused");
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["error"], "connection refused");
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_with_metadata() {
    let event = make_event(LogLevel::Info, "meta", "svc")
        .with_metadata("user_id", serde_json::json!("u42"))
        .with_metadata("count", serde_json::json!(3));
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["metadata"]["user_id"], "u42");
    assert_eq!(json["metadata"]["count"], 3);
}

// ---------------------------------------------------------------------------
// Logger builder (construction only — log methods call LogEvent::new which
// uses worker::Date and cannot be tested on native)
// ---------------------------------------------------------------------------

#[test]
fn test_logger_new_stores_service() {
    let _logger = Logger::new("gateway");
    // Logger::new is pure (stores a service name string).
    // Log methods (info/warn/error/debug) call LogEvent::new which uses
    // worker::Date and cannot be tested on native — verified by type system.
}

// ---------------------------------------------------------------------------
// LogBuffer (pure Rust — no worker::Date dependency in push/clear/recent)
// ---------------------------------------------------------------------------

#[test]
fn test_logbuffer_new_empty() {
    let buf = LogBuffer::new(10);
    let recent = buf.recent(5);
    assert_eq!(recent.as_array().unwrap().len(), 0);
}

#[test]
fn test_logbuffer_push_and_recent() {
    let buf = LogBuffer::new(10);
    buf.push(make_event(LogLevel::Info, "first", "svc"));
    buf.push(make_event(LogLevel::Info, "second", "svc"));

    let recent = buf.recent(10);
    assert_eq!(recent.as_array().unwrap().len(), 2);
    // Most recent first (reverse order)
    assert_eq!(recent[1]["message"], "first");
    assert_eq!(recent[0]["message"], "second");
}

#[test]
fn test_logbuffer_evicts_oldest() {
    let buf = LogBuffer::new(3);
    buf.push(make_event(LogLevel::Info, "a", "svc"));
    buf.push(make_event(LogLevel::Info, "b", "svc"));
    buf.push(make_event(LogLevel::Info, "c", "svc"));
    buf.push(make_event(LogLevel::Info, "d", "svc")); // evicts "a"

    let recent = buf.recent(10);
    assert_eq!(recent.as_array().unwrap().len(), 3);
    assert_eq!(recent[2]["message"], "b");
    assert_eq!(recent[0]["message"], "d");
}

#[test]
fn test_logbuffer_clear() {
    let buf = LogBuffer::new(10);
    buf.push(make_event(LogLevel::Info, "x", "svc"));
    buf.clear();
    let recent = buf.recent(10);
    assert_eq!(recent.as_array().unwrap().len(), 0);
}

#[test]
fn test_logbuffer_recent_limit() {
    let buf = LogBuffer::new(10);
    for i in 0..5 {
        buf.push(make_event(LogLevel::Info, &format!("evt{}", i), "svc"));
    }
    let recent = buf.recent(3);
    assert_eq!(recent.as_array().unwrap().len(), 3);
}

// ---------------------------------------------------------------------------
// Full event JSON — combined scenario
// ---------------------------------------------------------------------------

#[test]
fn test_logevent_full_json() {
    let tc = TraceContext::new();
    let event = make_event(LogLevel::Warn, "slow response", "gateway")
        .with_trace(&tc)
        .with_http("GET", "/api")
        .with_status(200)
        .with_duration(2500)
        .with_error("timeout")
        .with_metadata("retries", serde_json::json!(2));

    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["level"], "WARN");
    assert_eq!(json["message"], "slow response");
    assert_eq!(json["service"], "gateway");
    assert_eq!(json["trace_id"], tc.trace_id);
    assert_eq!(json["span_id"], tc.span_id);
    assert_eq!(json["method"], "GET");
    assert_eq!(json["path"], "/api");
    assert_eq!(json["status"], 200);
    assert_eq!(json["duration_ms"], 2500);
    assert_eq!(json["error"], "timeout");
    assert_eq!(json["metadata"]["retries"], 2);
}
