# Testing

## Test Suites

| Suite | Count | Command | Notes |
|-------|-------|---------|-------|
| Unit (wasm) | 92 (12 suites) | `cargo test --workspace` | 92 pass — all crates: cloudflare-shared (unit+integ), gateway, auth, analytics |
| Integration (shell) | ~50 assertions | `make test-integration` | 7 scripts: gateway, auth, analytics, observability, lambda-worker, lib.sh, runner |
| SLO rules | 10+ tests | `make prom-test` | Promtool unit tests for `worker-slo.yml` burn-rate alerts |
| Load test | Variable | `make load-test` | k6 script for SLO validation under traffic |

## Unit Tests (Rust, 92 passing)

### cloudflare-shared (6 integration tests)

| File | Count | What it tests |
|------|-------|---------------|
| `shared/tests/obs_health.rs` | — | Dependency health, status propagation |
| `shared/tests/obs_loki.rs` | — | Loki push encoding, URL format, protobuf fields |
| `shared/tests/obs_metrics.rs` | — | Histogram quantiles, prometheus format, label ordering |
| `shared/tests/obs_otlp.rs` | — | OTLP protobuf roundtrip (all 12 span fields) |
| `shared/tests/obs_structlog.rs` | — | LogBuffer, LogEvent, structured serialization |
| `shared/tests/obs_trace_context.rs` | — | W3C traceparent parse/format, edge cases |

### Workers (inline `#[cfg(test)]`)

| Crate | Location | What it tests |
|-------|----------|---------------|
| gateway-worker | `workers/gateway/src/aws_sigv4.rs` | SigV4 signing, canonical request |
| auth-worker | `workers/auth/src/lib.rs` | — (wasm-dependent, runs in CI) |
| analytics-worker | `workers/analytics/src/lib.rs` | — (wasm-dependent, runs in CI) |

## Integration Tests (Shell, 7 scripts)

Run against local wrangler dev servers. Boots all workers, runs HTTP-level assertions.

```
tests/integration/
├── run-all.sh          — Orchestrator: boots/wait/teardown workers, runs suites
├── lib.sh              — Assertion library (pass/fail, assert_status, assert_header, assert_json_field)
├── gateway.sh          — 10+ tests: /health, CORS, 404, X-Request-Id, /metrics, /logs
├── auth.sh             — 6+ tests: register (201/409/400), login (200/401), verify (200/401), rate-limit
├── analytics.sh        — 6+ tests: /track, /events, /summary with Bearer auth
├── observability.sh    — 6+ tests: /metrics prom format, /logs ring buffer, trace header pass-through, CORS
└── lambda-worker.sh    — Lambda round-trip tests (requires LAMBDA_URL, skips if unset)
```

### Usage

```bash
# All suites (boots gateway:8787, auth:8788, analytics:8789)
bash tests/integration/run-all.sh

# Specific suites only
bash tests/integration/run-all.sh --only gateway,auth

# With D1 persistence between runs
PERSIST=1 bash tests/integration/run-all.sh

# Skip binding-dependent tests (no D1/KV/DO)
SKIP_NO_BINDINGS=1 bash tests/integration/run-all.sh

# Via Make (recommended)
make test-integration
```

## SLO Rule Tests (Promtool)

Validates SLO burn-rate alert rules against expected metric scenarios.

```bash
make prom-test
# => promtool test rules prometheus/rules/tests/worker-slo.test.yml
```

Rules file: `prometheus/rules/worker-slo.yml`
Tests file: `prometheus/rules/tests/worker-slo.test.yml`
SLO targets: 99.9% success rate, p99 latency < 2000ms (worker), < 5000ms (Lambda)

## Load Test (k6)

```bash
make load-test
# => k6 run k6/load.js
```

## Test Structure (per crate)

```
cloudflare-lab/
├── shared/
│   ├── src/observability/        — Inline tests in otel.rs, trace_context.rs
│   └── tests/                    — Integration tests (6 files)
├── workers/
│   ├── gateway/src/              — Inline tests in aws_sigv4.rs
│   ├── auth/src/lib.rs           — Inline tests (wasm runtime needed)
│   ├── analytics/src/lib.rs      — Inline tests (wasm runtime needed)
│   └── rate-limiter/src/lib.rs   — —
├── tests/
│   └── integration/              — Integration test scripts (7 files)
├── prometheus/rules/tests/       — SLO test YAML
└── k6/                           — Load test script
```

## Running Everything

```bash
# Full test suite (unit + integration + SLO + CLI)
make test-all

# Unit tests only
make test

# Integration tests only (boots workers, needs npx+jq+curl)
make test-integration

# SLO rule validation
make prom-test
```