// Structured JSON logging for Cloudflare Workers.
//
// Produces consistent JSON log entries across all services:
//   {"timestamp":"2026-07-24T10:30:00.000Z","level":"INFO","message":"...",
//    "service":"gateway","trace_id":"...","span_id":"...","duration_ms":42,
//    "method":"GET","path":"/health","status":200,"error":null}
//
// All logs go through `console_log!` which appears in Cloudflare's dashboard
// and logpush. The JSON format enables structured querying in log analysis tools.

use std::sync::Mutex;

use serde::Serialize;
use super::now_string;

use crate::observability::trace_context::TraceContext;

/// Log severity levels matching standard logging conventions.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// A structured log event with consistent schema.
#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub service: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl LogEvent {
    /// Create a new log event with the current timestamp.
    pub fn new(level: LogLevel, message: &str, service: &str) -> Self {
        Self {
            timestamp: now_string(),
            level,
            message: message.to_string(),
            service: service.to_string(),
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

    /// Attach trace context to this log event.
    pub fn with_trace(mut self, ctx: &TraceContext) -> Self {
        self.trace_id = Some(ctx.trace_id.clone());
        self.span_id = Some(ctx.span_id.clone());
        self
    }

    /// Attach HTTP request context.
    pub fn with_http(mut self, method: &str, path: &str) -> Self {
        self.method = Some(method.to_string());
        self.path = Some(path.to_string());
        self
    }

    /// Attach HTTP response status.
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Attach duration in milliseconds.
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    /// Attach error information.
    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }

    /// Attach arbitrary metadata as key-value pairs.
    pub fn with_metadata(mut self, key: &str, value: serde_json::Value) -> Self {
        let mut meta = self.metadata.unwrap_or(serde_json::json!({}));
        if let Some(obj) = meta.as_object_mut() {
            obj.insert(key.to_string(), value);
        }
        self.metadata = Some(meta);
        self
    }

    /// Emit this log event via console_log! as a JSON string.
    pub fn emit(&self) {
        let json_str = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"level":"ERROR","message":"log serialization failed","service":"{}"}}"#,
                self.service
            )
        });
        worker::console_log!("{}", json_str);
    }
}

/// Convenience builder for common log patterns.
pub struct Logger {
    service: String,
}

impl Logger {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }

    /// Log at INFO level.
    pub fn info(&self, msg: &str) -> LogEvent {
        LogEvent::new(LogLevel::Info, msg, &self.service)
    }

    /// Log at WARN level.
    pub fn warn(&self, msg: &str) -> LogEvent {
        LogEvent::new(LogLevel::Warn, msg, &self.service)
    }

    /// Log at ERROR level.
    pub fn error(&self, msg: &str) -> LogEvent {
        LogEvent::new(LogLevel::Error, msg, &self.service)
    }

    /// Log at DEBUG level.
    pub fn debug(&self, msg: &str) -> LogEvent {
        LogEvent::new(LogLevel::Debug, msg, &self.service)
    }

    /// Log an incoming request with trace context.
    pub fn request(&self, method: &str, path: &str, ctx: &TraceContext) -> LogEvent {
        self.info(&format!("{} {}", method, path))
            .with_trace(ctx)
            .with_http(method, path)
    }

    /// Log a completed response with duration.
    pub fn response(&self, method: &str, path: &str, status: u16, duration_ms: u64, ctx: &TraceContext) -> LogEvent {
        let level = if status >= 500 {
            LogLevel::Error
        } else if status >= 400 {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };
        LogEvent::new(level, &format!("{} {} -> {}", method, path, status), &self.service)
            .with_trace(ctx)
            .with_http(method, path)
            .with_status(status)
            .with_duration(duration_ms)
    }
}

/// In-memory ring buffer of recent log events for debugging endpoints.
/// Uses RefCell for interior mutability (WASM is single-threaded).
pub struct LogBuffer {
    events: Mutex<Vec<LogEvent>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Mutex::new(Vec::with_capacity(capacity)),
            capacity,
        }
    }

    /// Push a log event into the buffer. Evicts oldest if at capacity.
    pub fn push(&self, event: LogEvent) {
        let mut events = self.events.lock().unwrap();
        if events.len() >= self.capacity {
            events.remove(0);
        }
        events.push(event);
    }

    /// Return recent events as JSON value.
    pub fn recent(&self, limit: usize) -> serde_json::Value {
        let events = self.events.lock().unwrap();
        let count = limit.min(events.len());
        let slice: Vec<&LogEvent> = events.iter().rev().take(count).collect();
        serde_json::to_value(slice).unwrap_or(serde_json::json!([]))
    }

    /// Clear all buffered events.
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}
