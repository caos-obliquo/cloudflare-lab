// Integration tests for the metrics module (Counter, Histogram, MetricsRegistry).
// Pure Rust — no WASM dependencies needed.

use std::collections::BTreeMap;

use cloudflare_shared::observability::metrics::{Counter, Histogram, MetricsRegistry};

// ---------------------------------------------------------------------------
// Counter
// ---------------------------------------------------------------------------

#[test]
fn test_counter_new_zero() {
    let c = Counter::new("test_counter", BTreeMap::new());
    assert_eq!(c.value(), 0);
}

#[test]
fn test_counter_inc() {
    let c = Counter::new("test_counter", BTreeMap::new());
    c.inc();
    assert_eq!(c.value(), 1);
    c.inc();
    assert_eq!(c.value(), 2);
}

#[test]
fn test_counter_inc_by() {
    let c = Counter::new("test_counter", BTreeMap::new());
    c.inc_by(42);
    assert_eq!(c.value(), 42);
    c.inc_by(8);
    assert_eq!(c.value(), 50);
}

#[test]
fn test_counter_render_no_labels() {
    let c = Counter::new("test_total", BTreeMap::new());
    c.inc_by(3);
    assert_eq!(c.to_prometheus(), "test_total 3");
}

#[test]
fn test_counter_render_with_labels() {
    let mut labels = BTreeMap::new();
    labels.insert("method".to_string(), "GET".to_string());
    labels.insert("path".to_string(), "/health".to_string());
    let c = Counter::new("test_total", labels);
    c.inc();
    assert_eq!(
        c.to_prometheus(),
        "test_total{method=\"GET\",path=\"/health\"} 1"
    );
}

// ---------------------------------------------------------------------------
// Histogram
// ---------------------------------------------------------------------------

#[test]
fn test_histogram_new_empty() {
    let h = Histogram::new("test_duration_ms", BTreeMap::new());
    assert_eq!(h.count(), 0);
    assert_eq!(h.sum(), 0.0);
}

#[test]
fn test_histogram_observe() {
    let h = Histogram::new("test_duration_ms", BTreeMap::new());
    h.observe(1.5);
    h.observe(2.5);
    assert_eq!(h.count(), 2);
    assert!((h.sum() - 4.0).abs() < f64::EPSILON);
}

#[test]
fn test_histogram_render_no_labels() {
    let h = Histogram::new("test_duration_ms", BTreeMap::new());
    h.observe(10.0);
    h.observe(20.0);
    h.observe(30.0);
    let text = h.to_prometheus();
    assert!(text.contains("test_duration_ms_count 3\n"), "count line");
    assert!(text.contains("test_duration_ms_sum 60\n"), "sum line");
    // Quantile lines
    assert!(text.contains("test_duration_ms{quantile=\"0.5\"}"), "p50");
    assert!(text.contains("test_duration_ms{quantile=\"0.9\"}"), "p90");
    assert!(text.contains("test_duration_ms{quantile=\"0.99\"}"), "p99");
    assert!(text.ends_with('\n'), "ends with newline");
}

#[test]
fn test_histogram_render_empty_gives_no_quantiles() {
    let h = Histogram::new("empty_ms", BTreeMap::new());
    let text = h.to_prometheus();
    assert!(text.contains("empty_ms_count 0\n"), "count line missing in: {:?}", text);
    // Sum of empty f64 iterator is -0.0 in Rust; both 0 and -0 are acceptable
    assert!(
        text.contains("empty_ms_sum 0\n") || text.contains("empty_ms_sum -0\n"),
        "sum line missing in: {:?}",
        text
    );
    assert_eq!(text.matches("quantile=").count(), 0);
}

#[test]
fn test_histogram_p50_p90_p99_on_known_dataset() {
    let h = Histogram::new("known_ms", BTreeMap::new());
    // Record 1..=100
    for i in 1..=100u64 {
        h.observe(i as f64);
    }
    let text = h.to_prometheus();
    // For dataset 1..=100:
    //   p50 ~ 50.0 or 51.0 depending on interpolation
    //   p90 ~ 90.0 or 91.0
    //   p99 ~ 99.0 or 100.0
    // Our percentile fn: idx = round(p/100 * (len-1))
    //   p50: idx=round(0.5*99)=50 -> val=50.0 (0-indexed: sorted[50]=51)
    // Actually sorted[50] = 51 (0-indexed: element 0 is 1, element 50 is 51)
    // Let's verify the actual values by checking the output
    assert!(text.contains("known_ms_count 100\n"));
    assert!((h.sum() - 5050.0).abs() < f64::EPSILON);
    // Just check quantile lines exist — precise values depend on percentile fn
    assert!(text.contains("known_ms{quantile=\"0.5\"}"));
    assert!(text.contains("known_ms{quantile=\"0.9\"}"));
    assert!(text.contains("known_ms{quantile=\"0.99\"}"));
}

#[test]
fn test_histogram_percentile_values() {
    // Verify exact p50/p90/p99 on dataset 1..=10
    let h = Histogram::new("pct_ms", BTreeMap::new());
    for i in 1..=10u64 {
        h.observe(i as f64);
    }
    // sorted = [1,2,3,4,5,6,7,8,9,10], len=10
    // p50: idx = round(0.5 * 9) = round(4.5) = 5 => sorted[5] = 6
    // p90: idx = round(0.9 * 9) = round(8.1) = 8 => sorted[8] = 9
    // p99: idx = round(0.99 * 9) = round(8.91) = 9 => sorted[9] = 10
    let text = h.to_prometheus();
    // Extract quantile values from the text — look for specific patterns
    assert!(
        text.contains("pct_ms{quantile=\"0.5\"} 6\n"),
        "p50 should be ~6: got lines:\n{}",
        text
    );
    assert!(
        text.contains("pct_ms{quantile=\"0.9\"} 9\n"),
        "p90 should be ~9"
    );
    assert!(
        text.contains("pct_ms{quantile=\"0.99\"} 10\n"),
        "p99 should be ~10"
    );
}

// ---------------------------------------------------------------------------
// Histogram with labels — the KNOWN BUG regression test
// ---------------------------------------------------------------------------

#[test]
fn test_histogram_render_with_labels_preserves_them() {
    let mut labels = BTreeMap::new();
    labels.insert("method".to_string(), "GET".to_string());
    labels.insert("path".to_string(), "/health".to_string());
    let h = Histogram::new("req_duration_ms", labels);
    h.observe(12.3);
    h.observe(45.6);

    let text = h.to_prometheus();
    // _count line must include labels
    assert!(
        text.contains("req_duration_ms{method=\"GET\",path=\"/health\"} 2\n"),
        "count line missing labels"
    );
    // _sum line must include labels
    assert!(
        text.contains("req_duration_ms{method=\"GET\",path=\"/health\"} 57.9"),
        "sum line missing labels"
    );
    // Each quantile line must have the FULL label set PLUS quantile INSIDE braces
    for quantile in ["0.5", "0.9", "0.99"] {
        let expected = format!(
            "req_duration_ms{{method=\"GET\",path=\"/health\",quantile=\"{}\"}}",
            quantile
        );
        assert!(
            text.contains(&expected),
            "quantile q={} line must preserve labels:\n{}",
            quantile,
            text
        );
    }
    // Verify the OLD BUG pattern is NOT present (quantile outside braces)
    let old_bug = format!(
        "req_duration_ms{}{{quantile=",
        "{method=\"GET\",path=\"/health\"}"
    );
    assert!(
        !text.contains(&old_bug),
        "OLD BUG: labels and quantile must be INSIDE same braces, not concatenated"
    );
    // Verify no line has two brace groups {..}{..}
    for line in text.lines() {
        let brace_count = line.chars().filter(|&c| c == '{').count();
        assert!(
            brace_count <= 1,
            "line has multiple brace groups (invalid Prometheus): {}",
            line
        );
    }
}

#[test]
fn test_histogram_render_with_labels_only_one_label() {
    let mut labels = BTreeMap::new();
    labels.insert("status".to_string(), "200".to_string());
    let h = Histogram::new("http_requests", labels);
    h.observe(5.0);

    let text = h.to_prometheus();
    assert!(text.contains("http_requests{status=\"200\"} 1\n"), "count");
    assert!(text.contains("http_requests{status=\"200\",quantile=\"0.5\"}"), "p50 with label");
    assert!(text.contains("http_requests{status=\"200\",quantile=\"0.9\"}"), "p90 with label");
    assert!(text.contains("http_requests{status=\"200\",quantile=\"0.99\"}"), "p99 with label");
}

// ---------------------------------------------------------------------------
// MetricsRegistry
// ---------------------------------------------------------------------------

#[test]
fn test_registry_register_and_export_help_type() {
    let reg = MetricsRegistry::new();
    let _ep = reg.register("GET", "/health");

    let output = reg.export_prometheus();
    // HELP and TYPE lines must appear once per metric family
    assert!(output.contains("# HELP cloudflare_requests_total Total request count\n"));
    assert!(output.contains("# TYPE cloudflare_requests_total counter\n"));
    assert!(output.contains("# HELP cloudflare_request_errors_total Total error count\n"));
    assert!(output.contains("# TYPE cloudflare_request_errors_total counter\n"));
    assert!(output.contains("# HELP cloudflare_request_duration_ms Request duration in milliseconds\n"));
    assert!(output.contains("# TYPE cloudflare_request_duration_ms summary\n"));
}

#[test]
fn test_registry_one_endpoint_exports_all_metrics() {
    let reg = MetricsRegistry::new();
    let ep = reg.register("GET", "/api");

    // Simulate some activity (without calling record() which uses worker::Date)
    ep.requests.inc_by(5);
    ep.errors.inc_by(1);
    ep.latency.observe(10.0);
    ep.latency.observe(20.0);

    let output = reg.export_prometheus();
    assert!(output.contains("cloudflare_requests_total{method=\"GET\",path=\"/api\"} 5\n"));
    assert!(output.contains("cloudflare_request_errors_total{method=\"GET\",path=\"/api\"} 1\n"));
    assert!(output.contains("cloudflare_request_duration_ms{method=\"GET\",path=\"/api\"} 2\n")); // count
    assert!(output.contains("cloudflare_request_duration_ms{method=\"GET\",path=\"/api\",quantile=\"0.5\"}"));
}

#[test]
fn test_registry_multiple_endpoints() {
    let reg = MetricsRegistry::new();
    let ep1 = reg.register("GET", "/health");
    let ep2 = reg.register("POST", "/data");

    ep1.requests.inc_by(3);
    ep2.requests.inc_by(7);

    let output = reg.export_prometheus();
    assert!(output.contains(r#"{method="GET",path="/health"}"#));
    assert!(output.contains(r#"{method="POST",path="/data"}"#));
    assert!(output.contains("cloudflare_requests_total{method=\"GET\",path=\"/health\"} 3\n"));
    assert!(output.contains("cloudflare_requests_total{method=\"POST\",path=\"/data\"} 7\n"));
}

#[test]
fn test_registry_empty() {
    let reg = MetricsRegistry::new();
    let output = reg.export_prometheus();
    // HELP/TYPE still present even with no endpoints
    assert!(output.contains("# HELP"));
    assert!(output.contains("# TYPE"));
    // No data lines (no counters/histograms without endpoints)
    assert!(!output.contains("cloudflare_requests_total{"));
}

// ---------------------------------------------------------------------------
// Label escaping — if labels contain special chars
// ---------------------------------------------------------------------------

#[test]
fn test_counter_labels_with_special_chars() {
    let mut labels = BTreeMap::new();
    labels.insert("path".to_string(), "/api/v1/users".to_string());
    let c = Counter::new("requests", labels);
    c.inc();
    let text = c.to_prometheus();
    assert_eq!(text, "requests{path=\"/api/v1/users\"} 1");
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_histogram_large_values() {
    let h = Histogram::new("big", BTreeMap::new());
    h.observe(1e12);
    h.observe(2e12);
    assert!((h.sum() - 3e12).abs() / 3e12 < 0.01);
    let text = h.to_prometheus();
    assert!(text.contains("big_count 2\n"));
}
