# Cloudflare Lab

Demo-ready observability portfolio for Mid-Cloud Observability Engineer role. Rust Workers (auth/gateway/analytics), AWS Lambda, Terraform IaC, OTel/SigNoz/Prometheus/Grafana dashboards, CloudWatch/X-Ray/ADOT tracing, SLO burn-rate alerts, CI/CD with integration tests, cost monitoring, and disaster recovery strategy.

## Architecture

```
Browser ──▶ Gateway Worker ──┬── Service Binding ──▶ Auth Worker (D1+DO)
                             │── KV                   Analytics Worker (D1)
                             │── D1                   Rate Limiter (DO)
                             │── Queue
                             │── Workers AI
                             │── SigV4 ──▶ AWS Lambda (devops-api)
                             │── OTLP  ──▶ SigNoz Collector ──▶ ClickHouse
                             └── EventBridge (LocalStack)
```

## Workers

### Gateway (`workers/gateway`) — central router + observability hub

| Route | Method | Binding |
|-------|--------|---------|
| `/` | GET | — |
| `/kv` | GET | KV |
| `/d1` | GET | D1 |
| `/queue` | GET | Queues |
| `/ai` | GET | Workers AI |
| `/health` | GET | all bindings health check |
| `/livez` | GET | — |
| `/readyz` | GET | — |
| `/metrics` | GET | in-memory Prometheus |
| `/logs` | GET | in-memory ring buffer |
| `/v1/models` | GET | — |
| `/protected` | GET | auth-worker service binding |
| `/lambda/query` | POST | SigV4 -> Lambda Function URL |
| `OPTIONS *` | — | CORS |

### Auth (`workers/auth`) — HMAC stateless tokens + pbkdf2

| Route | Method | Rate Limit |
|-------|--------|------------|
| `/register` | POST | 5/min/IP |
| `/login` | POST | 10/min/IP |
| `/verify` | GET | — |
| `/me` | GET | — |

### Analytics (`workers/analytics`) — event tracking on D1

| Route | Method | Auth |
|-------|--------|------|
| `/track` | POST | Bearer |
| `/events` | GET | Bearer |
| `/summary` | GET | Bearer |

### Rate Limiter (`workers/rate-limiter`) — DO atomic counter

POST `{"limit":N,"window":T}` → `{"allowed":bool,"remaining":N}`

### Lambda (`lambda/devops-api`) — Rust provided.al2023

| Route | Method |
|-------|--------|
| `/health` | GET |
| `/config` | GET |
| `/workers/query` | POST |
| `/workers/register` | POST |
| `/d1/query` | POST |

## Quick Start

```bash
# Prerequisites
rustup target add wasm32-unknown-unknown
cargo install worker-build
npm install -g wrangler
terraform -v # >= 1.5

# Cloudflare infra
cp terraform.tfvars.example terraform.tfvars
terraform init && terraform apply

# Secrets (run from worker dir!)
cd workers/auth && echo "secret" | wrangler secret put SESSION_SECRET --name auth-worker
cd workers/gateway && wrangler secret put LAMBDA_URL --name gateway-worker
cd workers/gateway && wrangler secret put AWS_ACCESS_KEY_ID --name gateway-worker
cd workers/gateway && wrangler secret put AWS_SECRET_ACCESS_KEY --name gateway-worker

# Deploy
make build-all
make deploy-all

# Lambda
cd aws && terraform init && terraform apply
bash scripts/deploy-localstack.sh

# Test
bash tests/integration/lambda-worker.sh
```

## Observability

Multi-layer observability across Cloudflare Workers + AWS Lambda.

### OTel Pipeline (Workers → SigNoz)

```
Worker (Rust WASM) ──OTLP/protobuf──▶ SigNoz Collector ──▶ ClickHouse
                     port 4318              │
                                            └──▶ Prometheus ──▶ Grafana
```

- OTLP span exporter: prost/protobuf encoding, HTTP transport, retry + in-memory buffer
- W3C traceparent propagation across all services (gateway → auth/analytics/lambda)
- Loki structured log exporter (JSON over HTTP, optional)
- In-memory Prometheus metrics endpoint (`/metrics`)
- Dependency health checks (`/health`, `/livez`, `/readyz`)

### Prometheus / Grafana

| Asset | Description |
|-------|-------------|
| `prometheus/prometheus.yml` | Scrape targets: SigNoz OTLP, worker metrics endpoint |
| `prometheus/rules/worker-slo.yml` | SLO burn-rate alerts (5m/30m). 99.9% success, p99 < 2s |
| `tests/worker-slo.test.yml` | SLO rule validation (make prom-test) |
| `grafana/dashboards/worker-red.json` | RED metrics dashboard (Rate/Errors/Duration) |
| `grafana/dashboards/loki-logs.json` | Logs dashboard with trace_id correlation |
| `k6/load.js` | Load test script for SLO validation |
| `scripts/validate-dashboards.sh` | Dashboard JSON validation |

Validate: `make prom-test` (promtool check rules + unit test)

### AWS CloudWatch / X-Ray / ADOT (Lambda)

| Feature | Terraform Resource |
|---------|-------------------|
| Dashboard | `aws/cloudwatch.tf` — invocations, errors, p50/p95/p99 duration |
| Alarms | Lambda errors > 0, p95 > 5s, throttles > 0, ERROR log count |
| X-Ray tracing | `lambda.tf`: `tracing_config { mode = "Active" }` |
| ADOT layer | aws-otel-collector-amd64 sidecar for enhanced metrics |
| IAM permissions | `AWSXRayDaemonWriteAccess` + `cloudwatch:PutMetricData` |

### SLOs & Runbooks

- SLO targets: 99.9% success rate, p99 latency < 2000ms (worker), < 5000ms (Lambda)
- Burn-rate alerts fire when 5m or 30m error budget is consumed
- See [RUNBOOK.md](./RUNBOOK.md) for alert fire-fighting procedures:
  - Worker error budget burn
  - Lambda error rate / duration
  - D1 query latency
  - Auth failure spike

### Distributed Trace Flow

```
Browser ──▶ Gateway Worker ──▶ Auth/Analytics (traceparent)
                │──▶ Lambda (traceparent via SigV4 headers)
                └──▶ SigNoz OTLP (spans)
```

Every service extracts or creates a W3C `traceparent` header, passes it to downstream calls, and echoes it in responses. Correlate logs across services via `trace_id`.

### Local Dev

```bash
docker compose up -d      # Prometheus, Grafana, SigNoz, LocalStack
make prom-test            # validate SLO rules
cd workers/auth && wrangler dev --port 8788  # local auth worker
```

## Disaster Recovery

### D1 Database Backup

D1 has no built-in export. Back up via wrangler:

```bash
# Manual backup
wrangler d1 dump test-d1-database --remote > backups/d1-$(date +%Y%m%d).sql

# Auto-backup (cron, deploy trigger, or GitHub Action)
# Recommended: schedule via GitHub Actions weekly
```

Backup strategy:
- **Frequency**: weekly full dump, daily via `--no-data` (schema-only for drift detection)
- **Retention**: 30 days local, 90 days S3/GitHub artifacts
- **Restore**: `cat backup.sql | wrangler d1 execute test-d1-database --remote --file=-`

### Cross-Region Lambda Failover

Lambda deployment is single-region in this setup. For DR:

1. **Secondary region stack**: duplicate `aws/` Terraform in `us-east-2` (or preferred failover region)
2. **Route53 failover**: DNS failover with health checks on Lambda Function URL
3. **Worker circuit breaker**: gateway worker detects primary Lambda failure (5xx, timeout) → retry secondary URL
4. **Data replication**: D1 is single-region; for DR, use D1 replication (GA) or periodic export to S3

### Worker Multi-Region

Workers are Cloudflare-global by default (no region concept). Key DR properties:
- **Stateless**: auth tokens are HMAC-signed (no session store dependency for validation)
- **D1 single-region**: D1 queries fail if primary region is down. Mitigation: D1 replication when available, or fallback to read-only KV cache
- **Queue retry**: failed queue messages retry automatically (configurable `max_retries`)

## Known Issues

### Terraform v5
- `cloudflare_workers_secret` resource does not exist. Use `wrangler secret put`.
- `cloudflare_workers_route` uses `script` (not `script_name`).
- `cloudflare_zone_setting` uses `setting_id` (not `setting`).
- Logpush `ownership_challenge` string not exported — cannot automate `logpush_job`.
- Renaming resources = destroy (workspace prefix for NEW resources only).

### WASM
- `std::time::SystemTime::now()` panics. Use `js_sys::Date::now()`.
- `ring::rand::SystemRandom` unavailable. Use `getrandom` with `js` feature.
- D1 pagination: `LIMIT x OFFSET y` only. No cursor.

### Durable Objects
- Free tier: `new_sqlite_classes` only. `new_classes` requires Paid ($5/mo).
- Cross-worker DO refs need `script_name` in binding. Target worker owns the class.

### D1 Multi-Account
- Same D1 name in two accounts → wrong binding on deploy. Fix: use `database_name` only (no `database_id`). Verify via `wrangler d1 info <name> --json` from target worker dir.

### SigNoz
- Collector/query-service version mismatch → schema conflict. Set `use_new_schema: false` or upgrade.
- OTLP JSON rejected over HTTP. Must use `application/x-protobuf`.
- No retry in `cx.wait_until()` — silently dropped if collector down. Best-effort only.
- Login: `admin@signoz.com` / `admin123`.

### Secrets
- `wrangler secret put` reports "Success" even from wrong directory. Always run from worker dir. Verify: `wrangler secret list --name <worker>`.

### Gateway
- `LogBuffer`: 100-entry ring buffer shared via `OnceLock`. High concurrency overwrites entries.
- `check_bindings()` latency includes prior checks' wall time. OK for >0.5s detection, not per-binding SLA.
