pub mod health;
pub mod metrics;
pub mod otel;
pub mod otlp_proto;
pub mod structured_log;
pub mod trace_context;

pub use health::{DependencyHealth, HealthCheck, HealthRegistry, HealthStatus};
pub use metrics::{Counter, EndpointMetrics, Histogram, MetricsRegistry};
pub use otel::export_span;
pub use structured_log::{LogBuffer, LogEvent, LogLevel, Logger};
pub use trace_context::TraceContext;
