#!/usr/bin/env bash
# Integration tests: Lambda <-> Worker round-trip.
# Tests bidirectional communication between AWS Lambda and Cloudflare Workers.
#
# Prerequisites:
#   1. awscurl installed (pip install awscurl) for SigV4-signed requests
#   2. wrangler CLI installed (for worker side)
#   3. LAMBDA_URL env var set for full round-trip tests
#
# Usage:
#   export LAMBDA_URL=<deployed-lambda-url>
#   export WORKER_URL=http://localhost:8787
#   bash tests/integration/lambda-worker.sh
#
# Without Lambda deployed, skips Lambda-specific tests when SKIP_LAMBDA=1:
#   SKIP_LAMBDA=1 bash tests/integration/lambda-worker.sh

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/lib.sh"

WORKER_URL="${WORKER_URL:-http://localhost:8787}"
LAMBDA_URL="${LAMBDA_URL:-}"
SKIP_LAMBDA="${SKIP_LAMBDA:-1}"

require_cmd jq "install jq via apt/brew/nix" || exit 1

echo "=== Lambda-Worker Integration Tests ==="
echo "WORKER_URL=$WORKER_URL"
echo "LAMBDA_URL=${LAMBDA_URL:-"(not set — SKIP_LAMBDA=$SKIP_LAMBDA)"}"
echo ""

# --- Register test user ---
test_user="inttest_$(date +%s)_$$"
test_pass="Integration99!"
echo "--- Setup: register test user '$test_user' ---"
register_resp=$(curl -s -X POST "$WORKER_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
echo "  Register: $register_resp"

TOKEN=
# Try extracting token from register response
if echo "$register_resp" | jq -e '.token' &>/dev/null; then
  TOKEN=$(echo "$register_resp" | jq -r '.token // empty')
fi

# Fallback: login
if [ -z "$TOKEN" ]; then
  login_resp=$(curl -s -X POST "$WORKER_URL/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
  echo "  Login: $login_resp"
  TOKEN=$(echo "$login_resp" | jq -r '.token // empty')
fi

if [ -z "$TOKEN" ]; then
  echo "  ERROR: Could not get auth token. Aborting."
  exit 1
fi
echo "  Token: ${TOKEN:0:20}..."

# --- Test 1: Lambda health check ---
echo ""
echo "=== Test 1: Lambda health check ==="
if [ -n "$LAMBDA_URL" ] && [ "$SKIP_LAMBDA" != "1" ]; then
  if require_cmd awscurl "install: pip install awscurl"; then
    resp=$(awscurl --service lambda "$LAMBDA_URL/health")
    echo "  Lambda /health: $resp"
    status=$(echo "$resp" | jq -r '.status // empty')
    if [ "$status" = "ok" ]; then
      pass "Lambda /health returns ok"
    else
      fail "Lambda /health — expected status 'ok', got '$status'"
    fi
  fi
else
  echo "  SKIP: LAMBDA_URL not set or SKIP_LAMBDA=1"
fi

# --- Test 2: Worker health check ---
echo ""
echo "=== Test 2: Worker health check ==="
resp=$(curl -s -o /tmp/lambda_health.json -w '%{http_code}' "$WORKER_URL/health")
assert_status 200 "$resp" "Worker /health"
assert_json_matches '.status' 'healthy|degraded' /tmp/lambda_health.json "Worker /health status"

# --- Test 3: Worker -> Lambda proxy ---
echo ""
echo "=== Test 3: Worker -> Lambda proxy (/lambda/query) ==="
if [ -n "$LAMBDA_URL" ] && [ "$SKIP_LAMBDA" != "1" ]; then
  proxy_code=$(curl -s -o /tmp/lambda_proxy.json -w '%{http_code}' \
    -X POST "$WORKER_URL/lambda/query" \
    -H "Content-Type: application/json" \
    -d '{"action":"health"}')
  echo "  Proxy status: $proxy_code"
  if [ "$proxy_code" -eq 200 ]; then
    assert_json_field '.status' 'ok' /tmp/lambda_proxy.json "Lambda proxy response"
  else
    fail "Worker -> Lambda proxy — expected 200, got $proxy_code"
  fi
else
  echo "  SKIP: LAMBDA_URL not set or SKIP_LAMBDA=1"

  # Test endpoint exists but expect 502 without Lambda configured
  proxy_code=$(curl -s -o /tmp/lambda_proxy.json -w '%{http_code}' \
    -X POST "$WORKER_URL/lambda/query" \
    -H "Content-Type: application/json" \
    -d '{"test":true}')
  echo "  Worker /lambda/query (no Lambda): HTTP $proxy_code"
  if [ "$proxy_code" -eq 502 ]; then
    pass "Worker /lambda/query returns 502 when Lambda not configured (expected)"
  elif [ "$proxy_code" -eq 200 ]; then
    pass "Worker /lambda/query reachable (code $proxy_code)"
  else
    fail "Worker /lambda/query — unexpected status $proxy_code"
  fi
fi

# --- Test 4: Auth round-trip via gateway ---
echo ""
echo "=== Test 4: Gateway auth round-trip (/protected) ==="
prot_code=$(curl -s -o /tmp/lambda_protected.json -w '%{http_code}' \
  "$WORKER_URL/protected" \
  -H "Authorization: Bearer $TOKEN")
assert_status 200 "$prot_code" "Gateway /protected (valid token)"
assert_json_field '.status' 'ok' /tmp/lambda_protected.json "Gateway protected response"

# --- Test 5: Direct auth verify ---
echo ""
echo "=== Test 5: Direct auth verify ==="
verify_code=$(curl -s -o /tmp/lambda_verify.json -w '%{http_code}' \
  "$WORKER_URL/verify" \
  -H "Authorization: Bearer $TOKEN")
assert_status 200 "$verify_code" "Direct /verify (valid token)"
assert_json_field '.status' 'ok' /tmp/lambda_verify.json "Direct verify status"

# --- Test 6: Auth validation (wrong password) ---
echo ""
echo "=== Test 6: Auth validation ==="

# Wrong password -> 401
bad_code=$(curl -s -o /tmp/lambda_bad_login.json -w '%{http_code}' \
  -X POST "$WORKER_URL/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"WRONG\"}")
assert_status 401 "$bad_code" "Wrong password returns 401"

# Short password -> 400
short_code=$(curl -s -o /tmp/lambda_short_reg.json -w '%{http_code}' \
  -X POST "$WORKER_URL/register" \
  -H "Content-Type: application/json" \
  -d '{"username":"newuser","password":"ab"}')
assert_status 400 "$short_code" "Short password returns 400"

# --- Lambda config endpoint ---
echo ""
echo "=== Test 7: Lambda config endpoint ==="
if [ -n "$LAMBDA_URL" ] && [ "$SKIP_LAMBDA" != "1" ] && command -v awscurl &>/dev/null; then
  resp=$(awscurl --service lambda "$LAMBDA_URL/config")
  echo "  Lambda /config: $resp"
  if echo "$resp" | jq -e '.environment' &>/dev/null; then
    pass "Lambda /config returns environment variables"
  else
    fail "Lambda /config — missing 'environment' field"
  fi
else
  echo "  SKIP: needs LAMBDA_URL + awscurl"
fi

print_summary "lambda-worker"
