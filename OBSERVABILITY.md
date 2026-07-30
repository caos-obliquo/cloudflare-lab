# Observability Architecture

Production observability for Cloudflare Workers: OTel traces, Prometheus metrics,
structured logs, dependency health checks. Gateway worker is the observability hub.
SigNoz collector receives OTel spans over HTTP/protobuf.

**Reference**: README.md for project overview, worker routes, deployment.

---

## Architecture

```
                                             SigNoz Stack
                                         ┌──────────────────────────┐
                                         │  SigNoz Query Service    │
                                         │  (Grafana-like frontend) │
                                         └──────────┬───────────────┘
                                                    │ SQL
                                         ┌──────────▼───────────────┐
                                         │  ClickHouse (traces)     │
                                         │  │  (metrics)            │
                                         │  │  (logs)               │
                                         └──────────┬───────────────┘
                                                    │ OTLP/gRPC
                                         ┌──────────▼───────────────┐
                                         │  OTel Collector          │
                                         │  :4317 gRPC              │
                                         │  :4318 HTTP/protobuf     │
                                         └──────────┬───────────────┘
                                                    │ HTTP POST :4318/v1/traces
                                                    │ (protobuf binary)
                                                    │
  ┌──────────────┐   ┌──────────────────────────────────────────────────────┐
  │   Browser    │──▶│  Gateway Worker (observability hub)                  │
  │              │   │                                                      │
  │  Landing     │   │  ┌─────────────┐   ┌──────────────┐   ┌──────────┐  │
  │  Page (LP)   │   │  │ Router      │──▶│ OTel export  │──▶│ SigNoz   │  │
  │  cloudflare- │   │  │ (routes.rs) │   │ (otel.rs)    │   │ Collector│  │
  │  lab.com     │   │  │             │   │ retry×3      │   │ :4318    │  │
  │              │   │  │ /metrics ───│──▶│ buffer 100   │   └──────────┘  │
  │              │   │  │ /logs    ───│──▶│ FIFO drop    │                 │
  │              │   │  │ /health ───│──▶│              │                 │
  └──────────────┘   │  │ /livez/    │   │              │                  │
                      │  │ readyz     │   └──────────────┘                  │
                      │  │            │                                     │
                      │  │ Service Bindings                                 │
                      │  │  ├── AUTH ──────────▶ Auth Worker               │
                      │  │  │                     │ D1 (users table)        │
                      │  │  │                     │ DO rate limiter         │
                      │  │  ├── KV (TEST_KV) ────▶ Cloudflare KV           │
                      │  │  ├── D1 (analytics) ──▶ D1 Database             │
                      │  │  ├── Queue ───────────▶ Queue Consumer          │
                      │  │  ├── AI ──────────────▶ Workers AI              │
                      │  │  └── SigV4 ───────────▶ AWS Lambda Function URL │
                      │  │                                                   │
                      │  │ Console output (JSON logs via console_log!)       │
                      │  │  └── Cloudflare Logpush ──▶ R2 / S3              │
                      └──────────────────────────────────────────────────────┘
                                      │
                                      │ Bearer Token (HMAC)
                                      │ SESSION_SECRET shared secret
                                      ▼
                         ┌───────────────────────┐
                         │ Analytics Worker       │
                         │ (bypasses gateway)     │
                         │ D1 (analytics_events)  │
                         │ /track /events /summary│
                         └───────────────────────┘
```

## Three Pillars

### Metrics (Prometheus /metrics endpoint)

In-memory counters and histograms per endpoint, exported at GET /metrics in
Prometheus text format.

**Counter**: `cloudflare_requests_total{method,path}` — incremented per request.
**Error counter**: `cloudflare_request_errors_total{method,path}` — incremented
when status >= 400.
**Summary**: `cloudflare_request_duration_ms{method,path}` — raw values with
p50/p90/p99 computed on-scrape.

```prometheus
# HELP cloudflare_requests_total Total request count
# TYPE cloudflare_requests_total counter
cloudflare_requests_total{method="GET",path="/health"} 1423

# HELP cloudflare_request_errors_total Total error count
# TYPE cloudflare_request_errors_total counter
cloudflare_request_errors_total{method="GET",path="/health"} 0

# HELP cloudflare_request_duration_ms Request duration in milliseconds
# TYPE cloudflare_request_duration_ms summary
cloudflare_request_duration_ms{method="GET",path="/health"} 1423
cloudflare_request_duration_ms_sum{method="GET",path="/health"} 2846.5
cloudflare_request_duration_ms{method="GET",path="/health",quantile="0.5"} 1.2
cloudflare_request_duration_ms{method="GET",path="/health",quantile="0.9"} 3.1
cloudflare_request_duration_ms{method="GET",path="/health",quantile="0.99"} 8.7
```

**Storage**: in-process `MetricsRegistry` with `Arc<EndpointMetrics>` sharing.
`register()` returns an `Arc` — callers record via the Arc, export iterates the
registry. WASM-single-threaded: `Mutex<Vec<Arc<...>>>` for the registry list.

**Reset on deploy**: Every worker deployment resets all counters. This is
intentional — Workers are ephemeral, and a deploy is a new process. Grafana
should use `rate()` not absolute values.

### Traces (OTLP over HTTP/protobuf)

W3C `traceparent` header propagation, OTLP protobuf export to SigNoz.

**TraceContext** (`shared/src/observability/trace_context.rs`):
- 16-byte trace ID (32 hex chars), 8-byte span ID (16 hex chars)
- `trace_flags` = `01` (always sampled)
- Parses incoming `traceparent` header (W3C format)
- Generates fresh context if no header present
- Injects `traceparent` into responses and downstream requests

**Export** (`shared/src/observability/otel.rs`):
- `export_span()` builds OTLP `ExportTraceServiceRequest` protobuf
- POSTs to `{SIGNOZ_OTEL_ENDPOINT}/v1/traces` with `Content-Type: application/x-protobuf`
- Runs inside `cx.wait_until()` (async, non-blocking, no delay to client response)

**Retry**: 3 attempts with backoff [0ms, 100ms, 300ms].

**Buffer**: 100 pending spans in `VecDeque<PendingSpan>` protected by `Mutex`.
FIFO eviction when full. On successful export, `flush_buffer()` drains all
queued spans.

**Trace attributes per span**:
- `http.method`, `http.target`, `http.status_code`, `http.route`
- `error.message` (present when status >= 400)
- `service.name` (gateway), `telemetry.sdk.name`, `telemetry.sdk.language`

```protobuf
// OTLP ExportTraceServiceRequest structure (prost-generated)
message ExportTraceServiceRequest {
  repeated ResourceSpans resource_spans = 1;
}
message ResourceSpans {
  Resource resource = 1;     // service.name, telemetry.sdk.*
  repeated ScopeSpans scope_spans = 2;
}
message ScopeSpans {
  Scope scope = 1;           // name, version
  repeated Span spans = 2;
}
message Span {
  bytes trace_id = 1;        // 16 bytes
  bytes span_id = 2;         // 8 bytes
  bytes parent_span_id = 4;  // empty = root span
  string name = 5;           // "GET /health"
  int32 kind = 6;            // 2 = Server
  fixed64 start_time_unix_nano = 7;
  fixed64 end_time_unix_nano = 8;
  repeated KeyValue attributes = 9;
  Status status = 15;        // code=1 Ok, code=2 Error
}
```

### Logs (Structured JSON)

Every request produces two log events (request start, response complete) as JSON
via `worker::console_log!()`. Format:

```json
{"timestamp":"2026-07-24T10:30:00.000Z","level":"INFO","message":"GET /health",
 "service":"gateway","trace_id":"ab12...","span_id":"cd34...",
 "method":"GET","path":"/health","status":200,"duration_ms":42}
```

**Ring buffer** (`shared/src/observability/structured_log.rs`):
- 100-entry `LogBuffer` shared via `OnceLock`
- Evicts oldest on overflow
- Exposed at GET /logs (returns last 50 entries as JSON)
- `Logger` builder pattern: `logger().info("msg").with_trace(&ctx).emit()`

**Log levels mapped from HTTP status**:
- 2xx/3xx → INFO
- 4xx → WARN
- 5xx → ERROR

**Logpush integration**: Cloudflare's dashboard captures `console_log!()` output.
Logpush can forward to R2 or S3 for long-term retention.

---

## RED Metrics Per Worker Per Route

### Gateway Worker

| Route | Method | Requests | Errors (4xx+) | Duration (p50/p90/p99) |
|-------|--------|----------|---------------|------------------------|
| `/` | GET | cloudflare_requests_total | cloudflare_request_errors_total | cloudflare_request_duration_ms |
| `/kv` | GET | same pattern | same | same |
| `/d1` | GET | same | same | same |
| `/queue` | GET | same | same | same |
| `/ai` | GET | same | same | same |
| `/health` | GET | same | same | same |
| `/livez` | GET | same | same | same |
| `/readyz` | GET | same | same | same |
| `/metrics` | GET | same | same | same |
| `/logs` | GET | same | same | same |
| `/v1/models` | GET | same | same | same |
| `/protected` | GET | same | same | same |
| `/lambda/query` | POST | same | same | same |
| `OPTIONS *` | OPTIONS | same | same | same |

Label dimensions: `method`, `path`. Every route tracked individually.

### Auth Worker

No Prometheus endpoint (separate worker, no MetricsRegistry imported). Structured
logs only for request/response. OTel traces available if SIGNOZ_OTEL_ENDPOINT
configured as env var (not yet wired; runs in fetch handler directly).

### Analytics Worker

Same as auth: no Prometheus endpoint, structured logs only. HMAC Bearer auth (no
gateway involvement). D1-backed event storage.

---

## OTel Pipeline Detail

### Span Creation Flow

```
1. Browser sends request (no traceparent → Gateway generates fresh TraceContext)
   OR upstream service sends traceparent → Gateway parses it

2. Router creates TraceContext from request headers:
   TraceContext::from_request(&req)
   - Checks "traceparent" header (W3C format)
   - Falls back to "X-Trace-Id" (legacy)
   - Generates fresh 16-byte trace_id + 8-byte span_id if absent

3. Start timestamp captured:
   let start_ms = Date::now().as_millis();

4. Request is routed to handler, response is built

5. Duration calculated, metrics recorded, log emitted

6. OTel export scheduled (non-blocking):
   cx.wait_until(async move {
       export_span(url, "gateway", &tc, None, &name, start_ms, end_ms,
                   &method, &path, status, None).await?
   })
```

### Export Detail (otel.rs)

```
export_span()
  │
  ├─▶ Decode trace_id (hex→16 bytes), span_id (hex→8 bytes)
  ├─▶ Build OTLP Span protobuf (kind=Server, status from error)
  ├─▶ Encode ExportTraceServiceRequest with prost
  ├─▶ send_with_retry(url, proto_bytes)
  │     │
  │     ├─▶ Attempt 1: send_post() → 0ms delay
  │     ├─▶ Attempt 2: send_post() → 100ms delay (if failed)
  │     ├─▶ Attempt 3: send_post() → 300ms delay (if failed)
  │     │
  │     ├─▶ SUCCESS: flush_buffer() → drain all queued spans
  │     └─▶ FAILURE: buffer span → push to VecDeque (FIFO drop at 100)
  │
  └─▶ Return Ok/Err to caller (logged to console)
```

### Collector Configuration

SigNoz collector at SIGNOZ_OTEL_ENDPOINT (default: `http://localhost:4318` for
local dev; production URL set as wrangler secret).

The collector's HTTP endpoint only accepts `application/x-protobuf`. JSON is
rejected. This is a SigNoz constraint: the OTLP HTTP receiver on :4318/v1/traces
requires protobuf content-type.

### Known Limitations

- **`cx.wait_until()` best-effort**: If the worker terminates before the wait_until
  future resolves, the span is lost. No durability guarantee.
- **Buffer in-process**: Buffered spans live in global static `Mutex<VecDeque>`.
  If the worker restarts (deploy, scale-to-zero), buffered spans are lost.
- **No gRPC**: Workers cannot open gRPC connections. HTTP/protobuf only.
- **Single span per request**: No nested child spans within the gateway. The
  auth-worker sub-request is not traced as a separate span (future work).

---

## How to Add: Developer Guide

### Add a New Metric

```rust
// In any handler that has access to the MetricsRegistry:
use crate::metrics;  // returns &'static MetricsRegistry

// Register returns Arc<EndpointMetrics> — call once, record on each request.
let ep = metrics().register("POST", "/my-new-route");
let duration_ms = ep.record(status, start_ms);
// record() increments requests counter, observes latency, increments errors
// if status >= 400. Returns duration in ms.
```

To add a new metric type (not per-endpoint):

```rust
use cloudflare_shared::observability::metrics::{Counter, Histogram};
use std::collections::BTreeMap;

let mut labels = BTreeMap::new();
labels.insert("worker".to_string(), "gateway".to_string());

let cache_hits = Counter::new("cloudflare_cache_hits_total", labels.clone());
cache_hits.inc();
// Export: call cache_hits.to_prometheus() and append to /metrics output
```

### Add a New Trace Span

```rust
use cloudflare_shared::observability::{otel::export_span, trace_context::TraceContext};
use worker::Date;

let tc = TraceContext::new();  // or child_span() from parent context
let start_ms = Date::now().as_millis();

// ... do work ...

let end_ms = Date::now().as_millis();
let collector_url = env.var("SIGNOZ_OTEL_ENDPOINT")?.to_string();

// Blocking variant (for service bindings / sub-requests in fetch handler):
export_span(&collector_url, "my-worker", &tc, None,
            "my-operation-name", start_ms, end_ms,
            "POST", "/path", 200, None).await?;

// Non-blocking variant (in gateway, after response is built):
cx.wait_until(async move {
    if let Err(e) = export_span(...).await {
        console_log!("otel export error: {}", e);
    }
});
```

### Add a New Log Event

```rust
use cloudflare_shared::observability::structured_log::Logger;

let logger = Logger::new("my-worker");

// Simple info log
logger.info("Cache miss for key xyz").emit();

// With context
logger.info("Processing queue message")
    .with_trace(&trace_ctx)
    .with_metadata("queue_size", serde_json::json!(42))
    .emit();

// Request/response helpers
logger.request("POST", "/webhook", &trace_ctx).emit();
logger.response("POST", "/webhook", 200, duration_ms, &trace_ctx).emit();
```

### Add a New Health Check

```rust
use cloudflare_shared::observability::health::HealthRegistry;
use worker::Date;

let registry = HealthRegistry::new();
registry.register("my-queue", || {
    let start = Date::now().as_millis();
    // ... check the binding ...
    let latency = Date::now().as_millis() - start;
    Ok(latency)  // or Err("connection refused")
});

// Run all checks:
let results = registry.check_all();
let (status, details) = registry.overall_status();
```

---

## Debugging Guide

### Scenario: Request returns 5xx

```
┌─ Step 1: Check /health for binding status
│  $ curl https://gateway-worker/health
│  Look for "status":"unhealthy" or individual checks with errors
│
├─ Step 2: Check /metrics for error counter
│  $ curl https://gateway-worker/metrics | grep errors_total
│  Check cloudflare_request_errors_total — which path?
│
├─ Step 3: Check /logs for recent error events
│  $ curl https://gateway-worker/logs | jq '.logs[] | select(.level=="ERROR")'
│
├─ Step 4: Check SigNoz for trace (trace_id in log output)
│  In SigNoz: Trace → Search by trace_id → look at span attributes
│
└─ Step 5: Check Cloudflare dashboard
   Workers & Pages → gateway-worker → Logs (console_log! output)
```

### Scenario: High latency on /d1

```
┌─ Check /health — D1 binding healthy?
│  If unhealthy: check D1 dashboard for query concurrency limits
│
├─ Check /metrics for /d1 latency:
│  cloudflare_request_duration_ms{path="/d1",quantile="0.99"}
│  Normal: ~10-50ms. Above 200ms = degraded.
│
├─ Check D1 query in SigNoz (trace attribute)
│  Each gateway request has span with http.target="/d1"
│
└─ Check D1 console for slow queries
   Cloudflare Dashboard → D1 → test-d1-database → Queries
```

### Scenario: Auth tokens failing

```
┌─ Check /health on gateway — AUTH binding up?
│  If auth-worker is down, all /protected requests fail 502
│
├─ Check auth-worker logs directly
│  Cloudflare Dashboard → Workers → auth-worker → Logs
│
├─ Check SESSION_SECRET is set on both workers:
│  wrangler secret list --name auth-worker
│  wrangler secret list --name analytics-worker
│  (Both must have same SESSION_SECRET)
│
├─ Verify DO rate limiter
│  If rate-limited: /login returns 429. Check DO metrics.
│
└─ Check analytics Bearer token
   Analytics uses same SESSION_SECRET for HMAC validation
   Token format: s2.<base64_payload>.<base64_sig>
```

### Scenario: OTel spans not appearing in SigNoz

```
┌─ Check SIGNOZ_OTEL_ENDPOINT env var
│  wrangler secret list --name gateway-worker
│
├─ Check collector health
│  curl http://collector:13133/ (collector health endpoint)
│
├─ Check collector logs for protobuf errors
│  Collector logs show "unsupported content type" if JSON sent
│
├─ Check ClickHouse connectivity
│  SigNoz → Settings → Health → ClickHouse
│
└─ Check cx.wait_until() timing
   If worker returns response before wait_until completes and worker
   is terminated, span is lost. This is a known limitation.
```

### Scenario: /metrics not updating

```
┌─ MetricsRegistry is in-process — deploys reset it
│  After deploy, counters start at 0. Use rate() in Grafana.
│
├─ Check that register() is being called
│  EndpointMetrics is created on first request to each route.
│  Metrics appear after first request.
│
└─ Check /metrics endpoint returns 200
   curl -v https://gateway-worker/metrics
```

---

## SLO Definitions

### SLO: Gateway Availability

```
SLO: 99% of requests return 2xx/3xx
Measurement: cloudflare_request_errors_total / cloudflare_requests_total
Window: 30 days
```

### SLO: Gateway Latency

```
SLO: 99% of requests complete under 500ms
Measurement: cloudflare_request_duration_ms{quantile="0.99"}
Window: 7 days
```

### SLO: D1 Availability

```
SLO: 99.5% of D1 queries succeed
Measurement: health check pass rate
Window: 30 days
```

### SLO: Auth Success Rate

```
SLO: 99% of /verify requests return 2xx for valid tokens
Measurement: rate of 4xx on /verify (false rejections)
Window: 7 days
```

### Burn-Rate Alerting

```
Alert: WorkerSLOFastBurn (CRITICAL)
  - Multi-window (5m AND 1h) error ratio > 0.144
  - burn_rate = 14.4× target (1% budget consumed in ~1h)
  - Severity: page

Alert: WorkerSLOSlowBurn (WARNING)
  - Multi-window (30m AND 6h) error ratio > 0.06
  - burn_rate = 6× target (1% budget consumed in ~2.4h)
  - Severity: ticket

Implementation:
  - Recording rules pre-compute per-worker error ratios at [5m/1h/30m/6h]
  - Alerts require BOTH short and long windows to exceed threshold
  - Prevents flapping from transient spikes
  - See prometheus/rules/worker-slo.yml for exact definitions
```

---

## Dashboards

### Dashboard: RED (Rate, Errors, Duration)

![RED Dashboard](../screenshots/red-metrics.jpg)

**Purpose**: Real-time request health per route.

**Panels**:

```
Panel: Request Rate
  Query: rate(cloudflare_requests_total[5m])
  Group by: path
  Type: stacked area

Panel: Error Rate
  Query: rate(cloudflare_request_errors_total[5m])
  Group by: path, status
  Type: stacked area

Panel: Error %
  Query: rate(cloudflare_request_errors_total[5m])
         / rate(cloudflare_requests_total[5m]) * 100
  Group by: path
  Alert threshold: > 5%

Panel: Latency (p50/p90/p99)
  Query: cloudflare_request_duration_ms{quantile="0.5"}
         cloudflare_request_duration_ms{quantile="0.9"}
         cloudflare_request_duration_ms{quantile="0.99"}
  Group by: path
  Type: line chart

Panel: Latency Heatmap
  Query: cloudflare_request_duration_ms{quantile=~"0.5|0.9|0.99"}
  Type: heatmap over time
```

**Grafana PromQL queries**:

```promql
# Request rate per path
rate(cloudflare_requests_total[5m])

# Error rate per path
rate(cloudflare_request_errors_total{path=~".+"}[5m])

# Error ratio
sum(rate(cloudflare_request_errors_total[5m]))
/
sum(rate(cloudflare_requests_total[5m]))
* 100

# p99 latency per path
cloudflare_request_duration_ms{quantile="0.99"}
```

### Dashboard: Dependency Health

**Purpose**: Binding health status over time.

**Panels**:

```
Panel: Overall Health Status
  Query: /health endpoint (JSON) parsed for overall status
  Type: status indicator (green/red)

Panel: Binding Latency
  Query: per-binding latency from /health checks
  Type: line chart

Panel: Check Status
  Query: per-binding status (healthy/degraded/unhealthy)
  Type: table
```

### Dashboard: Traces (SigNoz)

**Purpose**: Trace-level debugging via SigNoz UI.

**SigNoz queries**:

```
# All traces for a specific route
http.target="/d1"
Operation: GET /d1
Min Duration: 0

# Error traces
has(error.message) OR status.code = 2

# Slow traces
duration > 1000ms

# Traces by service
service.name="gateway"

# Trace waterfall for specific trace_id
trace_id="<trace_id_from_log>"
```

### Dashboard: Log Explorer

![Worker Logs Dashboard](../screenshots/worker-logs.jpg)

**Purpose**: Recent error log browsing.

**Panels**:

```
Panel: Logs by Level (pie chart)
  Query: count by level from /logs endpoint

Panel: Recent Errors (table)
  Query: /logs endpoint, filter level=ERROR
  Columns: timestamp, message, path, duration_ms

Panel: Log Volume (line chart)
  Query: rate of log events by level over time
```

---

## Runbook

### Collector Down

**Symptoms**: Span exports fail silently. `console_log!` shows `otel export
attempt 1/3 failed: ...`. No new traces in SigNoz.

**Impact**: Metrics and logs unaffected (in-process). Trace debugging degraded.

**Steps**:

```
1. Verify collector is running:
   curl http://<collector>:13133/

2. Check collector logs:
   docker logs signoz-otel-collector  # if Docker
   kubectl logs -n signoz otel-collector-*  # if Kubernetes

3. Verify ClickHouse connectivity:
   From collector container: curl clickhouse:8123/ping

4. If collector is restarting:
   - Check OOM killer (dmesg)
   - Check disk space (SigNoz fills ClickHouse ~1GB/day per 100 req/s)
   - Check ClickHouse schema version mismatch:
     Set use_new_schema: false in collector config, or upgrade query-service

5. If collector is unreachable from Workers:
   - Check network path (Workers → SigNoz collector)
   - If in different region, latency may cause timeouts
   - Verify SIGNOZ_OTEL_ENDPOINT secret value

6. Buffered spans are lost on worker restart.
   Acceptable for best-effort tracing.
```

### D1 Latency

**Symptoms**: /d1 endpoint slow (>200ms). p99 latency on RED dashboard shows
spike.

**Impact**: All workers using D1 affected (auth, analytics, gateway queue
consumer).

**Steps**:

```
1. Confirm it's D1, not the worker:
   curl /health — check D1 binding latency
   Normal: <50ms. Above 200ms = degraded.

2. Check D1 query concurrency:
   Cloudflare Dashboard → D1 → test-d1-database → Metrics
   D1 has 5 concurrent query limit on free plan.
   Queries beyond 5 are queued.

3. Check for slow queries in D1 dashboard:
   Look for queries scanning many rows (missing indexes).

4. Common issues:
   - analytics_events table growing unbounded (no TTL/purge)
   - Missing index on event_type column in GROUP BY queries
   - Sequential scan on users table in auth worker

5. Mitigation:
   a. Add index on analytics_events(event_type, created_at)
      CREATE INDEX idx_events_type_created ON analytics_events(event_type, created_at);
   b. Add LIMIT to all queries (already done for /events)
   c. Consider partitioning by time if table > 100k rows
   d. Upgrade to paid plan for higher concurrency

6. If D1 is unavailable:
   - Auth tokens still work (tokens are stateless HMAC, no DB lookup
     needed for /verify on new tokens)
   - Gateway falls back to 502 on D1 failures
   - Queue consumer retries on D1 insert failure
```

### Auth Failure Spike

**Symptoms**: 403/401 spike on `/protected`. `cloudflare_request_errors_total`
jumps for path="/protected". User complaints of access denied.

**Impact**: All protected routes inaccessible.

**Steps**:

```
1. Check auth-worker health:
   curl /health on gateway — AUTH binding shows unhealthy?
   If gateway cannot reach auth-worker, check service binding.

2. Check auth-worker logs:
   Cloudflare Dashboard → auth-worker → Logs
   Look for "Invalid or expired token" or "SESSION_SECRET not configured"

3. Check SESSION_SECRET consistency:
   wrangler secret list --name auth-worker
   wrangler secret list --name analytics-worker
   Both must have the SAME value. Changing it invalidates ALL tokens.

4. Check DO rate limiter:
   If /login returns 429, users cannot refresh tokens.
   Rate limit is 5/min/ip for /register, 10/min/ip for /login.
   Check DO metrics in Cloudflare dashboard.

5. Check for expired tokens:
   Token format: s2.<base64_payload>.<base64_sig>
   Decode payload: echo "<payload>" | base64 -d
   Check exp field (Unix timestamp). Default: 7 days.

6. Emergency override:
   Deploy auth-worker with DO rate limiter check bypassed
   (comment out check_rate_limit call in routes.rs)
```

### Metrics Spikes (Traffic Surge)

**Symptoms**: Request rate 10x normal. Error rate normal proportionally. No
degradation.

**Impact**: Metrics dashboard scales. Workers auto-scale (no capacity issues).

**Steps**:

```
1. Identify source path in RED dashboard:
   Which path is surging? /ai? /d1?

2. Check if it's legitimate traffic or abuse:
   - Check CF-Connecting-IP distribution
   - Check User-Agent distribution
   - Check geo distribution

3. If abuse:
   - Deploy rate limiting at Cloudflare WAF level
   - Add IP-based rate limiting to gateway
   - Block suspicious User-Agents

4. If legitimate:
   - Monitor p99 latency — do not let surge degrade quality
   - Consider upgrading D1 plan if surge is D1-heavy
   - No action needed (Workers scale transparently)
```

### Queue Backlog

**Symptoms**: Queue consumer not keeping up. Messages accumulating in queue.

**Impact**: Analytics events delayed. No data loss (queue retains messages up to
4 days or 100k messages).

**Steps**:

```
1. Check queue consumer logs in gateway:
   Cloudflare Dashboard → gateway-worker → Logs
   Look for "Received N messages from queue" and individual D1 insert errors.

2. If D1 is the bottleneck:
   - D1 has 5 concurrent write limit
   - Queue batch size is 10 (see wrangler.toml)
   - Consumer takes ~2s per batch if D1 is slow
   - Reduce max_batch_size or increase visibility timeout

3. If consumer is crashing:
   - Check for malformed messages (garbage JSON)
   - Consumer acks malformed messages (does not retry)
   - Check queue handler error handling
```

### New Worker Missing Observability

**Symptoms**: New worker deployed but no metrics, no traces in SigNoz, no health
check endpoint.

**Fix**:

```
1. Add MetricsRegistry (static OnceLock) to lib.rs:
   pub fn metrics() -> &'static MetricsRegistry {
       static METRICS: OnceLock<MetricsRegistry> = OnceLock::new();
       METRICS.get_or_init(MetricsRegistry::new)
   }

2. Add /metrics route handler:
   "/metrics" => {
       let prom = metrics().export_prometheus();
       let resp = Response::from_bytes(prom.as_bytes().to_vec())?;
       resp.headers().set("content-type", "text/plain; charset=utf-8")?;
       Ok(resp)
   }

3. Add TraceContext::from_request() at entry point:
   let ctx = TraceContext::from_request(&req)?;

4. Add Logger + LogBuffer for structured logging:
   pub fn logger() -> &'static Logger { ... }
   pub fn log_buffer() -> &'static LogBuffer { ... }

5. Add /logs route:
   "/logs" => json_response(200, &json!({"logs": log_buffer().recent(50)}))

6. Wire OTel export (if SigNoz endpoint is available):
   cx.wait_until(async { export_span(...).await });

7. Add HealthRegistry with binding checks:
   Register checks for each binding, expose at /health and /readyz.
```

---

## Key Metrics Exported

### Gateway Worker (Prometheus /metrics)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `cloudflare_requests_total` | Counter | method, path | Total requests |
| `cloudflare_request_errors_total` | Counter | method, path | Requests with status >= 400 |
| `cloudflare_request_duration_ms` | Summary | method, path, quantile | Latency with p50/p90/p99 |

### Gateway Worker (OTLP trace attributes)

| Attribute | Type | Example |
|-----------|------|---------|
| `http.method` | string | `GET` |
| `http.target` | string | `/d1` |
| `http.status_code` | int | `200` |
| `http.route` | string | `/d1` |
| `error.message` | string | `D1 query failed` (if error) |
| `service.name` | string | `gateway` |
| `telemetry.sdk.name` | string | `cloudflare-lab` |

### Gateway Worker (Health Check)

| Check | Type | Status |
|-------|------|--------|
| kv | KV binding probe | healthy/degraded/unhealthy |
| d1 | D1 binding probe | healthy/degraded/unhealthy |
| queue | Queue binding probe | healthy/degraded/unhealthy |
| ai | Workers AI binding probe | healthy/degraded/unhealthy |
| auth | Service binding probe | healthy/degraded/unhealthy |

### All Workers (Structured Log Schema)

| Field | Type | Optional | Description |
|-------|------|----------|-------------|
| timestamp | string | no | ISO 8601 |
| level | string | no | DEBUG, INFO, WARN, ERROR |
| message | string | no | Log message |
| service | string | no | worker name |
| trace_id | string | yes | 32 hex chars |
| span_id | string | yes | 16 hex chars |
| duration_ms | number | yes | Request duration |
| method | string | yes | HTTP method |
| path | string | yes | HTTP path |
| status | number | yes | HTTP status code |
| error | string | yes | Error details |
| metadata | object | yes | Arbitrary key-value |

---

## Configuration Reference

### Environment Variables

| Variable | Required | Workers | Description |
|----------|----------|---------|-------------|
| `SIGNOZ_OTEL_ENDPOINT` | No | gateway | SigNoz collector URL (e.g. `http://localhost:4318`). If unset, OTel export is skipped. |
| `LOKI_ENDPOINT` | No | gateway | Loki HTTP push URL (e.g. `http://localhost:3100`). If unset, log export is skipped. |
| `LOKI_TENANT_ID` | No | gateway | Loki tenant ID for multi-tenant setups. Omit for single-tenant. |
| `SESSION_SECRET` | Yes | auth, analytics | HMAC signing key (must match across workers). Changed → all tokens invalidated. |
| `LAMBDA_URL` | Yes | gateway | AWS Lambda Function URL for SigV4 proxy. |
| `AWS_ACCESS_KEY_ID` | Yes | gateway | AWS IAM access key for SigV4 signing. |
| `AWS_SECRET_ACCESS_KEY` | Yes | gateway | AWS IAM secret key for SigV4 signing. |

### Secrets Management

```bash
# Set a secret (run from worker's directory)
cd workers/gateway && echo "https://otel-collector:4318" | wrangler secret put SIGNOZ_OTEL_ENDPOINT --name gateway-worker

# Verify secrets
wrangler secret list --name gateway-worker

# Remove a secret
wrangler secret delete SIGNOZ_OTEL_ENDPOINT --name gateway-worker
```

**Important**: `wrangler secret put` reports "Success" even from the wrong
directory. Always verify with `wrangler secret list`.

---

## Known Issues

See README.md for full list. Observability-specific:

- **OTLP JSON rejected**: SigNoz collector HTTP endpoint only accepts
  `application/x-protobuf`. This is by design (protobuf is smaller and faster).
- **No retry in cx.wait_until()**: If the collector is down when the worker
  ends, the span is lost. The in-memory buffer (100 spans) provides best-effort
  delivery for transient failures within a single request.
- **LogBuffer overwrite**: 100-entry ring buffer is shared via `OnceLock`. Under
  high concurrency, entries are overwritten before you read them via /logs.
- **check_bindings() wall time**: Each health check runs sequentially. Total
  latency includes all prior checks. Use for binary health (>500ms thresholds),
  not per-binding SLA measurement.
- **Metrics reset on deploy**: `MetricsRegistry` is in-process. Every deploy
  resets counters. Always use `rate()` in Grafana queries.
- **No nested span for auth sub-request**: Gateway calls auth-worker via service
  binding but does not create a child span for it. The auth call's latency is
  included in the parent span duration. Future improvement: create child span
  via `tc.child_span()` and export separately.

---

## Future Improvements

- **Child span for auth-worker sub-request**: Create a child span and export it
  to trace the auth call independently.
- **Push metrics to SigNoz**: Export Prometheus metrics to SigNoz for unified
  dashboarding instead of scraping /metrics.
- **Log correlation**: Push structured logs to SigNoz via OTLP logs signal.
- **Distributed tracing across all workers**: Wire OTel export in auth-worker
  and analytics-worker (currently only gateway exports spans).
- **Real user monitoring (RUM)**: W3C traceparent header from browser to
  correlate frontend performance with server traces.
- **Sampling**: Currently 100% sampling. Add head-based sampling for high-traffic
  routes.
