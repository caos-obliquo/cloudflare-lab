pub mod trace_context;
pub mod structured_log;
pub mod metrics;
pub mod health;
pub mod otel;
pub mod otlp_proto;

pub use trace_context::TraceContext;
pub use structured_log::{LogEvent, LogLevel, Logger, LogBuffer};
pub use metrics::{Counter, Histogram, EndpointMetrics, MetricsRegistry};
pub use health::{HealthRegistry, HealthStatus, DependencyHealth, HealthCheck};
pub use otel::export_span;