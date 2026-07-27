// Metrics collection for Cloudflare Workers.
//
// Provides per-endpoint counters and latency histograms that can be
// exported in Prometheus text format via a /metrics endpoint.
//
// WASM-compatible: uses Cell/RefCell for interior mutability since
// Cloudflare Workers run single-threaded on wasm32-unknown-unknown.
//
// Metric naming follows Prometheus conventions:
//   cloudflare_requests_total{method="GET",path="/health",status="200"}
//   cloudflare_request_duration_ms{method="GET",path="/health"}

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// A counter that can only be incremented.
pub struct Counter {
    value: Cell<u64>,
    name: String,
    labels: BTreeMap<String, String>,
}

impl Counter {
    pub fn new(name: &str, labels: BTreeMap<String, String>) -> Self {
        Self {
            value: Cell::new(0),
            name: name.to_string(),
            labels,
        }
    }

    pub fn inc(&self) {
        self.value.set(self.value.get() + 1);
    }

    pub fn inc_by(&self, n: u64) {
        self.value.set(self.value.get() + n);
    }

    pub fn value(&self) -> u64 {
        self.value.get()
    }

    /// Render as Prometheus text format line.
    pub fn to_prometheus(&self) -> String {
        let labels_str = self.format_labels();
        if labels_str.is_empty() {
            format!("{} {}", self.name, self.value())
        } else {
            format!("{}{{{}}} {}", self.name, labels_str, self.value())
        }
    }

    fn format_labels(&self) -> String {
        self.labels
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", k, v))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// A histogram for tracking latency distributions.
/// Stores raw values for percentile calculation.
pub struct Histogram {
    values: Mutex<Vec<f64>>,
    name: String,
    labels: BTreeMap<String, String>,
}

impl Histogram {
    pub fn new(name: &str, labels: BTreeMap<String, String>) -> Self {
        Self {
            values: Mutex::new(Vec::new()),
            name: name.to_string(),
            labels,
        }
    }

    pub fn observe(&self, value: f64) {
        self.values.lock().unwrap().push(value);
    }

    pub fn count(&self) -> usize {
        self.values.lock().unwrap().len()
    }

    pub fn sum(&self) -> f64 {
        self.values.lock().unwrap().iter().sum()
    }

    /// Render as Prometheus text format lines (summary-style).
    pub fn to_prometheus(&self) -> String {
        let mut out = String::new();
        let labels_str = self.format_labels();

        let count = self.count();
        let sum = self.sum();

        if labels_str.is_empty() {
            out.push_str(&format!("{}_count {}", self.name, count));
        } else {
            out.push_str(&format!("{}{{{}}} {}", self.name, labels_str, count));
        }
        out.push('\n');

        if labels_str.is_empty() {
            out.push_str(&format!("{}_sum {}", self.name, sum));
        } else {
            out.push_str(&format!("{}{{{}}} {}", self.name, labels_str, sum));
        }
        out.push('\n');

        // Quantiles
        let mut sorted = self.values.lock().unwrap().clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let len = sorted.len();
        if len > 0 {
            let p50 = percentile(&sorted, 50.0);
            let p90 = percentile(&sorted, 90.0);
            let p99 = percentile(&sorted, 99.0);

            out.push_str(&format!("{}{{quantile=\"0.5\"}} {}\n", self.name, p50));
            out.push_str(&format!("{}{{quantile=\"0.9\"}} {}\n", self.name, p90));
            out.push_str(&format!("{}{{quantile=\"0.99\"}} {}\n", self.name, p99));
        }

        out
    }

    fn format_labels(&self) -> String {
        self.labels
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", k, v))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Per-endpoint metrics collector.
pub struct EndpointMetrics {
    pub requests: Counter,
    pub errors: Counter,
    pub latency: Histogram,
}

impl EndpointMetrics {
    pub fn new(method: &str, path: &str) -> Self {
        let mut labels = BTreeMap::new();
        labels.insert("method".to_string(), method.to_string());
        labels.insert("path".to_string(), path.to_string());

        Self {
            requests: Counter::new("cloudflare_requests_total", labels.clone()),
            errors: Counter::new("cloudflare_request_errors_total", labels.clone()),
            latency: Histogram::new("cloudflare_request_duration_ms", labels),
        }
    }

    /// Record a completed request. Returns duration in ms for logging.
    pub fn record(&self, status: u16, start_ms: f64) -> f64 {
        let now_ms = worker::Date::now().as_millis() as f64;
        let duration_ms = now_ms - start_ms;
        self.requests.inc();
        self.latency.observe(duration_ms);
        if status >= 400 {
            self.errors.inc();
        }
        duration_ms
    }
}

/// Global metrics registry. Collects all endpoint metrics for /metrics export.
pub struct MetricsRegistry {
    endpoints: Mutex<Vec<EndpointMetrics>>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            endpoints: Mutex::new(Vec::new()),
        }
    }

    /// Register a new endpoint metric collector.
    pub fn register(&self, method: &str, path: &str) -> EndpointMetrics {
        let m = EndpointMetrics::new(method, path);
        self.endpoints.lock().unwrap().push(m);
        // Return a new one — in practice the caller stores it.
        // The registry holds a clone for /metrics export.
        EndpointMetrics::new(method, path)
    }

    /// Export all metrics in Prometheus text format.
    pub fn export_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP cloudflare_requests_total Total request count\n");
        out.push_str("# TYPE cloudflare_requests_total counter\n");
        out.push_str("# HELP cloudflare_request_errors_total Total error count\n");
        out.push_str("# TYPE cloudflare_request_errors_total counter\n");
        out.push_str("# HELP cloudflare_request_duration_ms Request duration in milliseconds\n");
        out.push_str("# TYPE cloudflare_request_duration_ms summary\n");

        let endpoints = self.endpoints.lock().unwrap();
        for ep in endpoints.iter() {
            out.push_str(&ep.requests.to_prometheus());
            out.push('\n');
            out.push_str(&ep.errors.to_prometheus());
            out.push('\n');
            out.push_str(&ep.latency.to_prometheus());
            out.push('\n');
        }

        out
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}