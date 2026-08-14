#!/usr/bin/env bash
# Checkpoint-4 regression tests: strict fresh-install success semantics.
#
# Covers:
#   - require_subscription_tls() fails closed (exit) instead of warning
#     and continuing subscription-less.
#   - ensure_first_user() is mandatory on a fresh/pending-onboarding run
#     (dies on failure) and NEVER auto-creates/rotates credentials on a
#     repair of an already-accepted install, even with zero users.
#   - a pending-install retry reuses the existing user and mints only a
#     fresh subscription TOKEN (never new VLESS/Hysteria2 credentials).
#   - extract_subscription_url() correctly parses both `user create`'s
#     and `user rotate-token`'s real (differently-worded) output.
#   - the "accepted" manifest write happens strictly after every
#     server-side gate, including the new subscription-through-nginx one.
#   - verify_subscription_through_nginx() never weakens TLS verification
#     (-k) and checks the real REALITY public key/short_id, not just
#     "some bytes came back".
#
# Functional checks source the real install.sh (guarded against
# auto-running main()) and override BIN_DIR/STATE_DIR/DEPLOYMENT_TOML to
# point at fixtures/mocks — no root, network, or real vpn-admin binary
# required. Static checks are plain source inspection.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# ---------------------------------------------------------------------
# Static: require_subscription_tls() fails closed.
# ---------------------------------------------------------------------
echo "--- static: require_subscription_tls() exits (fails closed) rather than warn-and-continue ---"
req_tls_body="$(sed -n '/^require_subscription_tls() {/,/^}/p' "$INSTALL_SH")"
if echo "$req_tls_body" | grep -q 'SUBSCRIPTION_TLS_READY=0'; then
  fail "require_subscription_tls() still sets SUBSCRIPTION_TLS_READY=0 and continues — subscription HTTPS must be mandatory"
else
  ok "require_subscription_tls() no longer has a SUBSCRIPTION_TLS_READY=0 continue-anyway path"
fi
if echo "$req_tls_body" | grep -q '^\s*exit 1\s*$'; then
  ok "require_subscription_tls() exits nonzero when the certificate is unavailable"
else
  fail "require_subscription_tls() does not clearly exit nonzero on certificate failure"
fi

echo
echo "--- static: acceptance_stage() runs verify_subscription_through_nginx() except on a repair of an already-accepted install ---"
acc_body="$(sed -n '/^acceptance_stage() {/,/^}/p' "$INSTALL_SH")"
if echo "$acc_body" | grep -q 'PRIOR_ACCEPTANCE_STATE" = "accepted"' && echo "$acc_body" | grep -q 'verify_subscription_through_nginx'; then
  ok "acceptance_stage() gates verify_subscription_through_nginx() on PRIOR_ACCEPTANCE_STATE"
else
  fail "acceptance_stage() does not clearly gate the subscription-through-nginx check"
fi

echo
echo "--- static: verify_subscription_through_nginx() never weakens TLS verification and checks real REALITY material ---"
vsn_body="$(sed -n '/^verify_subscription_through_nginx() {/,/^}/p' "$INSTALL_SH")"
if echo "$vsn_body" | grep -E '^\s*curl ' | grep -qE -- ' -k(\s|$)| --insecure(\s|$)'; then
  fail "verify_subscription_through_nginx() uses -k/--insecure — this must never weaken TLS verification to pass"
else
  ok "verify_subscription_through_nginx() never uses -k/--insecure"
fi
for needle in 'reality/public.key' 'reality/short_id.txt' 'pbk=' 'sid=' "vless://" 'privatekey'; do
  echo "$vsn_body" | grep -qi -- "$needle" && ok "verify_subscription_through_nginx() checks for '$needle'" || fail "verify_subscription_through_nginx() does not check for '$needle'"
done

echo
echo "--- static: print_status() writes 'accepted' only AFTER every server-side gate (never before) ---"
print_status_body="$(sed -n '/^print_status() {/,/^}/p' "$INSTALL_SH")"
# The FIRST occurrence is the actual gate check; SUBSCRIPTION_FETCH_OK is
# also referenced again later, inside the banner heredoc itself (to show
# a checkmark), which is expected to come AFTER the manifest write.
last_gate_line="$(echo "$print_status_body" | grep -n '\[ "\$SUBSCRIPTION_FETCH_OK" -eq 1 \] || die' | head -n1 | cut -d: -f1)"
write_line="$(echo "$print_status_body" | grep -n 'write_install_state_manifest "accepted"' | head -n1 | cut -d: -f1)"
if [ -n "$last_gate_line" ] && [ -n "$write_line" ] && [ "$last_gate_line" -lt "$write_line" ]; then
  ok "'accepted' is written strictly after the subscription-fetch gate check"
else
  fail "'accepted' write does not clearly follow every gate check (gate=$last_gate_line write=$write_line)"
fi
for gate in VLESS_REALITY_OK HYSTERIA2_OK SUBSCRIPTION_BACKEND_OK SUBSCRIPTION_HTTPS_OK NGINX_OK; do
  echo "$print_status_body" | grep -q "\"\$${gate}\" -eq 1 \] || die" && ok "print_status() hard-gates on $gate" || fail "print_status() does not hard-gate on $gate"
done

echo
echo "--- static: PRIOR_ACCEPTANCE_STATE is captured in preflight_stage before any 'installing'/'pending' manifest write in this run ---"
preflight_body="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
capture_line="$(echo "$preflight_body" | grep -n 'PRIOR_ACCEPTANCE_STATE=' | head -n1 | cut -d: -f1)"
installing_write_line="$(echo "$preflight_body" | grep -n 'write_install_state_manifest "installing"' | head -n1 | cut -d: -f1)"
if [ -n "$capture_line" ] && [ -n "$installing_write_line" ] && [ "$capture_line" -lt "$installing_write_line" ]; then
  ok "PRIOR_ACCEPTANCE_STATE is captured before this run's own manifest writes begin"
else
  fail "PRIOR_ACCEPTANCE_STATE capture does not clearly precede this run's manifest writes"
fi

# ---------------------------------------------------------------------
# Functional: extract_subscription_url() against REAL captured output
# shapes from cmd_user_create/cmd_user_rotate_token (apps/admin/src/
# main.rs) — not invented approximations.
# ---------------------------------------------------------------------
echo
echo "--- functional: extract_subscription_url() parses real 'user create --qr' output ---"
CREATE_OUTPUT='User ID:
  user_61ea95cd-5dfc-4c97-a8b3-a378a7bc7afa

IMPORTANT:
  The User ID above is NOT a credential and NOT your subscription token.

Hiddify subscription URL (this IS the credential — treat it like a password):
  https://sub.example.com:8443/sub/N8ZsLdoMjbs_Sx6K1dyBrJO77zc?format=hiddify

This URL is shown once and cannot be recovered later.

Native sing-box clients (not Hiddify) should use the ?format=singbox variant instead:
  https://sub.example.com:8443/sub/N8ZsLdoMjbs_Sx6K1dyBrJO77zc?format=singbox'
url="$(
  # shellcheck disable=SC1090
  . "$INSTALL_SH"
  extract_subscription_url "$CREATE_OUTPUT"
)"
[ "$url" = "https://sub.example.com:8443/sub/N8ZsLdoMjbs_Sx6K1dyBrJO77zc?format=hiddify" ] \
  && ok "extract_subscription_url() correctly picks the FIRST (hiddify-format) URL from 'user create' output, not the singbox one" \
  || fail "extract_subscription_url() got '$url' from 'user create' output"

echo
echo "--- functional: extract_subscription_url() parses real 'user rotate-token --qr' output (different header text) ---"
ROTATE_OUTPUT='New Hiddify subscription URL for user_61ea95cd-5dfc-4c97-a8b3-a378a7bc7afa:
  https://sub.example.com:8443/sub/rotatedTokenXYZ?format=hiddify

The previous subscription URL now 404s.'
url2="$(
  # shellcheck disable=SC1090
  . "$INSTALL_SH"
  extract_subscription_url "$ROTATE_OUTPUT"
)"
[ "$url2" = "https://sub.example.com:8443/sub/rotatedTokenXYZ?format=hiddify" ] \
  && ok "extract_subscription_url() correctly parses 'user rotate-token' output" \
  || fail "extract_subscription_url() got '$url2' from 'user rotate-token' output"

# ---------------------------------------------------------------------
# Functional: ensure_first_user() against a mocked "vpn" binary,
# exercised via the real function (sourced, not re-implemented).
# ---------------------------------------------------------------------
setup_mock_vpn() {
  local dir="$1"
  mkdir -p "$dir"
  cat > "$dir/vpn" <<'EOF'
#!/usr/bin/env bash
touch "$MOCK_CALL_LOG_DIR/$(date +%s%N)-$$" 2>/dev/null || true
case "$*" in
  *"user list"*)
    echo "$MOCK_USER_LIST_OUTPUT"
    ;;
  *"user create --name"*)
    echo "CREATE_CALLED" >> "$MOCK_CALL_MARKER"
    echo "$MOCK_CREATE_OUTPUT"
    exit "${MOCK_CREATE_RC:-0}"
    ;;
  *"user rotate-token"*)
    echo "ROTATE_CALLED" >> "$MOCK_CALL_MARKER"
    echo "$MOCK_ROTATE_OUTPUT"
    exit "${MOCK_ROTATE_RC:-0}"
    ;;
  *) exit 0 ;;
esac
EOF
  chmod +x "$dir/vpn"
}

run_ensure_first_user() {
  # $1=PRIOR_ACCEPTANCE_STATE $2=MOCK_USER_LIST_OUTPUT
  (
    set -Eeuo pipefail
    MOCKDIR="$TMPDIR_TEST/mockbin-$$-$RANDOM"
    setup_mock_vpn "$MOCKDIR"
    export MOCK_CALL_MARKER="$TMPDIR_TEST/call-marker-$$-$RANDOM"
    : > "$MOCK_CALL_MARKER"
    export MOCK_CALL_LOG_DIR="$TMPDIR_TEST"
    export MOCK_USER_LIST_OUTPUT="$2"
    export MOCK_CREATE_OUTPUT="$CREATE_OUTPUT"
    export MOCK_CREATE_RC="${MOCK_CREATE_RC:-0}"
    export MOCK_ROTATE_OUTPUT="$ROTATE_OUTPUT"
    export MOCK_ROTATE_RC="${MOCK_ROTATE_RC:-0}"
    # shellcheck disable=SC1090
    . "$INSTALL_SH"
    log() { :; }; warn() { :; }
    # These are all read dynamically by ensure_first_user() (sourced
    # above from the real install.sh) — this tool cannot see that use.
    # shellcheck disable=SC2034
    BIN_DIR="$MOCKDIR"
    # shellcheck disable=SC2034
    DEPLOYMENT_TOML="$TMPDIR_TEST/deployment.toml"
    # shellcheck disable=SC2034
    PRIOR_ACCEPTANCE_STATE="$1"
    # shellcheck disable=SC2034
    FIRST_USER_QR_OUTPUT=""
    SUBSCRIPTION_URL=""
    ensure_first_user
    echo "SUBSCRIPTION_URL=$SUBSCRIPTION_URL"
    echo "CALLS=$(cat "$MOCK_CALL_MARKER" | tr '\n' ',' )"
  )
}

echo
echo "--- functional: fresh install (no users, PRIOR_ACCEPTANCE_STATE=none) creates the default user ---"
out="$(run_ensure_first_user "none" "" 2>&1)" || true
if echo "$out" | grep -q 'CALLS=CREATE_CALLED,' && echo "$out" | grep -q 'SUBSCRIPTION_URL=https://sub.example.com:8443/sub/N8ZsLdoMjbs_Sx6K1dyBrJO77zc?format=hiddify'; then
  ok "fresh install creates the default user and captures a usable subscription URL"
else
  fail "fresh install did not create the user / capture the URL as expected: $out"
fi

echo
echo "--- functional: initial user creation failure is FATAL on a fresh install (never warn-and-continue) ---"
rc=0
out="$(MOCK_CREATE_RC=1 run_ensure_first_user "none" "" 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && ok "ensure_first_user() dies (rc=$rc) when initial user creation fails on a fresh install" \
  || fail "ensure_first_user() did NOT fail when initial user creation failed: $out"

echo
echo "--- functional: repair of an ALREADY-ACCEPTED install with ZERO users does NOT auto-create one ---"
out="$(run_ensure_first_user "accepted" "" 2>&1)" || true
if echo "$out" | grep -q '^CALLS=$'; then
  ok "repair with zero users never calls 'user create' or 'user rotate-token' — operator's (deliberately empty) user state is preserved"
else
  fail "repair with zero users unexpectedly touched user state: $out"
fi

echo
echo "--- functional: repair of an ALREADY-ACCEPTED install with existing users does NOT rotate/mint anything ---"
out="$(run_ensure_first_user "accepted" "$(printf "%-20s %-16s %-8s\n" ID NAME ENABLED)
$(printf "%-20s %-16s %-8s\n" user_abc123 existing-name yes)" 2>&1)" || true
if echo "$out" | grep -q '^CALLS=$'; then
  ok "repair with existing users never calls 'user create' or 'user rotate-token'"
else
  fail "repair with existing users unexpectedly touched user state: $out"
fi

echo
echo "--- functional: PENDING retry with an existing user (from a prior failed attempt) mints ONLY a fresh subscription token ---"
out="$(run_ensure_first_user "pending" "$(printf "%-20s %-16s %-8s\n" ID NAME ENABLED)
$(printf "%-20s %-16s %-8s\n" user_abc123 default yes)" 2>&1)" || true
if echo "$out" | grep -q 'CALLS=ROTATE_CALLED,' && echo "$out" | grep -qv 'CREATE_CALLED' \
    && echo "$out" | grep -q 'SUBSCRIPTION_URL=https://sub.example.com:8443/sub/rotatedTokenXYZ?format=hiddify'; then
  ok "pending-retry with an existing user mints a fresh token via rotate-token (not create) and captures a usable URL"
else
  fail "pending-retry with an existing user did not behave as expected: $out"
fi

echo
echo "--- functional: PENDING retry with NO existing user still creates the default user (onboarding not yet complete) ---"
out="$(run_ensure_first_user "pending" "" 2>&1)" || true
if echo "$out" | grep -q 'CALLS=CREATE_CALLED,'; then
  ok "pending-retry with no existing user still creates the default user"
else
  fail "pending-retry with no existing user did not create one: $out"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all fresh-install-acceptance tests passed"
