#!/usr/bin/env bash
# Validate all Grafana dashboards in grafana/dashboards/*.json.
#
# Checks:
#   1. JSON parses correctly (jq)
#   2. .uid is a non-empty string
#   3. .title is a non-empty string
#   4. (.panels | length) > 0
#   5. Every panel's datasource uid resolves to a known datasource
#   6. Warns (does not fail) on panels without .gridPos
#
# Datasource reference file: grafana/datasources/datasource.yml
# Falls back to grafana/datasources/datasources.yml if singular doesn't exist.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DASHBOARDS_DIR="${REPO_ROOT}/grafana/dashboards"
ERRORS=0
WARNINGS=0

# Locate datasource file
DS_FILE="${REPO_ROOT}/grafana/datasources/datasource.yml"
if [ ! -f "$DS_FILE" ]; then
  DS_FILE="${REPO_ROOT}/grafana/datasources/datasources.yml"
fi
if [ ! -f "$DS_FILE" ]; then
  echo "ERROR: No datasource file found at grafana/datasources/datasource.yml or datasources.yml"
  exit 1
fi
echo "Datasource reference: ${DS_FILE}"

# Check prerequisites
if ! command -v jq &>/dev/null; then
  echo "FATAL: jq is required but not installed."
  exit 1
fi

echo ""
echo "=== Validating dashboards ==="

for dashboard in "${DASHBOARDS_DIR}"/*.json; do
  [ -f "$dashboard" ] || continue
  name="$(basename "$dashboard")"
  echo ""
  echo "--- ${name} ---"

  # 1. JSON parse check
  if ! jq empty "$dashboard" 2>/dev/null; then
    echo "  FAIL: Invalid JSON"
    ERRORS=$((ERRORS + 1))
    continue
  fi

  # 2. .uid is non-empty string
  uid="$(jq -r '.uid | select(type == "string" and length > 0)' "$dashboard")"
  if [ -z "$uid" ]; then
    echo "  FAIL: .uid is missing or empty"
    ERRORS=$((ERRORS + 1))
  else
    echo "  OK: uid = ${uid}"
  fi

  # 3. .title is non-empty string
  title="$(jq -r '.title | select(type == "string" and length > 0)' "$dashboard")"
  if [ -z "$title" ]; then
    echo "  FAIL: .title is missing or empty"
    ERRORS=$((ERRORS + 1))
  else
    echo "  OK: title = ${title}"
  fi

  # 4. (.panels | length) > 0
  panel_count="$(jq '(.panels | length) // 0' "$dashboard")"
  if [ "$panel_count" -eq 0 ]; then
    echo "  FAIL: No panels found"
    ERRORS=$((ERRORS + 1))
  else
    echo "  OK: ${panel_count} panels"
  fi

  # 5. Panel gridPos check (warn, not fail)
  missing_gridpos="$(jq -r '[.panels[] | select(.gridPos == null or .gridPos == {}) | .id // .title] | join(", ")' "$dashboard")"
  if [ -n "$missing_gridpos" ] && [ "$missing_gridpos" != "" ]; then
    echo "  WARN: Panels missing gridPos: ${missing_gridpos}"
    WARNINGS=$((WARNINGS + 1))
  else
    echo "  OK: All panels have gridPos"
  fi

  # 6. Panel datasource uid verification
  # Extract all unique panel datasource references (non-null)
  ds_refs="$(jq -c '[.panels[] | select(.datasource != null) | .datasource | {uid: .uid, type: .type}] | unique[]' "$dashboard" 2>/dev/null || true)"

  if [ -z "$ds_refs" ] || [ "$ds_refs" = "" ]; then
    echo "  OK: No panel datasource references to verify"
  else
    echo "$ds_refs" | while IFS= read -r ref; do
      ds_uid="$(echo "$ref" | jq -r '.uid // empty')"
      ds_type="$(echo "$ref" | jq -r '.type // empty')"
      [ -z "$ds_uid" ] && continue

      # Resolve template variables like ${DS_PROMETHEUS}
      if echo "$ds_uid" | grep -q '^\${DS_'; then
        # Extract the input variable name
        var_name="$(echo "$ds_uid" | sed 's/^\${DS_//;s/}$//')"
        # Look up the input label in __inputs
        ds_label="$(jq -r --arg vn "$var_name" '.__inputs[] | select(.name == ("DS_" + $vn)) | .label // empty' "$dashboard")"
        if [ -n "$ds_label" ]; then
          # Verify the label appears in datasource file (as name or uid)
          if grep -qi "$ds_label" "$DS_FILE" 2>/dev/null; then
            echo "  OK: Panel datasource ${ds_uid} → label '${ds_label}' found in datasource file"
          else
            echo "  FAIL: Panel datasource ${ds_uid} → label '${ds_label}' NOT found in datasource file"
            ERRORS=$((ERRORS + 1))
          fi
        else
          # Fall back: grep the raw template string in datasource file
          if grep -q "${ds_uid}" "$DS_FILE" 2>/dev/null; then
            echo "  OK: Panel datasource ${ds_uid} found in datasource file"
          else
            echo "  FAIL: Panel datasource ${ds_uid} not resolvable via __inputs and not found in datasource file"
            ERRORS=$((ERRORS + 1))
          fi
        fi
      else
        # Direct uid — grep in datasource file
        if grep -q "${ds_uid}" "$DS_FILE" 2>/dev/null; then
          echo "  OK: Panel datasource uid ${ds_uid} found in datasource file"
        else
          echo "  FAIL: Panel datasource uid ${ds_uid} NOT found in datasource file"
          ERRORS=$((ERRORS + 1))
        fi
      fi
    done
  fi
done

echo ""
echo "=== Summary ==="
echo "Errors: ${ERRORS}  Warnings: ${WARNINGS}"

if [ "$ERRORS" -gt 0 ]; then
  echo "FAILED: ${ERRORS} validation error(s) found"
  exit 1
fi

echo "PASSED: All validations passed"
exit 0
