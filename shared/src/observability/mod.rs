pub mod health;
pub mod loki;
pub mod metrics;
pub mod otel;
pub mod otlp_proto;
pub mod structured_log;
pub mod trace_context;

// Time helpers gated by target: workers always run wasm32, but unit tests run
// natively where worker::Date (js-sys) panics. Native fallbacks exist ONLY so
// `cargo test` works on the host; production behavior is unchanged.
#[cfg(target_arch = "wasm32")]
pub(crate) fn now_string() -> String {
    worker::Date::now().to_string()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    secs.to_string()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_millis() -> u64 {
    worker::Date::now().as_millis()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub use health::{DependencyHealth, HealthCheck, HealthRegistry, HealthStatus};
pub use loki::{buffer_event, push_logs};
pub use metrics::{Counter, EndpointMetrics, Histogram, MetricsRegistry};
pub use otel::export_span;
pub use structured_log::{LogBuffer, LogEvent, LogLevel, Logger};
pub use trace_context::TraceContext;
