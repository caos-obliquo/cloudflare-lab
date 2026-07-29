#!/usr/bin/env bash
# Integration tests: Auth Worker
# Tests: register (201/409/400), login (200/401), verify (200/401), rate-limit
#
# Usage:
#   AUTH_URL=http://127.0.0.1:8788 bash tests/integration/auth.sh
#
# Routes (actual): POST /register, POST /login, GET /verify, GET /me
# Rate-limit tests need the rate-limiter DO running (SKIP_NO_BINDINGS env).

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/lib.sh"

AUTH_URL="${AUTH_URL:-http://127.0.0.1:8788}"
SKIP_NO_BINDINGS="${SKIP_NO_BINDINGS:-0}"

require_cmd jq "install jq via apt/brew/nix" || exit 1
require_cmd curl "should be pre-installed" || exit 1

echo "=== Auth Worker Tests ==="
echo "AUTH_URL=$AUTH_URL"
echo ""

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# Unique test user per run
test_user="authtest$(date +%s)$$"
test_pass="StrongPass99!"

# --------------------------------------------------
# Test 1: POST /register -> 201
# --------------------------------------------------
echo "--- Test 1: POST /register (valid) -> 201 ---"
code=$(curl -s -o "$TMPDIR/register.json" -w '%{http_code}' \
  -X POST "$AUTH_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
assert_status 201 "$code" "POST /register (valid)"
assert_json_field '.status' 'ok' "$TMPDIR/register.json" "/register status"
assert_json_field '.username' "$test_user" "$TMPDIR/register.json" "/register username"

# --------------------------------------------------
# Test 2: Duplicate register -> 409
# --------------------------------------------------
echo ""
echo "--- Test 2: POST /register (duplicate) -> 409 ---"
code=$(curl -s -o "$TMPDIR/register_dup.json" -w '%{http_code}' \
  -X POST "$AUTH_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
# Code could be 409 (code) or 400 (if other validation catches first)
if [ "$code" -eq 409 ] || [ "$code" -eq 400 ]; then
  pass "POST /register duplicate — got HTTP $code (expected 409 or 400)"
else
  fail "POST /register duplicate — expected 409/400, got $code"
fi

# --------------------------------------------------
# Test 3: Login with wrong password -> 401
# --------------------------------------------------
echo ""
echo "--- Test 3: POST /login (wrong password) -> 401 ---"
code=$(curl -s -o "$TMPDIR/login_bad.json" -w '%{http_code}' \
  -X POST "$AUTH_URL/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"wrongpass99!\"}")
assert_status 401 "$code" "POST /login (wrong password)"

# --------------------------------------------------
# Test 4: Login with correct credentials -> 200 + token
# --------------------------------------------------
echo ""
echo "--- Test 4: POST /login (correct) -> 200 + token ---"
code=$(curl -s -o "$TMPDIR/login_ok.json" -w '%{http_code}' \
  -X POST "$AUTH_URL/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$test_user\",\"password\":\"$test_pass\"}")
assert_status 200 "$code" "POST /login (correct)"

TOKEN=$(jq -r '.token // empty' "$TMPDIR/login_ok.json")
if [ -n "$TOKEN" ]; then
  pass "Login response contains token (non-empty)"
else
  fail "Login response missing token"
fi

# --------------------------------------------------
# Test 5: GET /verify with Bearer -> 200
# --------------------------------------------------
echo ""
echo "--- Test 5: GET /verify (valid Bearer) -> 200 ---"
code=$(curl -s -o "$TMPDIR/verify_ok.json" -w '%{http_code}' \
  "$AUTH_URL/verify" \
  -H "Authorization: Bearer $TOKEN")
assert_status 200 "$code" "GET /verify (valid Bearer)"
assert_json_field '.status' 'ok' "$TMPDIR/verify_ok.json" "/verify status"

# --------------------------------------------------
# Test 6: GET /verify without token -> 401
# --------------------------------------------------
echo ""
echo "--- Test 6: GET /verify (no token) -> 401 ---"
code=$(curl -s -o "$TMPDIR/verify_noauth.json" -w '%{http_code}' \
  "$AUTH_URL/verify")
assert_status 401 "$code" "GET /verify (no token)"

# --------------------------------------------------
# Test 7: Weak password -> 400
# --------------------------------------------------
echo ""
echo "--- Test 7: POST /register (weak password) -> 400 ---"
code=$(curl -s -o "$TMPDIR/weak_reg.json" -w '%{http_code}' \
  -X POST "$AUTH_URL/register" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"weakuser\",\"password\":\"ab\"}")
assert_status 400 "$code" "POST /register (weak password < 8 chars)"

# --------------------------------------------------
# Test 8: 10 rapid failed logins -> >= one 429 (rate-limit)
# --------------------------------------------------
echo ""
echo "--- Test 8: Rapid failed logins -> rate-limit test ---"
if [ "$SKIP_NO_BINDINGS" = "1" ]; then
  echo "  SKIP: SKIP_NO_BINDINGS=1 (rate-limiter DO may not be available)"
else
  got_429=0
  for i in $(seq 1 10); do
    rc=$(curl -s -o /dev/null -w '%{http_code}' \
      -X POST "$AUTH_URL/login" \
      -H "Content-Type: application/json" \
      -d "{\"username\":\"ratelimit_test_user\",\"password\":\"wrong$i\"}")
    if [ "$rc" -eq 429 ]; then
      got_429=1
      pass "Rate-limit triggered on attempt $i (HTTP 429)"
      break
    fi
  done
  if [ "$got_429" -eq 0 ]; then
    fail "Rate-limit not triggered after 10 rapid failed logins (none returned 429)"
  fi
fi

print_summary "auth"
