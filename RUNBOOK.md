# RUNBOOK — Cloudflare Workers + AWS Lambda Observability
#
# Usage: first responder guide for alert fire-fighting.
# Every alert maps to: symptom → diagnose → resolve.
#
# Service boundaries:
#   Cloudflare Workers (auth/gateway/analytics) → Prometheus metrics + Loki logs
#   AWS Lambda (devops-api)                     → CloudWatch metrics + X-Ray traces
#   Shared OTel pipeline                       → SigNoz (OTLP via protobuf)

# ---------------------------------------------------------------------------
# ALERT: Worker SLO burn-rate (5m / 30m)
# ---------------------------------------------------------------------------
# Symptom:  pagerduty/webhook alert — "WorkerErrorBudgetBurn"
# Severity: CRITICAL

## Diagnose
# 1. Check worker metrics dashboard (Grafana):
#    - Error rate per route:   `rate(cloudflare_request_errors_total[5m])`
#    - P99 latency per route:  `cloudflare_request_duration_ms{quantile="0.99"}`
#    - Compare to SLO target:  99% success, p99 < 500ms
# 2. Check Loki logs for error trace_ids:
#    `{service="gateway"} |= "ERROR" | json | status >= 500`
# 3. Check if downstream dependencies are healthy:
#    - /health on auth, gateway, analytics workers
#    - D1 database latency via /health endpoint
#    - Lambda Function URL health

## Resolve
# - Auth 5xx:  check SESSION_SECRET rotation, D1 table bootstrap
# - Gateway 5xx: check service binding config, KV namespace availability
# - D1 timeout: verify query pattern (missing index, pagination)
# - Lambda 5xx: see "ALERT: Lambda Error Rate" below
# - If SLO burn rate persists → rollback last deploy (see Rollback section)

## Rollback
#    wrangler deploy --version <previous-version>
#    # or rollback via Cloudflare Dashboard:
#    # Workers & Pages → <worker> → Deployments → Rollback

# ---------------------------------------------------------------------------
# ALERT: Lambda Error Rate
# ---------------------------------------------------------------------------
# Symptom:  CloudWatch alarm — "devops-api-{env}-errors" firing
# Severity: CRITICAL

## Diagnose
# 1. CloudWatch → Logs → /aws/lambda/devops-api-{env} → Filter ERROR
#    grep '"level":"ERROR"' | jq '.message'
# 2. X-Ray traces → Service map → devops-api → View traces with errors
#    URL: https://console.aws.amazon.com/xray/home#/service-map
# 3. Check Lambda Function URL IAM credentials rotation
#    - Gateway worker signs requests via SigV4
#    - Stale AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY → 403

## Resolve
# - 403 Forbidden → update secrets in gateway worker:
#   `echo "new-key" | wrangler secret put AWS_ACCESS_KEY_ID --name gateway-worker`
# - 500 Internal → check Lambda logs for panic/traceback
# - Timeout → increase lambda.memory_size or review query perf
# - Concurrency → increase reserved concurrency or add DLQ

# ---------------------------------------------------------------------------
# ALERT: Lambda Duration p95 > 5s
# ---------------------------------------------------------------------------
# Symptom:  CloudWatch alarm — "devops-api-{env}-duration-p95"
# Severity: WARNING

## Diagnose
# 1. X-Ray trace detail: find the slowest subsegment
#    - Downstream HTTP call? → check worker gateway latency
#    - DB query? → D1 query performance (Cloudflare dashboard)
#    - CPU-bound? → increase Lambda memory_size (CPU scales with memory)
# 2. CloudWatch → Metrics → Lambda Duration p50/p95/p99 trend

## Resolve
# - Increase Lambda memory_size in aws/lambda.tf (128 → 256 or 512)
# - Add connection pooling for D1 queries
# - Review cold start: provisioned concurrency if sustained p99 > 5s
# - ADOT layer: add aws-otel-lambda layer for enhanced span detail

# ---------------------------------------------------------------------------
# ALERT: D1 Query Latency
# ---------------------------------------------------------------------------
# Symptom:  Prometheus alert — "HighD1QueryLatency"
# Severity: WARNING

## Diagnose
# 1. Check D1 query latency via gateway metrics:
#    `cloudflare_request_duration_ms{worker="gateway",path="/d1",quantile="0.99"}`
# 2. Review query patterns: missing index? full table scan?
# 3. Check D1 storage usage: `wrangler d1 info <db-name>`

## Resolve
# - Add WHERE clause index (D1 automatically indexes INTEGER PRIMARY KEY)
# - Paginate large result sets with LIMIT/OFFSET
# - Reduce query frequency with in-memory caching (Duration Object)
# - Upgrade D1 plan if storage > 1GB (paid tier)

# ---------------------------------------------------------------------------
# ALERT: Auth Failure Spike
# ---------------------------------------------------------------------------
# Symptom:  Grafana alert — "HighAuthFailureRate" or 401 spike
# Severity: WARNING → CRITICAL if > 50% of requests

## Diagnose
# 1. Check gateway logs for 401 patterns:
#    `{service="gateway"} |= "401"`
# 2. Verify session secret not recently rotated:
#    - SESSION_SECRET change invalidates all HMAC tokens
#    - Check `wrangler secret list --name auth-worker` for update timestamp
# 3. Check rate limiter DO status:
#    - Rate limiter DO binding must be connected
#    - If disconnected → rate limiting disabled → potential brute force

## Resolve
# - Old tokens: users re-authenticate → /login returns new HMAC token
# - Rate limiter DO: deploy rate-limiter worker, verify binding
# - Brute force: temporarily increase rate limit window, investigate IP

# ---------------------------------------------------------------------------
# Incident Response Escalation
# ---------------------------------------------------------------------------
# Severity | Response Time | Escalate To
# ---------|---------------|------------
# CRITICAL | 15 min        | on-call engineer → team lead → eng manager
# WARNING  | 1 hour        | on-call engineer
# INFO     | next business day | Slack #observability channel

# ---------------------------------------------------------------------------
# Observability Stack Access
# ---------------------------------------------------------------------------
# Prometheus:   http://localhost:9090  (docker compose up / podman compose up)
# Grafana:      http://localhost:3000  (anonymous admin — no login required)
# SigNoz:       http://localhost:8080  (admin@signoz.com / admin123)
# CloudWatch:   https://console.aws.amazon.com/cloudwatch/home
# X-Ray:        https://console.aws.amazon.com/xray/home
# wrangler dev: http://localhost:8788  (auth worker local)