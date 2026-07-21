# Cloudflare Terraform + Workers + AWS Lambda

Lab project: Cloudflare Workers (Rust) with Terraform management, AWS Lambda integration via IAM SigV4.

## Features

### Auth Worker (workers/auth)
- POST /register - user registration, input validation (username 3-32 alphanumeric, password 8-128 chars), rate limited 5/min/IP
- POST /login - credential verification, SHA256->pbkdf2 automatic migration on login, rate limited 10/min/IP
- GET /verify - token validation via KV lookup
- GET /me - current user info from token
- Session tokens: CSPRNG (getrandom), format sess_<32hex>, 1h TTL
- Passwords: pbkdf2-sha512, 100k iterations, random 16-byte salt

### Gateway Worker (workers/gateway)
- GET /health - binding readiness (KV/D1/Queue/AI/Auth)
- GET /livez - liveness probe
- GET /readyz - readiness probe with per-binding checks
- GET /kv, /d1, /queue, /ai - binding-specific handlers
- POST /queue - send message to queue consumer (inserts into D1 analytics_events)
- GET /protected - proxied auth via auth-worker service binding
- POST /lambda/query - proxy to AWS Lambda with IAM SigV4 signing
- GET /v1/models - list available AI models

### Analytics Worker (workers/analytics)
- GET /events - paginated event list (limit/cursor), Bearer auth required
- POST /track - create event, Bearer auth required
- GET /summary - event summary, Bearer auth required
- Idempotency via X-Idempotency-Key header

### Shared Crate (shared/)
- crypto: pbkdf2 hashing, legacy SHA256 migration
- bootstrap: auto-create D1 tables on startup
- tracing: X-Request-Id generation and propagation
- response: standard JSON error format (status/error/code/request_id)
- error: typed AppError hierarchy (Kv, D1, Queue, Ai, Binding, NotFound, Unauthorized)
- bindings: EnvBindings struct for typed binding access

### AWS Lambda (lambda/devops-api)
- Rust custom runtime (bootstrap binary, provided.al2023)
- Function URL with AWS_IAM auth
- Routes: GET /health, GET /config, POST /workers/query, POST /workers/register, POST /d1/query
- Communication with Workers via HTTPS (gateway-worker proxies requests with SigV4 signing)

## Knowns (gotchas, quirks, decisions)

### Terraform (Cloudflare Provider ~>5.0)

1. **No `cloudflare_workers_secret` resource.** v5 provider has no resource for worker env secrets. Use `wrangler secret put` instead.

2. **Logpush ownership challenge blocks TF automation.** `cloudflare_logpush_ownership_challenge` resource exposes only `filename` and `valid`. No `ownership_challenge` string export - cannot wire `cloudflare_logpush_job.ownership_challenge` in pure TF.

3. **Workspace prefix on existing resources = destroy.** Renaming existing KV/D1/R2/Queue/AI resources forces recreation. R2 bucket holds TF state. `local.env` prefix applied only to NEW resources.

4. **`cloudflare_workers_route` uses `script` attribute (v5).** Not `script_name`.

5. **`cloudflare_zone_setting` uses `setting_id` attribute (v5).** Not `setting`.

6. **`ruleset` `action_parameters.overrides` requires `enabled` + `sensitivity_level` together.** Both must be present.

7. **R2 remote state backend** needs `skip_credentials_validation = true`, `skip_requesting_account_id = true`, `skip_s3_checksum = true`.

### Rust Workers (Rust WASM)

8. **`ring::rand::SystemRandom` not available on `wasm32-unknown-unknown`.** Use `getrandom` with `js` feature for CSPRNG. Ring pbkdf2/hmac/sha256 works on WASM.

9. **`ring` 0.17 compiles for WASM.** Verified via `cargo check --target wasm32-unknown-unknown`.

10. **D1 pagination via OFFSET, not cursor.** Cloudflare D1 supports `LIMIT x OFFSET y`. No native cursor.

11. **`worker-build` reinstall every deploy.** Fix: pre-install `worker-build`, change command to `worker-build --release` (no `cargo install`). Saves ~3-5min per build.

12. **`worker::Error` has `From<&str>` impl.** `Err(Error::from("msg"))` works directly.

### AWS Lambda

13. **Lambda custom runtime binary must be named `bootstrap`.** `Cargo.toml` needs `[[bin]] name = "bootstrap"`.

14. **No `aws4fetch` crate - implement SigV4 manually.** Use `ring::hmac` + `ring::digest` for HMAC-SHA256 key derivation.

15. **Lambda Function URL with AWS_IAM auth returns 403 for unsigned requests.** Correct behavior. Test with `awscurl` or Python `requests-aws4auth`.

### EventBridge (LocalStack)

16. **EventBus → Rule (filter) → Target (Lambda).** Async decoupling: Gateway worker POSTs events to EventBridge, rule routes to Lambda.

17. **LocalStack only.** EventBridge runs inside LocalStack, not Cloudflare. See `scripts/deploy-localstack.sh` for full setup.

18. **Manual test.** Curl sends `PutEvents` with `X-Amz-Target` header to LocalStack → rule matches → Lambda receives event.

## Project Structure

```
cloudflare-lab/
├── provider.tf           # Cloudflare provider + R2 backend
├── variables.tf          # variables with env validation
├── terraform.tfvars      # actual secrets (gitignored)
├── compute.tf            # KV, D1, R2, Queue, AI Gateway
├── firewall.tf           # custom WAF rule (/wp-admin block)
├── firewall_managed.tf   # Cloudflare Managed + OWASP rulesets
├── dns.tf                # zone data source + 4 DNS records
├── routes.tf             # 3 worker routes (gateway/auth/analytics)
├── settings.tf           # zone-level SSL/security settings
├── outputs.tf            # KV/D1/R2/Queue/AI IDs
├── aws/
│   ├── provider.tf       # AWS provider (separate state)
│   ├── variables.tf      # region, function_name, worker URLs
│   ├── iam.tf            # Lambda IAM role (BasicExecutionRole)
│   ├── lambda.tf         # Lambda fn + Function URL + IAM auth
│   └── outputs.tf        # function URL, ARN, role ARN
├── workers/
│   ├── gateway/          # proxy: health, kv, d1, queue, ai, lambda proxy, auth, cors
│   ├── auth/             # identity: register, login, verify, me, rate limit, input validation
│   └── analytics/        # events: track, events (paginated), summary
├── shared/src/           # crypto, bootstrap, tracing, response, error, bindings
├── lambda/devops-api/    # Rust Lambda: health, config, workers proxy, d1 proxy
├── tests/integration/    # Lambda<->Worker round-trip tests
├── scripts/
│   └── deploy-localstack.sh  # one-command: LocalStack → Lambda → EventBridge
├── Makefile              # build/deploy targets per worker
└── .gitignore            # excludes .omo/, .opencode/, target/, .wrangler/, tfstate
```

## Deploy Order

1. `terraform apply` (Cloudflare - KV, D1, R2, DNS, routes, WAF, zone settings)
2. `wrangler secret put LAMBDA_URL` + `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` on gateway worker
3. `cd aws && terraform apply` (AWS - Lambda fn + Function URL + IAM)
4. `make deploy-all` (deploys gateway, auth, analytics workers)
5. `LAMBDA_URL=<url> bash tests/integration/lambda-worker.sh` (verify round-trip)
6. `bash scripts/deploy-localstack.sh` (LocalStack → Lambda → EventBridge, for local AWS dev)
