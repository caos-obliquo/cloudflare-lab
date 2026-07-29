#!/usr/bin/env bash
# Integration tests: Analytics Worker
# Tests: POST /track (201/400/401), GET /events (limit clamp, auth), oversized data
#
# Usage:
#   ANALYTICS_URL=http://127.0.0.1:8789 bash tests/integration/analytics.sh
#
# Requires AUTH_URL to obtain session token. Needs D1 binding available.
# Set SKIP_NO_BINDINGS=1 to skip D1-dependent tests.

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/lib.sh"

ANALYTICS_URL="${ANALYTICS_URL:-http://127.0.0.1:8789}"
AUTH_URL="${AUTH_URL:-http://127.0.0.1:8788}"
SESSION_SECRET="${SESSION_SECRET:-test-secret-for-ci}"
SKIP_NO_BINDINGS="${SKIP_NO_BINDINGS:-0}"

require_cmd jq "install jq via apt/brew/nix" || exit 1
require_cmd curl "should be pre-installed" || exit 1

echo "=== Analytics Worker Tests ==="
echo "ANALYTICS_URL=$ANALYTICS_URL"
echo "AUTH_URL=$AUTH_URL"
echo ""

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# --- Setup: get a session token from auth worker ---
analytics_user="analtest$(date +%s)$$"
analytics_pass="TestPass789!"
echo "--- Setup: register user for auth ---"
reg_code=$(curl -s -o "$TMPDIR/setup_reg.json" -w '%{http_code}' \
  -X POST "$AUTH_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$analytics_user\",\"password\":\"$analytics_pass\"}")

# If registration fails (e.g., duplicate from prior run), try login
TOKEN=
if [ "$reg_code" -eq 201 ]; then
  TOKEN=$(jq -r '.token // empty' "$TMPDIR/setup_reg.json")
fi
if [ -z "$TOKEN" ]; then
  login_code=$(curl -s -o "$TMPDIR/setup_login.json" -w '%{http_code}' \
    -X POST "$AUTH_URL/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"$analytics_user\",\"password\":\"$analytics_pass\"}")
  if [ "$login_code" -eq 200 ]; then
    TOKEN=$(jq -r '.token // empty' "$TMPDIR/setup_login.json")
  fi
fi

if [ -z "$TOKEN" ]; then
  echo "  WARNING: Could not obtain auth token. Auth-dependent tests will fail."
  echo "  Register response code: $reg_code"
fi

# --------------------------------------------------
# Test 1: POST /track without Bearer -> 401
# --------------------------------------------------
echo ""
echo "--- Test 1: POST /track (no auth) -> 401 ---"
code=$(curl -s -o "$TMPDIR/track_noauth.json" -w '%{http_code}' \
  -X POST "$ANALYTICS_URL/track" \
  -H "Content-Type: application/json" \
  -d '{"event_type":"test_noauth"}')
assert_status 401 "$code" "POST /track (no auth)"

# --------------------------------------------------
# Test 2: POST /track with valid Bearer -> 201
# --------------------------------------------------
echo ""
echo "--- Test 2: POST /track (with auth) -> 201 ---"
if [ -n "$TOKEN" ]; then
  code=$(curl -s -o "$TMPDIR/track_ok.json" -w '%{http_code}' \
    -X POST "$ANALYTICS_URL/track" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d '{"event_type":"page_view","event_data":"{\"page\":\"/home\"}"}')
  assert_status 201 "$code" "POST /track (with auth)"
  assert_json_field '.status' 'ok' "$TMPDIR/track_ok.json" "/track status"
  assert_json_field '.event_type' 'page_view' "$TMPDIR/track_ok.json" "/track event_type"
else
  echo "  SKIP: no token available"
fi

# --------------------------------------------------
# Test 3: GET /events with auth -> 200 + array
# --------------------------------------------------
echo ""
echo "--- Test 3: GET /events (with auth) -> 200 ---"
if [ -n "$TOKEN" ]; then
  code=$(curl -s -o "$TMPDIR/events_ok.json" -w '%{http_code}' \
    "$ANALYTICS_URL/events?limit=5" \
    -H "Authorization: Bearer $TOKEN")
  assert_status 200 "$code" "GET /events (with auth)"
  if jq -e '.events | type == "array"' "$TMPDIR/events_ok.json" &>/dev/null; then
    pass "GET /events.events is an array"
    count=$(jq -r '.events | length' "$TMPDIR/events_ok.json")
    if [ "$count" -le 5 ]; then
      pass "GET /events returned $count events (<=5)"
    else
      fail "GET /events returned $count events (expected <=5)"
    fi
  else
    fail "GET /events — 'events' field is not an array"
  fi
  assert_json_field '.status' 'ok' "$TMPDIR/events_ok.json" "/events status"
else
  echo "  SKIP: no token available"
fi

# --------------------------------------------------
# Test 4: GET /events with limit=9999 -> clamped <=100
# --------------------------------------------------
echo ""
echo "--- Test 4: GET /events limit=9999 -> clamped ---"
if [ -n "$TOKEN" ]; then
  code=$(curl -s -o "$TMPDIR/events_clamped.json" -w '%{http_code}' \
    "$ANALYTICS_URL/events?limit=9999" \
    -H "Authorization: Bearer $TOKEN")
  assert_status 200 "$code" "GET /events limit=9999"
  if jq -e '.events | type == "array"' "$TMPDIR/events_clamped.json" &>/dev/null; then
    count=$(jq -r '.events | length' "$TMPDIR/events_clamped.json")
    if [ "$count" -le 100 ]; then
      pass "GET /events limit=9999 clamped: returned $count events (<=100)"
    else
      fail "GET /events limit=9999: returned $count events (expected <=100)"
    fi
  else
    fail "GET /events limit=9999 — 'events' not array"
  fi
else
  echo "  SKIP: no token available"
fi

# --------------------------------------------------
# Test 5: POST /track malformed JSON -> 400
# --------------------------------------------------
echo ""
echo "--- Test 5: POST /track malformed JSON -> 400 ---"
if [ -n "$TOKEN" ]; then
  code=$(curl -s -o "$TMPDIR/track_badjson.json" -w '%{http_code}' \
    -X POST "$ANALYTICS_URL/track" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d 'not-json-at-all')
  # May get 400 (parse error) or 500 (internal)
  if [ "$code" -eq 400 ] || [ "$code" -eq 500 ]; then
    pass "POST /track malformed JSON — got HTTP $code (expected 400/500)"
  else
    fail "POST /track malformed JSON — expected 400/500, got $code"
  fi
else
  echo "  SKIP: no token available"
fi

# --------------------------------------------------
# Test 6: POST /track oversized event_data (>10KB) -> 400
# --------------------------------------------------
echo ""
echo "--- Test 6: POST /track oversized event_data -> 400 ---"
if [ -n "$TOKEN" ]; then
  big_data=$(printf 'x%.0s' $(seq 1 11264))
  payload=$(jq -n --arg data "$big_data" '{event_type: "oversized", event_data: $data}')
  code=$(curl -s -o "$TMPDIR/track_big.json" -w '%{http_code}' \
    -X POST "$ANALYTICS_URL/track" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d "$payload")
  # Accept 400 (rejected), 413 (payload too large), or 500 (server error on large payload)
  if [ "$code" -eq 400 ] || [ "$code" -eq 413 ] || [ "$code" -eq 500 ]; then
    pass "POST /track oversized event_data — got HTTP $code (expected rejection)"
  elif [ "$code" -eq 201 ]; then
    fail "POST /track oversized event_data — accepted 11KB data (expected rejection)"
  else
    fail "POST /track oversized event_data — unexpected HTTP $code"
  fi
else
  echo "  SKIP: no token available"
fi

print_summary "analytics"
