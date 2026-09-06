#!/usr/bin/env bash
# Regression coverage for failures reproduced by the September 2026
# destructive lifecycle run. These are static contract checks; the real
# service/ACME behavior remains covered by lifecycle-acceptance.sh on a VPS.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ROOT_INSTALL="$REPO_ROOT/install.sh"
PREFLIGHT="$REPO_ROOT/deploy/lib/preflight.sh"
TEMPLATE="$REPO_ROOT/deploy/almalinux/templates/deployment.toml.template"
DEPLOYMENT_RS="$REPO_ROOT/crates/compat-config/src/deployment.rs"
LIFECYCLE="$REPO_ROOT/deploy/almalinux/lifecycle-acceptance.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# A transient ECONNREFUSED from github.com was reproduced on a real VPS.
if grep -q -- '--retry-connrefused' "$ROOT_INSTALL"; then
  ok "bootstrap retries transient connection-refused downloads"
else
  fail "bootstrap lacks --retry-connrefused"
fi
if grep -q 'CURL_NET_FLAGS+=(--retry-connrefused)' "$PREFLIGHT" \
    && grep -q -- '--retry-connrefused' "$PREFLIGHT"; then
  ok "AlmaLinux installer curl flags are augmented with connection-refused retry"
else
  fail "shared preflight does not add --retry-connrefused to installer downloads"
fi

# A freshly rendered config must be current to the same binary that created it.
rust_schema="$(sed -nE 's/^pub const DEPLOYMENT_SCHEMA_VERSION: u32 = ([0-9]+);/\1/p' "$DEPLOYMENT_RS" | head -1)"
template_schema="$(sed -nE 's/^schema_version[[:space:]]*=[[:space:]]*([0-9]+).*/\1/p' "$TEMPLATE" | head -1)"
if [ -n "$rust_schema" ] && [ "$template_schema" = "$rust_schema" ]; then
  ok "fresh deployment.toml template schema matches DeploymentConfig schema ($rust_schema)"
else
  fail "fresh deployment schema drift: template=${template_schema:-missing} rust=${rust_schema:-missing}"
fi

# Lifecycle gate must not manufacture cascades or burn multiple production
# certificates for one destructive run.
if grep -q 'CERT_SNAPSHOT_REMOTE=' "$LIFECYCLE" \
    && grep -q 'capture_cert_for_reuse' "$LIFECYCLE" \
    && grep -q 'restore_cert_for_reuse' "$LIFECYCLE"; then
  ok "lifecycle gate snapshots and reuses one certificate lineage"
else
  fail "lifecycle gate has no certificate reuse contract"
fi
if grep -q 'WORKING_BASELINE_READY' "$LIFECYCLE" \
    && grep -q '\[BLOCKED\]' "$LIFECYCLE" \
    && grep -q 'dependent failures are intentionally not counted as separate bugs' "$LIFECYCLE"; then
  ok "dependent stages are blocked after a missing working baseline"
else
  fail "lifecycle gate can still cascade a baseline failure into fake downstream failures"
fi

# `certbot renew --dry-run` exit 0 with zero attempted lineages is not proof.
if grep -qF 'No simulated renewals were attempted.' "$LIFECYCLE" \
    && grep -q 'certbot_dry_rc' "$LIFECYCLE"; then
  ok "zero-attempt certbot dry-run is explicitly rejected"
else
  fail "certbot dry-run can still false-PASS without testing a lineage"
fi

# The watchdog timer itself must not race the deliberate FAILED-state test.
if grep -q 'systemctl stop vpn-service-watchdog.timer' "$LIFECYCLE" \
    && grep -q 'systemctl start vpn-service-watchdog.timer' "$LIFECYCLE"; then
  ok "crash-loop test suspends and re-arms the watchdog timer"
else
  fail "watchdog timer can still race the crash-loop FAILED-state assertion"
fi

if [ "$failures" -eq 0 ]; then
  echo "release-stability regression checks: PASS"
  exit 0
fi
echo "release-stability regression checks: FAIL ($failures)"
exit 1
