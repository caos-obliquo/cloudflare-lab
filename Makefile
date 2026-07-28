.PHONY: build-gateway build-auth build-analytics build-rate-limiter build-all deploy-gateway deploy-auth deploy-analytics deploy-rate-limiter deploy-all test test-integration test-all test-api prom-test validate-dashboards load-test

# Build individual workers
build-gateway:
	cd workers/gateway && worker-build --release

build-auth:
	cd workers/auth && worker-build --release

build-analytics:
	cd workers/analytics && worker-build --release

build-rate-limiter:
	cd workers/rate-limiter && worker-build --release

# Build all workers in parallel
build-all:
	@$(MAKE) -j4 build-gateway build-auth build-analytics build-rate-limiter

# Deploy individual workers (build + wrangler deploy)
deploy-gateway:
	cd workers/gateway && wrangler deploy

deploy-auth:
	cd workers/auth && wrangler deploy

deploy-analytics:
	cd workers/analytics && wrangler deploy

deploy-rate-limiter:
	cd workers/rate-limiter && wrangler deploy

# Deploy all workers
deploy-all: build-all
	@$(MAKE) deploy-gateway deploy-auth deploy-analytics deploy-rate-limiter

# ─── Test targets ──────────────────────────────────────────

# Rust unit tests
test:
	cargo test --workspace

# Integration tests (boots wrangler dev servers, needs npx+jq+curl)
test-integration:
	bash tests/integration/run-all.sh

# All tests: unit + integration + SLO
test-all: test test-integration prom-test

# CLI debug tool: hit worker APIs with canned payloads
# Usage: make test-api ARGS="auth health"
#        make test-api ARGS="auth register --user test --pass TestPass123!"
test-api:
	bash scripts/test-api.sh $(ARGS)

# Prometheus SLO rule validation
# Support PODMAN=1 for podman users
PROMTOOL_RUNNER = $(if $(PODMAN),podman,docker)

prom-test:
	$(PROMTOOL_RUNNER) run --rm -v $(PWD)/prometheus:/prometheus:ro --entrypoint promtool prom/prometheus:latest test rules /prometheus/rules/tests/worker-slo.test.yml

# Validate Grafana dashboards against datasource references
validate-dashboards:
	bash scripts/validate-dashboards.sh

# k6 load test against gateway worker
load-test:
	k6 run k6/load.js
