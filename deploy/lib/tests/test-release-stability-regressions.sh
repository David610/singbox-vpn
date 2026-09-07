#!/usr/bin/env bash
# Regression coverage for failures reproduced by the September 2026
# destructive lifecycle run. These are static contract checks; the real
# service/ACME behavior remains covered by lifecycle-acceptance.sh on a VPS.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ROOT_INSTALL="$REPO_ROOT/install.sh"
ROOT_UNINSTALL="$REPO_ROOT/uninstall.sh"
PREFLIGHT="$REPO_ROOT/deploy/lib/preflight.sh"
UPDATE="$REPO_ROOT/deploy/almalinux/update.sh"
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
if grep -q 'CURL_NET_FLAGS=.*--retry-connrefused' "$UPDATE"; then
  ok "production updater retries transient connection-refused downloads"
else
  fail "production updater lacks --retry-connrefused"
fi
if grep -q 'CURL_NET_FLAGS=.*--retry-connrefused' "$ROOT_UNINSTALL"; then
  ok "network uninstall fallback retries transient connection-refused downloads"
else
  fail "network uninstall fallback lacks --retry-connrefused"
fi
# The lifecycle controller has one network hop before the fetched bootstrap can
# apply its own retry policy. That outer curl must retry too, and its pipeline
# must use pipefail so a failed curl cannot be hidden by an empty `bash` exit 0.
if grep -q 'REMOTE_BOOTSTRAP_CURL_FLAGS=.*--retry-connrefused' "$LIFECYCLE" \
    && grep -qE 'ssh_run(_long)? "set -o pipefail; curl -fsSL \$REMOTE_BOOTSTRAP_CURL_FLAGS' "$LIFECYCLE"; then
  ok "lifecycle bootstrap fetch retries ECONNREFUSED and fails the pipeline closed"
else
  fail "lifecycle outer curl can still flake or false-PASS when bootstrap download fails"
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

# Stage 18 (September 2026 real-VPS run): the entire certbot renewal was
# hidden inside a plain `VAR="$(ssh_run_long ...)"` substitution and
# inherited ssh_run_long's 1200s Rust-build timeout — an already-finished
# renewal looked permanently frozen to the operator. Full behavioral
# coverage (streaming, sentinel, classification, cleanup verification)
# lives in test-lifecycle-acceptance-harness.sh; these are static
# contract checks that the fix's core shape has not regressed.
if grep -qE '^ssh_run_certbot\(\) \{' "$LIFECYCLE" \
    && grep -qE 'timeout -k 10s 360s ssh' "$LIFECYCLE"; then
  ok "stage 18 has its own dedicated, bounded, forced-kill SSH helper (ssh_run_certbot), separate from ssh_run_long"
else
  fail "ssh_run_certbot() is missing or lost its bounded/forced-kill timeout"
fi
if grep -qE "ssh_run_long 'sudo certbot renew --dry-run'" "$LIFECYCLE"; then
  fail "stage 18 regressed back to ssh_run_long's 1200s Rust-build-sized timeout"
else
  ok "stage 18 no longer uses ssh_run_long's Rust-build-sized timeout"
fi
if grep -q 'ssh_run_certbot "\$remote_certbot_cmd" 2>&1 | tee "\$CERTBOT_LOG_FILE"' "$LIFECYCLE" \
    && grep -q 'certbot_dry_rc=\${PIPESTATUS\[0\]}' "$LIFECYCLE"; then
  ok "stage 18 streams certbot output live via tee while still capturing the real exit code via PIPESTATUS"
else
  fail "stage 18 no longer streams certbot output live (or lost correct PIPESTATUS-based exit capture)"
fi
if grep -q "CERTBOT_SENTINEL='__SINGBOX_VPN_CERTBOT_DONE__'" "$LIFECYCLE" \
    && grep -q '__SINGBOX_VPN_CERTBOT_DONE__ rc=%d' "$LIFECYCLE"; then
  ok "stage 18 appends a completion sentinel carrying the real remote exit code"
else
  fail "stage 18 lost its completion sentinel"
fi
# All five required failure classes (task requirement: never collapse
# these into one generic "certbot failed").
CERTBOT_CLASSES=(CERTBOT_REMOTE_TIMEOUT SSH_TRANSPORT_TIMEOUT CERTBOT_EXIT_NONZERO NO_RENEWAL_ATTEMPTED SSH_SESSION_ENDED_UNEXPECTEDLY)
certbot_classes_missing=""
for c in "${CERTBOT_CLASSES[@]}"; do
  grep -q "$c" "$LIFECYCLE" || certbot_classes_missing="$certbot_classes_missing $c"
done
if [ -z "$certbot_classes_missing" ]; then
  ok "stage 18 differentiates all required certbot/SSH failure classes"
else
  fail "stage 18 is missing failure classification(s):$certbot_classes_missing"
fi
if grep -q 'POST_RENEWAL_STATE_INVALID' "$LIFECYCLE" \
    && grep -q 'compgen -G "/run/singbox-vpn-certbot-\*"' "$LIFECYCLE" \
    && grep -q 'systemctl is-active --quiet nginx' "$LIFECYCLE"; then
  ok "stage 18 independently verifies the post-renewal cleanup contract (nginx/markers/firewall)"
else
  fail "stage 18 no longer verifies the post-renewal cleanup contract"
fi

# The watchdog timer itself must not race the deliberate FAILED-state test,
# and a failure to create FAILED must block its dependent assertions instead of
# multiplying one prerequisite failure into several fake product failures.
if grep -q 'systemctl stop vpn-service-watchdog.timer' "$LIFECYCLE" \
    && grep -q 'systemctl start vpn-service-watchdog.timer' "$LIFECYCLE" \
    && grep -q 'failed_state_ready=0' "$LIFECYCLE" \
    && grep -q 'FAILED-state doctor/status/watchdog recovery assertions' "$LIFECYCLE"; then
  ok "crash-loop test is timer-isolated and dependency-aware"
else
  fail "watchdog crash-loop assertions can still race or cascade"
fi

# install.sh's own onboarding transcript (ensure_first_user()/print_status())
# must never render the real subscription URL/QR when
# SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 — the actual suppression
# behavior is covered end-to-end at the source (apps/admin/tests/cli.rs's
# user_create_qr_suppresses_the_credential_when_env_var_set and siblings);
# these are static contract checks that install.sh's own plumbing (which
# decides what gets ECHOED into its own transcript, separately from the
# CLI's own suppression) has not regressed.
ALMALINUX_INSTALL="$REPO_ROOT/deploy/almalinux/install.sh"
if grep -q 'onboarding_secrets_suppressed()' "$ALMALINUX_INSTALL" \
    && grep -q 'safe_onboarding_summary()' "$ALMALINUX_INSTALL" \
    && grep -q 'die_onboarding_failed()' "$ALMALINUX_INSTALL"; then
  ok "install.sh has an onboarding-secret-suppression contract (onboarding_secrets_suppressed/safe_onboarding_summary/die_onboarding_failed)"
else
  fail "install.sh's onboarding-secret-suppression helpers are missing"
fi
# 3 call sites total: die_onboarding_failed()'s own check, plus one in
# each of ensure_first_user()'s two onboarding paths (existing pending
# user, fresh user).
if grep -c 'if onboarding_secrets_suppressed; then' "$ALMALINUX_INSTALL" | grep -qx 3; then
  ok "both ensure_first_user() paths (existing pending user + fresh user) branch FIRST_USER_QR_OUTPUT on suppression"
else
  fail "ensure_first_user() no longer branches both onboarding paths on onboarding_secrets_suppressed — one of them may leak the real credential into print_status()'s transcript"
fi
if grep -q 'env -u SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS .*user rotate-token' "$ALMALINUX_INSTALL" \
    && grep -q 'env -u SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS .*user create' "$ALMALINUX_INSTALL"; then
  ok "both onboarding CLI calls run with the suppression variable unset (so the real subscription URL stays extractable for verify_subscription_through_nginx()'s own internal proof)"
else
  fail "install.sh no longer isolates the suppression variable around its internal user create/rotate-token calls — verify_subscription_through_nginx() would lose its real URL under suppression"
fi
if grep -qF 'die_onboarding_failed "could not mint a subscription token' "$ALMALINUX_INSTALL" \
    && grep -qF 'die_onboarding_failed "initial user creation failed' "$ALMALINUX_INSTALL"; then
  ok "both onboarding failure paths route through die_onboarding_failed (never echo raw \$out — which may already contain the real credential printed just before an unrelated later failure — verbatim when suppressed)"
else
  fail "an onboarding failure path still echoes raw command output directly, which could leak the credential into a suppressed transcript on a failure that happens after the credential was already printed"
fi
if grep -q 'SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1' "$LIFECYCLE"; then
  ok "the lifecycle harness itself sets SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 on install.sh invocations"
else
  fail "the lifecycle harness no longer auto-enables onboarding-secret suppression — its own transcript would carry the real credential again"
fi

if [ "$failures" -eq 0 ]; then
  echo "release-stability regression checks: PASS"
  exit 0
fi
echo "release-stability regression checks: FAIL ($failures)"
exit 1
