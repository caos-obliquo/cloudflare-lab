//! Rust Lambda custom runtime (provided.al2023). SigV4-authed Function URL handler.
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde_json::json;
use std::env;
use std::time::{Duration, Instant, SystemTime};

// ---------------------------------------------------------------------------
// TraceContext — W3C traceparent propagation (native Linux, not WASM)
// ---------------------------------------------------------------------------
// Duplicated from cloudflare-shared because this Lambda runs on x86_64 Linux
// (not wasm32-unknown-unknown) and depends on lambda_http, not worker.
// The protocol is identical: 00-<trace_id_32hex>-<span_id_16hex>-<trace_flags>

struct TraceContext {
    trace_id: String,
    span_id: String,
    trace_flags: String,
}

#[allow(dead_code)]
impl TraceContext {
    // Generate a new trace root with random IDs.
    fn new() -> Self {
        let mut trace_buf = [0u8; 16];
        let mut span_buf = [0u8; 8];
        getrandom::getrandom(&mut trace_buf).expect("getrandom trace_id");
        getrandom::getrandom(&mut span_buf).expect("getrandom span_id");
        TraceContext {
            trace_id: hex_encode(&trace_buf),
            span_id: hex_encode(&span_buf),
            trace_flags: "01".into(),
        }
    }

    // Parse a W3C traceparent header value. Returns None if the format is invalid.
    // Accepts: 00-<trace_id_32hex>-<span_id_16hex>-<flags>
    fn from_traceparent(header: &str) -> Option<Self> {
        let h = header.trim();
        let parts: Vec<&str> = h.split('-').collect();
        if parts.len() != 4 { return None; }
        if parts[0] != "00" { return None; }
        let trace_id = parts[1];
        let span_id = parts[2];
        let flags = parts[3];
        if trace_id.len() != 32 || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
        if span_id.len() != 16 || !span_id.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
        if flags.len() != 2 || !flags.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
        Some(TraceContext {
            trace_id: trace_id.to_lowercase(),
            span_id: span_id.to_lowercase(),
            trace_flags: flags.to_lowercase(),
        })
    }

    // Serialize back to W3C traceparent header format.
    fn to_traceparent(&self) -> String {
        format!("00-{}-{}-{}", self.trace_id, self.span_id, self.trace_flags)
    }

    // Derive a child span — same trace_id, new span_id. Used when this Lambda
    // makes downstream calls that need their own span identity.
    fn child_span(&self) -> Self {
        let mut span_buf = [0u8; 8];
        getrandom::getrandom(&mut span_buf).expect("getrandom child span_id");
        TraceContext {
            trace_id: self.trace_id.clone(),
            span_id: hex_encode(&span_buf),
            trace_flags: self.trace_flags.clone(),
        }
    }
}

// Encode bytes as lowercase hex. Pre-allocates exact capacity.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

// ---------------------------------------------------------------------------
// Structured JSON logging (stdout → CloudWatch)
// ---------------------------------------------------------------------------
// Every log entry is a single JSON line. Error-level logs go to stderr so
// Lambda's runtime can distinguish them from application output.
// Schema: timestamp, level, message, service, trace_id, span_id, duration_ms.

#[allow(dead_code)]
enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

struct LogEvent {
    timestamp: String,
    level: LogLevel,
    message: String,
    service: String,
    trace_id: String,
    span_id: String,
    duration_ms: Option<u64>,
}

impl LogEvent {
    fn emit(&self) {
        let lvl = self.level.to_string();
        let obj = json!({
            "timestamp": self.timestamp,
            "level": lvl,
            "message": self.message,
            "service": self.service,
            "trace_id": self.trace_id,
            "span_id": self.span_id,
            "duration_ms": self.duration_ms,
        });
        let line = serde_json::to_string(&obj).expect("serialize log event");
        match self.level {
            LogLevel::Error => eprintln!("{}", line),
            _ => println!("{}", line),
        }
    }
}

struct Logger {
    service: String,
}

#[allow(dead_code)]
impl Logger {
    fn new(service: &str) -> Self {
        Logger { service: service.to_owned() }
    }

    fn info(&self, msg: &str, tc: &TraceContext) {
        LogEvent {
            timestamp: iso_timestamp(),
            level: LogLevel::Info,
            message: msg.to_owned(),
            service: self.service.clone(),
            trace_id: tc.trace_id.clone(),
            span_id: tc.span_id.clone(),
            duration_ms: None,
        }.emit();
    }

    fn warn(&self, msg: &str, tc: &TraceContext) {
        LogEvent {
            timestamp: iso_timestamp(),
            level: LogLevel::Warn,
            message: msg.to_owned(),
            service: self.service.clone(),
            trace_id: tc.trace_id.clone(),
            span_id: tc.span_id.clone(),
            duration_ms: None,
        }.emit();
    }

    fn error(&self, msg: &str, tc: &TraceContext) {
        LogEvent {
            timestamp: iso_timestamp(),
            level: LogLevel::Error,
            message: msg.to_owned(),
            service: self.service.clone(),
            trace_id: tc.trace_id.clone(),
            span_id: tc.span_id.clone(),
            duration_ms: None,
        }.emit();
    }

    fn request(&self, method: &str, path: &str, tc: &TraceContext) {
        LogEvent {
            timestamp: iso_timestamp(),
            level: LogLevel::Info,
            message: format!("→ {} {}", method, path),
            service: self.service.clone(),
            trace_id: tc.trace_id.clone(),
            span_id: tc.span_id.clone(),
            duration_ms: None,
        }.emit();
    }

    fn response(&self, status: u16, duration: Duration, tc: &TraceContext) {
        let ms = duration.as_millis() as u64;
        let lvl = if status >= 500 { LogLevel::Error } else if status >= 400 { LogLevel::Warn } else { LogLevel::Info };
        LogEvent {
            timestamp: iso_timestamp(),
            level: lvl,
            message: format!("← {} ({}ms)", status, ms),
            service: self.service.clone(),
            trace_id: tc.trace_id.clone(),
            span_id: tc.span_id.clone(),
            duration_ms: Some(ms),
        }.emit();
    }
}

// ISO-8601 UTC timestamp without chrono dependency.
// Uses N. Devillard's civil-date-from-UNIX-seconds algorithm for date computation.
fn iso_timestamp() -> String {
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = d.as_secs();
    let millis = d.subsec_millis();

    // Civil date from UNIX seconds (algorithm from N. Devillard).
    let z = (total_secs / 86400) as i64;
    let mut y = (100 * z - 108) / 36525 + 1970;
    loop {
        let days_this_year = if is_leap(y) { 366 } else { 365 };
        let days_since_epoch = {
            let mut days = 0;
            for yr in 1970..y { days += if is_leap(yr) { 366 } else { 365 }; }
            days
        };
        if z - days_since_epoch < days_this_year { break; }
        y += 1;
    }
    let mut days_since_epoch = 0i64;
    for yr in 1970..y { days_since_epoch += if is_leap(yr) { 366 } else { 365 }; }
    let day_of_year = (z - days_since_epoch) as i64;

    let sec_today = total_secs % 86400;
    let h = sec_today / 3600;
    let m = (sec_today % 3600) / 60;
    let s = sec_today % 60;

    let month_len = if is_leap(y) { [31,29,31,30,31,30,31,31,30,31,30,31] } else { [31,28,31,30,31,30,31,31,30,31,30,31] };
    let mut mo: usize = 0;
    let mut rest = day_of_year;
    for (i, &ml) in month_len.iter().enumerate() {
        if rest < ml { mo = i; break; }
        rest -= ml;
    }
    let day = rest + 1;

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, mo + 1, day, h, m, s, millis)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

static SERVICE: &str = "devops-api";

// Main request handler. Every request follows the same lifecycle:
//   1. Extract or create trace context from traceparent header
//   2. Log the incoming request with trace IDs
//   3. Route to handler based on method + path
//   4. Log the response with duration and status
//   5. Return response with traceparent header for downstream trace continuity
async fn handler(req: Request) -> Result<Response<Body>, Error> {
    let log = Logger::new(SERVICE);
    let start = Instant::now();

    // If the Gateway worker proxied this request, it included a traceparent header
    // so we join the existing trace. Otherwise, start a new trace.
    let tc = req
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .and_then(TraceContext::from_traceparent)
        .unwrap_or_else(TraceContext::new);

    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    log.request(&method, &path, &tc);

    let result = match (method.as_str(), path.as_str()) {
        ("GET", "/health") => ok(json!({"status":"ok","service":"devops-api"}), &tc),
        ("GET", "/config") => {
            let config = json!({
                "environment": env::var("ENVIRONMENT").unwrap_or_default(),
                "worker_gateway_url": env::var("WORKER_GATEWAY_URL").unwrap_or_default(),
                "worker_auth_url": env::var("WORKER_AUTH_URL").unwrap_or_default(),
            });
            ok(config, &tc)
        }
        // Proxy stubs — each receives a traceparent header for end-to-end trace continuity.
        // These will forward to the corresponding Worker when the T25 integration is wired.
        ("POST", "/workers/query") => {
            ok(json!({"status":"ok","message":"worker proxy endpoint","note":"wired by T25"}), &tc)
        }
        ("POST", "/d1/query") => {
            ok(json!({"status":"ok","message":"d1 proxy endpoint","note":"wired by T25"}), &tc)
        }
        ("POST", "/workers/register") => {
            ok(json!({"status":"ok","message":"register proxy endpoint","note":"wired by T25"}), &tc)
        }
        _ => {
            let body = json!({"status":"error","error":"not found","path":path});
            let resp = Response::builder()
                .status(404)
                .header("content-type", "application/json")
                .header("x-request-id", uuid_v4())
                .header("traceparent", tc.to_traceparent())
                .body(Body::from(serde_json::to_string(&body)?))?;
            let dur = start.elapsed();
            log.response(404, dur, &tc);
            return Ok(resp);
        }
    };

    let dur = start.elapsed();
    let status = match &result {
        Ok(r) => r.status().as_u16(),
        Err(_) => 500,
    };
    log.response(status, dur, &tc);
    result
}

// Build a 200 JSON response with traceparent and request-id headers.
// Every response includes a traceparent so the Gateway worker (or any caller)
// can continue the trace across service boundaries.
fn ok(body: serde_json::Value, tc: &TraceContext) -> Result<Response<Body>, Error> {
    let resp = Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .header("x-request-id", uuid_v4())
        .header("traceparent", tc.to_traceparent())
        .body(Body::from(serde_json::to_string(&body)?))?;
    Ok(resp)
}

// Timestamp-based request ID. Not cryptographic — just a unique-enough identifier
// for correlating logs across services. "lam-" prefix distinguishes Lambda-issued
// IDs from Worker-issued IDs in distributed traces.
fn uuid_v4() -> String {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("lam-{:016x}", ts)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // lambda_http::run takes a service_fn that converts each incoming
    // Lambda Function URL request through our handler and returns a response.
    run(service_fn(handler)).await
}