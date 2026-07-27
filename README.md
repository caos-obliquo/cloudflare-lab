# Cloudflare Lab

CF Workers multi-cloud portfolio: Rust Workers (auth/gateway/analytics), AWS Lambda crate, Terraform IaC, OTel/SigNoz observability, LocalStack for local AWS dev.

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
