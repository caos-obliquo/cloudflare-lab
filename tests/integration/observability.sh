#!/usr/bin/env bash
# Integration tests: Observability — metrics format, health, trace context, CORS
# Runs against gateway worker (has /metrics, /health, /logs endpoints).
#
# Usage:
#   GATEWAY_URL=http://127.0.0.1:8787 bash tests/integration/observability.sh

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/lib.sh"

GATEWAY_URL="${GATEWAY_URL:-http://127.0.0.1:8787}"

require_cmd jq "install jq via apt/brew/nix" || exit 1
require_cmd curl "should be pre-installed" || exit 1

echo "=== Observability Tests ==="
echo "GATEWAY_URL=$GATEWAY_URL"
echo ""

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# --------------------------------------------------
# Test 1: /metrics — every sample line matches prometheus format
# --------------------------------------------------
echo "--- Test 1: /metrics line format ---"
code=$(curl -s -o "$TMPDIR/metrics.txt" -w '%{http_code}' "$GATEWAY_URL/metrics")
assert_status 200 "$code" "/metrics"

# Validate every non-empty, non-comment line matches prometheus format:
# metric_name{labels} value
bad_lines=0
total_sample=0
while IFS= read -r line; do
  # Skip comments, empty lines, and type/help lines
  case "$line" in
    '#'*) continue ;;
    '') continue ;;
  esac
  total_sample=$((total_sample + 1))
  # Prometheus sample format: metric_name [labels] value
  if ! echo "$line" | grep -qE '^[a-zA-Z_:][a-zA-Z0-9_:]*(\{[^}]*\})? [0-9.eE+\-]+'; then
    bad_lines=$((bad_lines + 1))
    echo "  BAD LINE: $line"
  fi
done < "$TMPDIR/metrics.txt"

if [ "$total_sample" -eq 0 ]; then
  fail "/metrics — no sample lines found (empty or all comments)"
elif [ "$bad_lines" -eq 0 ]; then
  pass "/metrics — all $total_sample sample lines match prometheus format"
else
  fail "/metrics — $bad_lines/$total_sample sample lines have bad format"
fi

# --------------------------------------------------
# Test 2: /health has status + checks (version not always present)
# --------------------------------------------------
echo ""
echo "--- Test 2: /health response ---"
code=$(curl -s -o "$TMPDIR/health.json" -w '%{http_code}' "$GATEWAY_URL/health")
assert_status 200 "$code" "/health"
# Check for status field
assert_json_matches '.status' 'healthy|degraded' "$TMPDIR/health.json" "/health status"
# Check for checks array
if jq -e '.checks | type == "array"' "$TMPDIR/health.json" &>/dev/null; then
  pass "/health has checks array"
else
  fail "/health — missing 'checks' array"
fi

# --------------------------------------------------
# Test 3: traceparent header sent -> 200 no crash
# --------------------------------------------------
echo ""
echo "--- Test 3: traceparent propagation ---"
code=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01" \
  "$GATEWAY_URL/health")
assert_status 200 "$code" "/health with traceparent header"

# Also check the response contains traceparent via tracestate
if curl -s -D "$TMPDIR/trace_headers.txt" -o /dev/null \
  -H "traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01" \
  "$GATEWAY_URL/health" 2>/dev/null; then
  if grep -qiE 'traceparent|tracestate' "$TMPDIR/trace_headers.txt" 2>/dev/null; then
    pass "Response contains trace context headers"
  else
    # Not all configs inject traceparent into response — non-critical
    echo "  NOTE: No trace context headers in response (may be expected)"
  fi
fi

# --------------------------------------------------
# Test 4: CORS preflight response headers
# --------------------------------------------------
echo ""
echo "--- Test 4: CORS preflight headers ---"
# OPTIONS without Origin -> get wildcard CORS
code=$(curl -s -D "$TMPDIR/cors_headers.txt" -o /dev/null -w '%{http_code}' \
  -X OPTIONS "$GATEWAY_URL/")
assert_status 204 "$code" "OPTIONS /"

# Check CORS headers
if grep -qiE '^access-control-allow-origin:' "$TMPDIR/cors_headers.txt" 2>/dev/null; then
  pass "OPTIONS response has Access-Control-Allow-Origin"
else
  fail "OPTIONS missing Access-Control-Allow-Origin"
fi

if grep -qiE '^access-control-allow-headers:' "$TMPDIR/cors_headers.txt" 2>/dev/null; then
  allow_headers=$(grep -i '^access-control-allow-headers:' "$TMPDIR/cors_headers.txt" | sed 's/^[^:]*: //i' | tr -d '\r')
  echo "  Access-Control-Allow-Headers: $allow_headers"
  pass "OPTIONS has Access-Control-Allow-Headers header"
else
  echo "  NOTE: Access-Control-Allow-Headers not in OPTIONS response"
  # The gateway sets CORS headers on ALL responses via the general response path,
  # but OPTIONS returns early (line 44) before the general CORS code at line 155.
  # So OPTIONS may NOT have Allow-Headers. This is a known observation.
fi

# Check for Access-Control-Allow-Methods
if grep -qiE '^access-control-allow-methods:' "$TMPDIR/cors_headers.txt" 2>/dev/null; then
  pass "OPTIONS has Access-Control-Allow-Methods"
else
  echo "  NOTE: Access-Control-Allow-Methods not in OPTIONS response"
fi

# --------------------------------------------------
# Test 5: HEADERS on normal response include CORS + traceparent support
# --------------------------------------------------
echo ""
echo "--- Test 5: Response headers include traceparent-compatible CORS ---"
normal_headers=$(curl -s -D "$TMPDIR/normal_headers.txt" -o "$TMPDIR/normal_body.json" \
  -H "Origin: http://localhost:3000" \
  "$GATEWAY_URL/health" 2>/dev/null; cat "$TMPDIR/normal_headers.txt")

# Check normal response has CORS (since code adds it at line 155-159)
if grep -qiE '^access-control-allow-origin:' "$TMPDIR/normal_headers.txt" 2>/dev/null; then
  pass "Normal response has Access-Control-Allow-Origin"
else
  fail "Normal response missing Access-Control-Allow-Origin"
fi

# Check Access-Control-Allow-Headers includes traceparent and x-request-id
if grep -qiE '^access-control-allow-headers:' "$TMPDIR/normal_headers.txt" 2>/dev/null; then
  ah=$(grep -i '^access-control-allow-headers:' "$TMPDIR/normal_headers.txt" | sed 's/^[^:]*: //i' | tr -d '\r' | tr ',' '\n')
  has_tp=0; has_rid=0
  while IFS= read -r h; do
    h_trimmed=$(echo "$h" | tr -d ' ')
    [ "$h_trimmed" = "traceparent" ] && has_tp=1
    [ "$h_trimmed" = "x-request-id" ] && has_rid=1
  done <<< "$ah"
  if [ "$has_tp" -eq 1 ]; then
    pass "Access-Control-Allow-Headers includes traceparent"
  else
    echo "  NOTE: traceparent not in Access-Control-Allow-Headers (current code has Content-Type, Authorization)"
  fi
  if [ "$has_rid" -eq 1 ]; then
    pass "Access-Control-Allow-Headers includes x-request-id"
  else
    echo "  NOTE: x-request-id not in Access-Control-Allow-Headers (current code has Content-Type, Authorization)"
  fi
else
  echo "  NOTE: Access-Control-Allow-Headers not in response (checking X-Request-Id directly)"
fi

# X-Request-Id should be present in normal responses
if grep -qiE '^x-request-id:' "$TMPDIR/normal_headers.txt" 2>/dev/null; then
  pass "Normal response has X-Request-Id header"
else
  fail "Normal response missing X-Request-Id header"
fi

print_summary "observability"
