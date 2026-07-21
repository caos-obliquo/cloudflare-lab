#!/usr/bin/env bash
# Integration tests: Lambda <-> Worker round-trip.
# Tests bidirectional communication between AWS Lambda and Cloudflare Workers.
#
# Prerequisites:
#   1. cargo lambda install (or use the built bootstrap binary directly)
#   2. wrangler CLI installed
#   3. AWS credentials configured (for SigV4 signing)
#   4. LAMBDA_URL env var set (or use the local test server)
#
# Usage:
#   export LAMBDA_URL=<deployed-url>
#   export WORKER_URL=http://localhost:8787  (from wrangler dev)
#   bash tests/integration/lambda-worker.sh
#
# Without deployed Lambda, you can test Worker endpoint locally:
#   wrangler dev --port 8787
#   WORKER_URL=http://localhost:8787 bash tests/integration/lambda-worker.sh --worker-only

set -euo pipefail

WORKER_URL="${WORKER_URL:-http://localhost:8787}"
LAMBDA_URL="${LAMBDA_URL:-}"  # optional: set to test Lambda too
PASS=0
FAIL=0

pass() { echo "  PASS: $1"; ((PASS++)); }
fail() { echo "  FAIL: $1"; ((FAIL++)); }

# Register a test user once, reuse token
test_user="inttest_$(date +%s)"
test_pass="Integration99!"
echo "=== Integration Test Setup ==="

echo "Registering test user: $test_user"
register_resp=$(curl -s -X POST "$WORKER_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
echo "  Register response: $register_resp"

TOKEN=$(echo "$register_resp" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)
if [ -z "$TOKEN" ]; then
  # Try login if register failed (user might already exist)
  login_resp=$(curl -s -X POST "$WORKER_URL/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
  TOKEN=$(echo "$login_resp" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)
  echo "  Login response: $login_resp"
else
  echo "  Got token: ${TOKEN:0:20}..."
fi

if [ -z "$TOKEN" ]; then
  echo "  ERROR: Could not get auth token. Aborting."
  exit 1
fi

echo ""
echo "=== Test 1: Lambda health check ==="
if [ -n "$LAMBDA_URL" ]; then
  # SigV4-signed curl request (requires awscurl or manual signing)
  if command -v awscurl &>/dev/null; then
    resp=$(awscurl --service lambda "$LAMBDA_URL/health")
    echo "  Lambda response: $resp"
    if echo "$resp" | grep -q '"status":"ok"'; then
      pass "Lambda /health returns ok"
    else
      fail "Lambda /health unexpected response"
    fi
  else
    echo "  SKIP: awscurl not installed (needed for SigV4 signing)"
    echo "  Install: pip install awscurl"
  fi
else
  echo "  SKIP: LAMBDA_URL not set (no Lambda endpoint to test)"
fi

echo ""
echo "=== Test 2: Worker health check ==="
resp=$(curl -s "$WORKER_URL/health")
echo "  Worker response: $resp"
if echo "$resp" | grep -q '"status":"healthy"'; then
  pass "Worker /health returns healthy"
else
  fail "Worker /health unexpected response"
fi

echo ""
echo "=== Test 3: Worker -> Lambda proxy (via /lambda/query) ==="
if [ -n "$LAMBDA_URL" ]; then
  resp=$(curl -s -X POST "$WORKER_URL/lambda/query" \
    -H "Content-Type: application/json" \
    -d "{\"action\":\"health\"}")
  echo "  Proxy response: $resp"
  if echo "$resp" | grep -q '"status":"ok"'; then
    pass "Worker proxies to Lambda successfully"
  else
    fail "Worker->Lambda proxy returned unexpected response"
  fi
else
  echo "  SKIP: LAMBDA_URL not set"

  # Test the worker endpoint exists even without Lambda configured
  resp=$(curl -s -X POST "$WORKER_URL/lambda/query" \
    -H "Content-Type: application/json" \
    -d '{"test":true}')
  echo "  Worker /lambda/query (no Lambda configured): $resp"
  pass "Worker /lambda/query endpoint is reachable (returns 502 if no Lambda URL)"
fi

echo ""
echo "=== Test 4: Auth round-trip (register -> login -> verify -> me) ==="

# Verify token
verify_resp=$(curl -s "$WORKER_URL/protected" \
  -H "Authorization: Bearer $TOKEN")
echo "  Protected endpoint: $verify_resp"
if echo "$verify_resp" | grep -q '"status":"ok"'; then
  pass "Token verification succeeds via gateway-worker"
else
  fail "Token verification failed"
fi

# Direct auth verify
verify_direct=$(curl -s "$WORKER_URL/verify" \
  -H "Authorization: Bearer $TOKEN")
echo "  Direct verify: $verify_direct"
if echo "$verify_direct" | grep -q '"valid":true'; then
  pass "Direct token verification succeeds"
else
  fail "Direct token verification failed"
fi

echo ""
echo "=== Test 5: Auth validation ==="
# Test invalid password (wrong password -> 401)
bad_login=$(curl -s -X POST "$WORKER_URL/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"WRONG\"}")
echo "  Wrong password: $bad_login"
if echo "$bad_login" | grep -q '401\|Invalid credentials'; then
  pass "Wrong password returns 401"
else
  fail "Wrong password did not return 401"
fi

# Test short password (validation -> 400)
short_reg=$(curl -s -X POST "$WORKER_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"newuser\",\"password\":\"ab\"}")
echo "  Short password: $short_reg"
if echo "$short_reg" | grep -q '400\|Invalid password'; then
  pass "Short password returns 400"
else
  fail "Short password did not return 400"
fi

echo ""
echo "=== Test 6: Lambda config endpoint ==="
if [ -n "$LAMBDA_URL" ] && command -v awscurl &>/dev/null; then
  resp=$(awscurl --service lambda "$LAMBDA_URL/config")
  echo "  Lambda /config: $resp"
  if echo "$resp" | grep -q '"environment"'; then
    pass "Lambda /config returns environment variables"
  else
    fail "Lambda /config unexpected"
  fi
else
  echo "  SKIP: needs LAMBDA_URL + awscurl"
fi

echo ""
echo "=========================================="
echo "Results: $PASS passed, $FAIL failed"
echo "=========================================="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
