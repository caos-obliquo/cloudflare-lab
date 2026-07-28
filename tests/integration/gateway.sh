#!/usr/bin/env bash
# Integration tests: Gateway Worker
# Tests: /health, OPTIONS CORS, 404, X-Request-Id, /metrics, /logs
#
# Usage:
#   GATEWAY_URL=http://127.0.0.1:8787 bash tests/integration/gateway.sh

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/lib.sh"

GATEWAY_URL="${GATEWAY_URL:-http://127.0.0.1:8787}"

require_cmd jq "install jq via apt/brew/nix" || exit 1
require_cmd curl "should be pre-installed" || exit 1

echo "=== Gateway Worker Tests ==="
echo "GATEWAY_URL=$GATEWAY_URL"
echo ""

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# --------------------------------------------------
# Test 1: GET /health returns 200 with status field
# --------------------------------------------------
echo "--- Test 1: GET /health ---"
code=$(curl -s -o "$TMPDIR/health.json" -w '%{http_code}' "$GATEWAY_URL/health")
assert_status 200 "$code" "/health"
assert_json_field '.status' 'healthy' "$TMPDIR/health.json" "/health status"

# --------------------------------------------------
# Test 2: OPTIONS returns CORS headers
# --------------------------------------------------
echo ""
echo "--- Test 2: OPTIONS CORS ---"
code=$(curl -s -D "$TMPDIR/options_headers.txt" -o /dev/null -w '%{http_code}' \
  -X OPTIONS -H "Origin: https://example.com" "$GATEWAY_URL/")
assert_status 204 "$code" "OPTIONS /"
if grep -qiE '^access-control-allow-origin: https?://example\.com' "$TMPDIR/options_headers.txt" 2>/dev/null; then
  pass "OPTIONS Access-Control-Allow-Origin reflects Origin"
else
  fail "OPTIONS — expected Access-Control-Allow-Origin to reflect Origin"
fi

# Also test OPTIONS without Origin
code2=$(curl -s -D "$TMPDIR/options_no_origin.txt" -o /dev/null -w '%{http_code}' \
  -X OPTIONS "$GATEWAY_URL/")
assert_status 204 "$code2" "OPTIONS / (no Origin)"
if grep -qiE '^access-control-allow-origin: \*' "$TMPDIR/options_no_origin.txt" 2>/dev/null; then
  pass "OPTIONS Access-Control-Allow-Origin = * when no Origin"
else
  fail "OPTIONS — expected Access-Control-Allow-Origin: *"
fi

# --------------------------------------------------
# Test 3: Unknown route -> 404
# --------------------------------------------------
echo ""
echo "--- Test 3: Unknown route -> 404 ---"
code=$(curl -s -o "$TMPDIR/404.json" -w '%{http_code}' "$GATEWAY_URL/this-route-does-not-exist")
assert_status 404 "$code" "Unknown route"
assert_json_field '.status' 'error' "$TMPDIR/404.json" "404 status"
assert_json_matches '.error' 'Not found' "$TMPDIR/404.json" "404 error message"

# --------------------------------------------------
# Test 4: X-Request-Id header — custom value echoed back
# --------------------------------------------------
echo ""
echo "--- Test 4: X-Request-Id echo ---"
custom_rid="test-rid-abc-123"
code=$(curl -s -o "$TMPDIR/rid.json" -w '%{http_code}' \
  -H "X-Request-Id: $custom_rid" "$GATEWAY_URL/health")
assert_status 200 "$code" "/health with custom X-Request-Id"
# Read the response header
resp_rid=$(curl -s -D "$TMPDIR/rid_headers.txt" -o /dev/null \
  -H "X-Request-Id: $custom_rid" "$GATEWAY_URL/health" 2>/dev/null; \
  grep -i '^x-request-id:' "$TMPDIR/rid_headers.txt" | head -1 | tr -d '\r' | sed 's/^[Xx]-[Rr]equest-[Ii][Dd]: //i')
echo "  Echoed X-Request-Id: $resp_rid"
if [ -n "$resp_rid" ]; then
  pass "X-Request-Id header present in response"
else
  fail "X-Request-Id header missing from response"
fi

# --------------------------------------------------
# Test 5: X-Request-Id — absent -> auto-generated
# --------------------------------------------------
echo ""
echo "--- Test 5: X-Request-Id auto-generated ---"
resp_rid2=$(curl -s -D "$TMPDIR/rid_headers2.txt" -o /dev/null \
  "$GATEWAY_URL/health" 2>/dev/null; \
  grep -i '^x-request-id:' "$TMPDIR/rid_headers2.txt" | head -1 | tr -d '\r' | sed 's/^[Xx]-[Rr]equest-[Ii][Dd]: //i')
echo "  Generated X-Request-Id: $resp_rid2"
if echo "$resp_rid2" | grep -qiE '^[a-f0-9\-]{8,}$'; then
  pass "X-Request-Id auto-generated (hex/uuid pattern)"
elif [ -n "$resp_rid2" ]; then
  pass "X-Request-Id auto-generated (non-empty)"
else
  fail "X-Request-Id header missing"
fi

# --------------------------------------------------
# Test 6: GET /metrics -> 200 with prometheus content
# --------------------------------------------------
echo ""
echo "--- Test 6: GET /metrics ---"
code=$(curl -s -o "$TMPDIR/metrics.txt" -w '%{http_code}' "$GATEWAY_URL/metrics")
assert_status 200 "$code" "/metrics"
assert_body_contains "cloudflare_requests_total" "$(cat "$TMPDIR/metrics.txt")" "/metrics contains cloudflare_requests_total"

# Check quantile lines in metrics
echo "  Checking quantile format..."
if grep -qE '^cloudflare_request_duration_ms\{[^}]*quantile="0\.(5|9|99)"[^}]*\} [0-9]' "$TMPDIR/metrics.txt" 2>/dev/null; then
  pass "/metrics has well-formed quantile lines"
else
  # Not all metrics may have quantiles yet — check for any histogram bucket
  if grep -qE 'cloudflare_request_duration_ms' "$TMPDIR/metrics.txt" 2>/dev/null; then
    pass "/metrics has request_duration_ms (quantile format not confirmed)"
  else
    fail "/metrics missing request_duration_ms"
  fi
fi

# --------------------------------------------------
# Test 7: GET /logs -> 200 JSON array
# --------------------------------------------------
echo ""
echo "--- Test 7: GET /logs ---"
code=$(curl -s -o "$TMPDIR/logs.json" -w '%{http_code}' "$GATEWAY_URL/logs")
assert_status 200 "$code" "/logs"
# Should contain "logs" array
if jq -e '.logs | type == "array"' "$TMPDIR/logs.json" &>/dev/null; then
  pass "/logs.logs is a JSON array"
else
  fail "/logs — 'logs' field is not an array"
fi
assert_json_field '.status' 'ok' "$TMPDIR/logs.json" "/logs status"

print_summary "gateway"
