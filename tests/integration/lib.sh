#!/usr/bin/env bash
# Integration test assertion library.
# Source via: source "$(dirname "$0")/lib.sh"

set -euo pipefail

PASS=0
FAIL=0
FAILED_NAMES=()

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAILED_NAMES+=("$1"); FAIL=$((FAIL + 1)); }

# Guard: skip if required command missing.
# Usage: require_cmd jq "install jq via apt/brew/nix"
require_cmd() {
  local cmd=$1 msg=$2
  if ! command -v "$cmd" &>/dev/null; then
    echo "  SKIP: '$cmd' not found — $msg"
    return 1
  fi
}

# Assert HTTP status code matches expected.
# Usage: assert_status 200 "$(curl -s -o /dev/null -w '%{http_code}' "$URL")"
assert_status() {
  local expected=$1 actual=$2 label="${3:-}"
  [ -n "$label" ] || label="status"
  if [ "$actual" -eq "$expected" ] 2>/dev/null; then
    pass "$label — expected $expected, got $actual"
  else
    fail "$label — expected HTTP $expected, got $actual"
  fi
}

# Assert response header value matches a grep -E pattern.
# Extracts the first matching header line, checks pattern against the value part.
# Usage: assert_header "Content-Type" "application/json" "$header_file"
assert_header() {
  local name=$1 pattern=$2 file=$3
  local line val
  line=$(grep -im1 "^$name:" "$file" 2>/dev/null || true)
  if [ -z "$line" ]; then
    fail "Header '$name' — not found in file"
    return
  fi
  val=$(echo "$line" | sed 's/^[^:]*:[[:space:]]*//' | tr -d '\r')
  if echo "$val" | grep -qE "$pattern"; then
    pass "Header '$name' value matches '$pattern' (got: '$val')"
  else
    fail "Header '$name' — expected pattern /$pattern/, got: '$val'"
  fi
}

# Assert JSON field matches expected value (string equality) via jq.
# Usage: assert_json_field '.status' 'ok' "$response_file"
assert_json_field() {
  local expr=$1 expected=$2 file=$3 label="${4:-}"
  [ -n "$label" ] || label="json field $expr"
  if ! [ -f "$file" ]; then
    fail "$label — file not found: $file"
    return
  fi
  local actual
  actual=$(jq -r "$expr" "$file" 2>/dev/null) || {
    fail "$label — jq error on expr '$expr'"
    return
  }
  if [ "$actual" = "$expected" ]; then
    pass "$label — $expr = '$expected'"
  else
    fail "$label — $expr expected '$expected', got '$actual'"
  fi
}

# Assert JSON field matches a grep-compatible pattern.
# Usage: assert_json_matches '.status' 'ok|healthy' "$file"
assert_json_matches() {
  local expr=$1 pattern=$2 file=$3 label="${4:-}"
  [ -n "$label" ] || label="json field $expr ~ $pattern"
  if ! [ -f "$file" ]; then
    fail "$label — file not found: $file"
    return
  fi
  local actual
  actual=$(jq -r "$expr" "$file" 2>/dev/null) || {
    fail "$label — jq error on expr '$expr'"
    return
  }
  if echo "$actual" | grep -qE "$pattern"; then
    pass "$label — $expr matches '$pattern'"
  else
    fail "$label — $expr expected pattern '$pattern', got '$actual'"
  fi
}

# Assert response body contains a substring.
# Usage: assert_body_contains "cloudflare_requests_total" "$response_body"
assert_body_contains() {
  local needle=$1 body=$2 label="${3:-}"
  [ -n "$label" ] || label="body contains '$needle'"
  if echo "$body" | grep -qF "$needle"; then
    pass "$label"
  else
    fail "$label — not found in body"
  fi
}

# Assert file body (already saved) contains a regex pattern.
# Usage: assert_body_matches '^[a-zA-Z_]' "$file"
assert_body_matches() {
  local pattern=$1 file=$2 label="${3:-}"
  [ -n "$label" ] || label="body matches pattern"
  if ! [ -f "$file" ]; then
    fail "$label — file not found: $file"
    return
  fi
  if grep -qE "$pattern" "$file"; then
    pass "$label"
  else
    fail "$label — no match for /$pattern/ in file"
  fi
}

# Print summary and exit 1 if any failures.
print_summary() {
  local suite_name="${1:-integration}"
  echo ""
  echo "=========================================="
  echo "Suite: $suite_name"
  echo "Results: $PASS passed, $FAIL failed"
  if [ "${#FAILED_NAMES[@]}" -gt 0 ]; then
    echo "Failed tests:"
    local f
    for f in "${FAILED_NAMES[@]}"; do echo "  - $f"; done
  fi
  echo "=========================================="
  [ "$FAIL" -eq 0 ]
}
