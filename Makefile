.PHONY: build-gateway build-auth build-analytics build-rate-limiter build-all deploy-gateway deploy-auth deploy-analytics deploy-rate-limiter deploy-all

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
