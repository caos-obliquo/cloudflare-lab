#!/usr/bin/env bash
# CLI debug tool: hit worker API endpoints with canned payloads.
# Like youtui's test-scrobble / test-validate-metadata subcommands.
#
# Usage:
#   bash scripts/test-api.sh auth health
#   bash scripts/test-api.sh auth register --user test123 --pass TestPass123!
#   bash scripts/test-api.sh auth login --user test123 --pass TestPass123!
#   bash scripts/test-api.sh auth verify --token <hmac-token>
#   bash scripts/test-api.sh gateway health
#   bash scripts/test-api.sh gateway metrics
#   bash scripts/test-api.sh analytics track --event page_view
#
# Environment:
#   AUTH_URL=http://127.0.0.1:8788   (default: http://127.0.0.1:8788)
#   GATEWAY_URL=http://127.0.0.1:8787 (default: http://127.0.0.1:8787)
#   ANALYTICS_URL=http://127.0.0.1:8789 (default: http://127.0.0.1:8789)

set -euo pipefail

AUTH_URL="${AUTH_URL:-http://127.0.0.1:8788}"
GATEWAY_URL="${GATEWAY_URL:-http://127.0.0.1:8787}"
ANALYTICS_URL="${ANALYTICS_URL:-http://127.0.0.1:8789}"

usage() {
  cat <<EOF
Usage: $0 <worker> <action> [options]

Workers:
  auth       Auth worker (register/login/verify/me/health)
  gateway    Gateway worker (health/metrics/logs/livez/readyz)
  analytics  Analytics worker (track/events/summary/health)

Actions:
  auth:
    health                        GET /health
    register  -u USER -p PASS     POST /register
    login     -u USER -p PASS     POST /login
    verify    -t TOKEN            GET /verify (Authorization: Bearer <token>)
    me        -t TOKEN            GET /me (Authorization: Bearer <token>)

  gateway:
    health                        GET /health
    livez                         GET /livez
    readyz                        GET /readyz
    metrics                       GET /metrics
    logs                          GET /logs
    kv                            GET /kv
    d1                            GET /d1

  analytics:
    health                        GET /health
    track     -e EVENT [-u USER]  POST /track
    events                        GET /events
    summary                       GET /summary

Environment:
  AUTH_URL       (default: $AUTH_URL)
  GATEWAY_URL    (default: $GATEWAY_URL)
  ANALYTICS_URL  (default: $ANALYTICS_URL)
  SESSION_SECRET (for token signing, needed for verify/me)

Examples:
  $0 auth health
  $0 auth register --user testuser --pass TestPass123!
  $0 auth login --user testuser --pass TestPass123!
  $0 gateway metrics
  $0 analytics track -e page_view
EOF
  exit 1
}

# ─── Auth Worker ──────────────────────────────────────────

cmd_auth() {
  local action="${1:-}"; shift || true
  case "$action" in
    health)
      curl -sf "$AUTH_URL/health" | jq .
      ;;
    register)
      local user="" pass=""
      while [[ $# -gt 0 ]]; do
        case "$1" in -u|--user) user="$2"; shift 2 ;; -p|--pass) pass="$2"; shift 2 ;; *) shift ;; esac
      done
      [ -n "$user" ] && [ -n "$pass" ] || { echo "Need --user and --pass"; exit 1; }
      curl -sf -X POST "$AUTH_URL/register" \
        -H "Content-Type: application/json" \
        -d "{\"username\":\"$user\",\"password\":\"$pass\"}" | jq .
      ;;
    login)
      local user="" pass=""
      while [[ $# -gt 0 ]]; do
        case "$1" in -u|--user) user="$2"; shift 2 ;; -p|--pass) pass="$2"; shift 2 ;; *) shift ;; esac
      done
      [ -n "$user" ] && [ -n "$pass" ] || { echo "Need --user and --pass"; exit 1; }
      curl -sf -X POST "$AUTH_URL/login" \
        -H "Content-Type: application/json" \
        -d "{\"username\":\"$user\",\"password\":\"$pass\"}" | jq .
      ;;
    verify)
      local token=""
      while [[ $# -gt 0 ]]; do case "$1" in -t|--token) token="$2"; shift 2 ;; *) shift ;; esac; done
      [ -n "$token" ] || { echo "Need --token"; exit 1; }
      curl -sf "$AUTH_URL/verify" -H "Authorization: Bearer $token" | jq .
      ;;
    me)
      local token=""
      while [[ $# -gt 0 ]]; do case "$1" in -t|--token) token="$2"; shift 2 ;; *) shift ;; esac; done
      [ -n "$token" ] || { echo "Need --token"; exit 1; }
      curl -sf "$AUTH_URL/me" -H "Authorization: Bearer $token" | jq .
      ;;
    *)
      echo "Unknown auth action: $action"; usage ;;
  esac
}

# ─── Gateway Worker ───────────────────────────────────────

cmd_gateway() {
  local action="${1:-}"; shift || true
  case "$action" in
    health)  curl -sf "$GATEWAY_URL/health" | jq . ;;
    livez)   curl -sf "$GATEWAY_URL/livez" | jq . ;;
    readyz)  curl -sf "$GATEWAY_URL/readyz" | jq . ;;
    metrics) curl -s "$GATEWAY_URL/metrics" ;;
    logs)    curl -s "$GATEWAY_URL/logs" | jq . ;;
    kv)      curl -sf "$GATEWAY_URL/kv" | jq . ;;
    d1)      curl -sf "$GATEWAY_URL/d1" | jq . ;;
    *)
      echo "Unknown gateway action: $action"; usage ;;
  esac
}

# ─── Analytics Worker ─────────────────────────────────────

cmd_analytics() {
  local action="${1:-}"; shift || true
  case "$action" in
    health)  curl -sf "$ANALYTICS_URL/health" | jq . ;;
    track)
      local event="" user="anon"
      while [[ $# -gt 0 ]]; do
        case "$1" in -e|--event) event="$2"; shift 2 ;; -u|--user) user="$2"; shift 2 ;; *) shift ;; esac
      done
      [ -n "$event" ] || { echo "Need --event"; exit 1; }
      curl -sf -X POST "$ANALYTICS_URL/track" \
        -H "Content-Type: application/json" \
        -d "{\"event\":\"$event\",\"user\":\"$user\"}" | jq .
      ;;
    events)  curl -sf "$ANALYTICS_URL/events" | jq . ;;
    summary) curl -sf "$ANALYTICS_URL/summary" | jq . ;;
    *)
      echo "Unknown analytics action: $action"; usage ;;
  esac
}

# ─── Main ──────────────────────────────────────────────────

[ $# -ge 2 ] || usage

worker="$1"; shift
action="$1"; shift

case "$worker" in
  auth)      cmd_auth "$action" "$@" ;;
  gateway)   cmd_gateway "$action" "$@" ;;
  analytics) cmd_analytics "$action" "$@" ;;
  *)         echo "Unknown worker: $worker"; usage ;;
esac