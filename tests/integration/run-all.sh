#!/usr/bin/env bash
# Integration test runner — boots wrangler dev servers, runs suites, tears down.
#
# Usage:
#   # Full run (all workers on default ports)
#   bash tests/integration/run-all.sh
#
#   # Specific suites only
#   bash tests/integration/run-all.sh --only gateway,auth
#
#   # With D1 persistence
#   PERSIST=1 bash tests/integration/run-all.sh
#
#   # Skip binding-dependent tests (no D1/KV/DO)
#   SKIP_NO_BINDINGS=1 bash tests/integration/run-all.sh
#
# Ports:
#   GATEWAY=8787  AUTH=8788  ANALYTICS=8789
#
# Environment:
#   SESSION_SECRET  — HMAC signing key (default: test-secret-for-ci)
#   LAMBDA_URL      — if set, runs lambda-worker tests too
#   PERSIST         — add --persist to wrangler dev for D1 state
#   SKIP_NO_BINDINGS=1 — skip tests needing D1/KV/DO bindings
#   ONLY            — comma-separated suite filter (alt: --only)

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"

# --- Prerequisites ---
for cmd in npx jq curl; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "FATAL: '$cmd' not found. Install it first."
    exit 1
  fi
done

# --- Ports ---
GATEWAY_PORT="${GATEWAY_PORT:-8787}"
AUTH_PORT="${AUTH_PORT:-8788}"
ANALYTICS_PORT="${ANALYTICS_PORT:-8789}"

export GATEWAY_URL="http://127.0.0.1:${GATEWAY_PORT}"
export AUTH_URL="http://127.0.0.1:${AUTH_PORT}"
export ANALYTICS_URL="http://127.0.0.1:${ANALYTICS_PORT}"
export SESSION_SECRET="${SESSION_SECRET:-test-secret-for-ci}"
export SKIP_LAMBDA="${SKIP_LAMBDA:-1}"
export SKIP_NO_BINDINGS="${SKIP_NO_BINDINGS:-0}"

# Determine worker directories (assume we're at repo root)
REPO_ROOT="$(cd "$DIR/../.." && pwd)"
WORKER_GATEWAY="$REPO_ROOT/workers/gateway"
WORKER_AUTH="$REPO_ROOT/workers/auth"
WORKER_ANALYTICS="$REPO_ROOT/workers/analytics"

# Wrangler dev flags
WDRY_FLAGS=""
[ -n "${PERSIST:-}" ] && WDRY_FLAGS="$WDRY_FLAGS --persist"
# In CI, skip binding validation (no D1/KV/DO available)
[ "${SKIP_NO_BINDINGS:-0}" = "1" ] && WDRY_FLAGS="$WDRY_FLAGS --no-bindings"

# --- Parse --only ---
RUN_ALL_SUITES=1
SUITE_FILTER=""
if [ $# -gt 0 ]; then
  case "$1" in
    --only)
      shift
      SUITE_FILTER="$1"
      RUN_ALL_SUITES=0
      ;;
    *)
      echo "Usage: $0 [--only gateway,auth,analytics,observability,lambda]"
      exit 1
      ;;
  esac
fi

should_run() {
  local name=$1
  if [ "$RUN_ALL_SUITES" -eq 1 ]; then return 0; fi
  IFS=',' read -ra PARTS <<< "$SUITE_FILTER"
  for part in "${PARTS[@]}"; do
    [ "$part" = "$name" ] && return 0
  done
  return 1
}

# --- PID tracking ---
WORKER_PIDS=()
CLEANUP_DONE=0

cleanup() {
  if [ "$CLEANUP_DONE" -eq 1 ]; then return; fi
  CLEANUP_DONE=1
  echo ""
  echo "=== Shutting down wrangler dev servers ==="
  local pid
  for pid in "${WORKER_PIDS[@]}"; do
    if kill "$pid" 2>/dev/null; then
      echo "  Stopped PID $pid"
    fi
  done
  # Wait for all to exit
  wait "${WORKER_PIDS[@]}" 2>/dev/null || true
  echo "  All stopped."
}

trap cleanup EXIT INT TERM

# --- Boot a wrangler dev server ---
boot_worker() {
  local name=$1 dir=$2 port=$3
  echo "  Booting $name on port $port..."
  # Redirect both stdout and stderr to a log file so output isn't lost
  local logfile="/tmp/wrangler-${name}.log"
  # The --ip 127.0.0.1 is important for security (not listening on all interfaces)
  # Use wrangler directly (not npx) — pre-installed globally in CI via npm install -g wrangler
  wrangler dev --port "$port" --ip 127.0.0.1 --inspector-port 0 $WDRY_FLAGS \
    --var SESSION_SECRET:"$SESSION_SECRET" \
    --cwd "$dir" \
    > "$logfile" 2>&1 &
  local pid=$!
  WORKER_PIDS+=("$pid")
  echo "  $name PID $pid, log at $logfile"
}

# --- Wait for health ---
wait_for_health() {
  local name=$1 url=$2 timeout_sec="${3:-120}"
  local waited=0
  echo -n "  Waiting for $name ($url/health) up to ${timeout_sec}s..."
  while [ "$waited" -lt "$timeout_sec" ]; do
    if curl -sf "$url/health" > /dev/null 2>&1; then
      echo " UP after ${waited}s"
      return 0
    fi
    sleep 2
    waited=$((waited + 2))
    echo -n "."
  done
  echo " TIMEOUT after ${timeout_sec}s"
  return 1
}

# --- Main ---
echo "=========================================="
echo " Cloudflare Workers — Integration Test Runner"
echo "=========================================="
echo "Repo:      $REPO_ROOT"
echo "Gateway:   $GATEWAY_URL"
echo "Auth:      $AUTH_URL"
echo "Analytics: $ANALYTICS_URL"
echo "SKIP_NO_BINDINGS=$SKIP_NO_BINDINGS"
echo "PERSIST=${PERSIST:-0}"
echo ""

# Boot all three workers
echo "--- Starting workers ---"
boot_worker "gateway" "$WORKER_GATEWAY" "$GATEWAY_PORT"
boot_worker "auth" "$WORKER_AUTH" "$AUTH_PORT"
boot_worker "analytics" "$WORKER_ANALYTICS" "$ANALYTICS_PORT"
echo ""

# Wait for health on each
echo "--- Waiting for health ---"
ALL_UP=1
wait_for_health "gateway" "$GATEWAY_URL" 120 || ALL_UP=0
wait_for_health "auth" "$AUTH_URL" 120 || ALL_UP=0
wait_for_health "analytics" "$ANALYTICS_URL" 120 || ALL_UP=0
echo ""

if [ "$ALL_UP" -eq 0 ]; then
  echo "FATAL: Not all workers became healthy. Aborting."
  echo ""
  echo "=== Dumping wrangler dev logs (last 20 lines each) ==="
  for log in /tmp/wrangler-gateway.log /tmp/wrangler-auth.log /tmp/wrangler-analytics.log; do
    echo "--- $log ---"
    tail -20 "$log" 2>/dev/null || echo "  (no log file)"
  done
  exit 1
fi

# Run suites
echo "--- Running test suites ---"
TOTAL_FAIL=0

run_suite() {
  local name=$1 script=$2
  if should_run "$name"; then
    echo ""
    echo "=========================================="
    echo " Suite: $name"
    echo "=========================================="
    if bash "$script"; then
      echo "  >>> Suite '$name' PASSED <<<"
    else
      echo "  >>> Suite '$name' FAILED <<<"
      TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
  else
    echo "  SKIP suite '$name' (filtered)"
  fi
}

run_suite "gateway" "$DIR/gateway.sh"
run_suite "auth" "$DIR/auth.sh"
run_suite "analytics" "$DIR/analytics.sh"
run_suite "observability" "$DIR/observability.sh"

# Lambda suite — only runs if LAMBDA_URL is set
if [ -n "${LAMBDA_URL:-}" ]; then
  run_suite "lambda-worker" "$DIR/lambda-worker.sh"
else
  echo "  SKIP suite 'lambda-worker' (LAMBDA_URL not set)"
fi

# Summary
echo ""
echo "=========================================="
echo " Overall Results"
echo "=========================================="
if [ "$TOTAL_FAIL" -eq 0 ]; then
  echo " All suites passed."
  exit 0
else
  echo " $TOTAL_FAIL suite(s) failed."
  exit 1
fi
