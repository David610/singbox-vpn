#!/usr/bin/env bash
# Focused regression tests for installer hardening requirements not
# already covered by the other deploy/lib/tests/*.sh files:
#   - SSH port detection (never assume 22 — docs/FINAL_PRODUCTION_AUDIT.md P0-10)
#   - custom domain is the default; the IP-derived sslip.io convenience
#     hostname requires an explicit opt-in when no interactive prompt is
#     possible (never a silent non-interactive default)
#   - ACME temporary port-80/firewall changes are ownership-tracked and
#     always restored (success, failure, and interrupt paths)
#   - firewall ownership/preservation of pre-existing rules is captured
#     BEFORE any firewall mutation
#
# shellcheck disable=SC2034
# (SSHD_CONFIG_FILE/NONINTERACTIVE/ALLOW_IP_HOSTNAME/DEPLOYMENT_TOML are
# read by preflight_detect_ssh_port()/resolve_host_config() after
# `. "$PREFLIGHT_SH"`/`source "$INSTALL_SH"` — shellcheck cannot see
# that dynamic use.)
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
PREFLIGHT_SH="$REPO_ROOT/deploy/lib/preflight.sh"
FIREWALL_SH="$REPO_ROOT/deploy/almalinux/firewall.sh"
FIREWALL_UFW_SH="$REPO_ROOT/deploy/almalinux/firewall-ufw.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- functional: preflight_detect_ssh_port() reads a non-default port from sshd_config (fixture) ---"
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
FAKE_SSHD_CONFIG="$TMPDIR_TEST/sshd_config"
cat > "$FAKE_SSHD_CONFIG" <<'EOF'
# comment line
Port 2222
EOF
port_out=""
rc=0
port_out="$(
  # shellcheck disable=SC1090
  . "$PREFLIGHT_SH"
  SSHD_CONFIG_FILE="$FAKE_SSHD_CONFIG"
  preflight_detect_ssh_port
)" || rc=$?
if [ "$rc" -eq 0 ] && [ "$port_out" = "2222" ]; then
  ok "preflight_detect_ssh_port() reads Port 2222 from a fixture sshd_config, not the real system one"
else
  fail "preflight_detect_ssh_port() did not detect the fixture non-default port (rc=$rc, got '$port_out')"
fi

echo
echo "--- functional: preflight_detect_ssh_port() prefers 'sshd -T' (effective config) — mocked for port 22 and port 2222 ---"
MOCK_BIN_DIR="$TMPDIR_TEST/mockbin"
mkdir -p "$MOCK_BIN_DIR"
for want_port in 22 2222; do
  cat > "$MOCK_BIN_DIR/sshd" <<EOF
#!/usr/bin/env bash
[ "\$1" = "-T" ] && echo "port $want_port"
EOF
  chmod +x "$MOCK_BIN_DIR/sshd"
  port_out=""
  rc=0
  port_out="$(
    PATH="$MOCK_BIN_DIR:$PATH"
    # shellcheck disable=SC1090
    . "$PREFLIGHT_SH"
    SSHD_CONFIG_FILE="$TMPDIR_TEST/does-not-exist-config"
    preflight_detect_ssh_port
  )" || rc=$?
  if [ "$rc" -eq 0 ] && [ "$port_out" = "$want_port" ]; then
    ok "preflight_detect_ssh_port() reads port $want_port from a mocked 'sshd -T'"
  else
    fail "preflight_detect_ssh_port() did not read mocked 'sshd -T' port $want_port (rc=$rc, got '$port_out')"
  fi
done

echo
echo "--- functional: preflight_detect_ssh_port() FAILS CLOSED (rc=1, nothing printed) when nothing is detectable — no fallback to 22 ---"
rc=0
port_out="$(
  # shellcheck disable=SC1090
  . "$PREFLIGHT_SH"
  SSHD_CONFIG_FILE="$TMPDIR_TEST/does-not-exist"
  preflight_detect_ssh_port
)" || rc=$?
if [ "$rc" -eq 1 ] && [ -z "$port_out" ]; then
  ok "preflight_detect_ssh_port() fails closed (rc=1, no port printed) when undetectable — never guesses 22"
else
  fail "preflight_detect_ssh_port() fail-closed behavior changed (rc=$rc, got '$port_out')"
fi

echo
echo "--- functional: preflight_resolve_ssh_port() also fails closed when detection is inconclusive and no override is given ---"
rc=0
port_out="$(
  # shellcheck disable=SC1090
  . "$PREFLIGHT_SH"
  SSHD_CONFIG_FILE="$TMPDIR_TEST/does-not-exist"
  unset SINGBOX_VPN_SSH_PORT
  preflight_resolve_ssh_port
)" || rc=$?
if [ "$rc" -ne 0 ] && [ -z "$port_out" ]; then
  ok "preflight_resolve_ssh_port() fails closed with no override and inconclusive detection"
else
  fail "preflight_resolve_ssh_port() did not fail closed (rc=$rc, got '$port_out')"
fi

echo
echo "--- functional: preflight_resolve_ssh_port() honours an explicit SINGBOX_VPN_SSH_PORT override even when detection would fail ---"
rc=0
port_out="$(
  # shellcheck disable=SC1090
  . "$PREFLIGHT_SH"
  SSHD_CONFIG_FILE="$TMPDIR_TEST/does-not-exist"
  SINGBOX_VPN_SSH_PORT="2222"
  preflight_resolve_ssh_port
)" || rc=$?
if [ "$rc" -eq 0 ] && [ "$port_out" = "2222" ]; then
  ok "preflight_resolve_ssh_port() uses the explicit SINGBOX_VPN_SSH_PORT override"
else
  fail "preflight_resolve_ssh_port() did not honour the override (rc=$rc, got '$port_out')"
fi

echo
echo "--- functional: preflight_resolve_ssh_port() rejects an invalid SINGBOX_VPN_SSH_PORT override ---"
for bad in "0" "70000" "notaport" "22; rm -rf /"; do
  rc=0
  port_out="$(
    # shellcheck disable=SC1090
    . "$PREFLIGHT_SH"
    SINGBOX_VPN_SSH_PORT="$bad"
    preflight_resolve_ssh_port
  )" 2>/dev/null || rc=$?
  if [ "$rc" -ne 0 ] && [ -z "$port_out" ]; then
    ok "preflight_resolve_ssh_port() rejects invalid override '$bad'"
  else
    fail "preflight_resolve_ssh_port() accepted invalid override '$bad' (rc=$rc, got '$port_out')"
  fi
done

echo
echo "--- static: preflight_detect_ssh_port() has a third (listener) fallback that greps 'ss' output for an sshd/ssh-owned socket ---"
# The listener fallback needs a process actually named sshd/ssh bound to
# a real port to exercise end-to-end, which this sandbox cannot produce
# without root and a real sshd — that combination is UNVERIFIED here
# (see docs/IMPLEMENTATION_STATUS.md); the config-file and override
# paths above ARE exercised functionally. This checks the code path
# exists and greps for the right process names.
if grep -q "ss -H -lntp 2>/dev/null | grep -E 'sshd|\"ssh\"'" "$PREFLIGHT_SH"; then
  ok "preflight_detect_ssh_port() has a listener-based fallback matching sshd/ssh-owned sockets (live-listener path itself is UNVERIFIED without a real sshd — see docs/IMPLEMENTATION_STATUS.md)"
else
  fail "preflight_detect_ssh_port() lost its listener-based fallback"
fi

echo
echo "--- static: firewall.sh/firewall-ufw.sh both call preflight_resolve_ssh_port() and never hardcode 22 as the only rule ---"
for f in "$FIREWALL_SH" "$FIREWALL_UFW_SH"; do
  name="$(basename "$f")"
  if grep -q 'preflight_resolve_ssh_port' "$f"; then
    ok "$name calls preflight_resolve_ssh_port()"
  else
    fail "$name does not call preflight_resolve_ssh_port() — would assume port 22"
  fi
  if grep -qE '\[ "\$SSH_PORT" != "22" \]' "$f"; then
    ok "$name adds an explicit rule for a detected non-default SSH port"
  else
    fail "$name has no non-default-SSH-port rule path"
  fi
  if grep -qE 'preflight_resolve_ssh_port\)"\s*\|\|\s*die' "$f"; then
    ok "$name fails closed (die) rather than falling back to 22 when SSH port resolution fails"
  else
    fail "$name does not fail closed on inconclusive SSH port resolution"
  fi
done

echo
echo "--- static: install.sh resolves the SSH port in preflight_stage (stage 1), strictly before packages_stage ever activates firewalld ---"
if grep -qE '^\s*resolve_ssh_port$' "$INSTALL_SH"; then
  ok "install.sh calls resolve_ssh_port()"
else
  fail "install.sh no longer calls resolve_ssh_port()"
fi
preflight_body_ssh="$(sed -n '/^preflight_stage() {/,/^}/p' "$INSTALL_SH")"
if echo "$preflight_body_ssh" | grep -qE '^\s*resolve_ssh_port$'; then
  ok "resolve_ssh_port() runs inside preflight_stage (stage 1)"
else
  fail "resolve_ssh_port() does not run inside preflight_stage"
fi
if grep -qE -- '--ssh-port\) SINGBOX_VPN_SSH_PORT=' "$INSTALL_SH"; then
  ok "--ssh-port is a real recognized CLI flag wired to SINGBOX_VPN_SSH_PORT"
else
  fail "--ssh-port flag is not wired into parse_cli_args()"
fi

echo
echo "--- static: firewalld is only ever activated via activate_firewalld_ssh_safe(), which stages+verifies the confirmed SSH port BEFORE firewalld ever starts (offline, no fail-open) ---"
if grep -qE 'systemctl enable --now firewalld' "$INSTALL_SH"; then
  fail "install.sh still contains a bare 'systemctl enable --now firewalld' — this is exactly the unsafe ordering (activates before SSH port is allowed)"
else
  ok "install.sh no longer bare-activates firewalld outside the SSH-safe helper"
fi
activate_body="$(sed -n '/^activate_firewalld_ssh_safe() {/,/^}/p' "$INSTALL_SH")"
# The REAL invariant: if firewalld is inactive, the SSH allow must be
# staged (via firewall-offline-cmd, which needs no running daemon) and
# positively verified BEFORE 'systemctl start firewalld' ever runs — not
# added afterward, however "immediately". A test that blesses
# start-then-add encodes the wrong invariant and would pass code with a
# real (if brief) default-deny window on a custom SSH port.
offline_add_line="$(echo "$activate_body" | grep -n 'firewall-offline-cmd.*add-service=ssh\|firewall-offline-cmd.*add-port="\${SSH_PORT}' | head -n1 | cut -d: -f1)"
offline_query_line="$(echo "$activate_body" | grep -n 'firewall-offline-cmd.*query-service=ssh\|firewall-offline-cmd.*query-port="\${SSH_PORT}' | head -n1 | cut -d: -f1)"
start_line="$(echo "$activate_body" | grep -n '^\s*systemctl start firewalld' | head -n1 | cut -d: -f1)"
runtime_query_line="$(echo "$activate_body" | grep -n 'firewall-cmd.*query-service=ssh\|firewall-cmd.*query-port="\${SSH_PORT}' | head -n1 | cut -d: -f1)"
if [ -n "$offline_add_line" ] && [ -n "$offline_query_line" ] && [ -n "$start_line" ] && [ -n "$runtime_query_line" ] \
    && [ "$offline_add_line" -lt "$start_line" ] && [ "$offline_query_line" -lt "$start_line" ] \
    && [ "$start_line" -lt "$runtime_query_line" ]; then
  ok "activate_firewalld_ssh_safe() stages the SSH rule via firewall-offline-cmd and verifies it offline, THEN starts firewalld, THEN verifies the runtime rule — no unsafe activate-then-allow window"
else
  fail "activate_firewalld_ssh_safe() does not clearly order offline-stage(add=$offline_add_line,query=$offline_query_line) before start($start_line) before runtime-verify($runtime_query_line)"
fi
if echo "$activate_body" | grep -qE 'firewall-cmd --zone="\$zone" --add-(service=ssh|port="\$\{SSH_PORT\}/tcp") >/dev/null 2>&1 \|\| true'; then
  fail "activate_firewalld_ssh_safe() still fail-opens an SSH-allow rule with '|| true' — a failed add would be silently treated as success"
else
  ok "activate_firewalld_ssh_safe() has no fail-open ('|| true') SSH-allow rule"
fi
if echo "$activate_body" | grep -q 'command -v firewall-offline-cmd'; then
  ok "activate_firewalld_ssh_safe() fails closed if firewall-offline-cmd is unavailable, rather than falling back to the unsafe start-then-add ordering"
else
  fail "activate_firewalld_ssh_safe() does not guard against a missing firewall-offline-cmd"
fi
if echo "$activate_body" | grep -q 'is-active --quiet firewalld'; then
  ok "activate_firewalld_ssh_safe() checks whether firewalld was already active and preserves its existing config in that case"
else
  fail "activate_firewalld_ssh_safe() does not guard against re-activating an already-active (pre-existing) firewalld"
fi
install_deps_rhel_body2="$(sed -n '/^install_dependencies_rhel() {/,/^}/p' "$INSTALL_SH")"
if echo "$install_deps_rhel_body2" | grep -q 'activate_firewalld_ssh_safe'; then
  ok "install_dependencies_rhel() (packages_stage) activates firewalld only through the SSH-safe helper"
else
  fail "install_dependencies_rhel() does not call activate_firewalld_ssh_safe()"
fi

echo
echo "--- static: firewall ownership of pre-existing state is captured before any firewall mutation ---"
main_body="$(sed -n '/^main() {/,/^}/p' "$INSTALL_SH")"
packages_line="$(echo "$main_body" | grep -n '^\s*packages_stage$' | head -n1 | cut -d: -f1)"
firewall_line="$(echo "$main_body" | grep -n '^\s*firewall_stage$' | head -n1 | cut -d: -f1)"
if [ -n "$packages_line" ] && [ -n "$firewall_line" ] && [ "$packages_line" -lt "$firewall_line" ]; then
  ok "main() runs packages_stage strictly before firewall_stage"
else
  fail "packages_stage does not run strictly before firewall_stage in main() (packages=$packages_line firewall=$firewall_line)"
fi
install_deps_rhel_body="$(sed -n '/^install_dependencies_rhel() {/,/^}/p' "$INSTALL_SH")"
install_deps_debian_body="$(sed -n '/^install_dependencies_debian() {/,/^}/p' "$INSTALL_SH")"
if echo "$install_deps_rhel_body" | grep -q 'ownership_set_baseline_once FIREWALLD_PRE_INSTALLED' \
    && echo "$install_deps_debian_body" | grep -q 'ownership_set_baseline_once UFW_PRE_ENABLED'; then
  ok "install_dependencies_rhel()/install_dependencies_debian() (packages_stage's real work, stage 2) capture pre-existing firewalld/ufw state before firewall_stage (stage ~15) ever runs"
else
  fail "pre-existing firewall state is not captured inside packages_stage's install_dependencies_* functions"
fi

echo
echo "--- static: ACME temporary TCP/80 rule is ownership-tracked and always closed (success, failure, interrupt) ---"
acme_body="$(sed -n '/^attempt_automatic_certbot() {/,/^}/p' "$INSTALL_SH")"
if echo "$acme_body" | grep -q 'firewall_open_port_80_temp' && echo "$acme_body" | grep -q 'firewall_close_port_80_temp'; then
  ok "attempt_automatic_certbot() opens AND closes the temporary TCP/80 rule"
else
  fail "attempt_automatic_certbot() does not both open and close the temporary TCP/80 rule"
fi
if echo "$acme_body" | grep -q "trap 'acme_cleanup; on_interrupt' INT" && echo "$acme_body" | grep -q "trap 'acme_cleanup; on_terminate' TERM"; then
  ok "attempt_automatic_certbot() restores state (acme_cleanup) AND still routes through the global rollback handler (on_interrupt/on_terminate) if interrupted (INT/TERM) -- not a bare exit that would skip rollback for a fresh install"
else
  fail "attempt_automatic_certbot() has no interrupt-safe cleanup trap, or it bypasses the global on_interrupt/on_terminate rollback handler"
fi
if echo "$acme_body" | grep -q 'trap - INT TERM'; then
  fail "attempt_automatic_certbot() clears INT/TERM to no handler (trap - INT TERM) instead of restoring on_interrupt/on_terminate -- a signal after this function returns would again skip rollback"
else
  ok "attempt_automatic_certbot() restores the global on_interrupt/on_terminate handler on exit rather than clearing it"
fi
if echo "$acme_body" | grep -q 'nginx_was_stopped=1' && echo "$acme_body" | grep -q 'systemctl start nginx'; then
  ok "attempt_automatic_certbot() restarts nginx if it stopped it for the ACME challenge"
else
  fail "attempt_automatic_certbot() does not restore nginx after stopping it"
fi

echo
echo "--- static: custom domain is the default; --allow-ip-hostname/SINGBOX_VPN_ALLOW_IP_HOSTNAME is required for a silent non-interactive IP-derived fallback ---"
resolve_host_body="$(sed -n '/^resolve_host_config() {/,/^}/p' "$INSTALL_SH")"
if echo "$resolve_host_body" | grep -q 'ALLOW_IP_HOSTNAME'; then
  ok "resolve_host_config() checks ALLOW_IP_HOSTNAME before silently falling back to sslip.io"
else
  fail "resolve_host_config() has no explicit-opt-in gate for the IP-derived hostname"
fi
if echo "$resolve_host_body" | grep -q 'die "no PUBLIC_HOST/--domain given'; then
  ok "resolve_host_config() refuses (die) rather than silently defaulting when no prompt was possible and no opt-in was given"
else
  fail "resolve_host_config() does not refuse the silent IP-derived fallback"
fi
if grep -qE -- '--allow-ip-hostname\) ALLOW_IP_HOSTNAME=1' "$INSTALL_SH"; then
  ok "--allow-ip-hostname is a real recognized CLI flag"
else
  fail "--allow-ip-hostname flag is not wired into parse_cli_args()"
fi

echo
echo "--- functional: resolve_host_config() dies without --allow-ip-hostname when non-interactive and no domain given ---"
rc=0
out="$( {
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  PUBLIC_HOST=""
  NONINTERACTIVE=1
  ALLOW_IP_HOSTNAME=0
  DEPLOYMENT_TOML="$TMPDIR_TEST/does-not-exist-deployment.toml"
  resolve_host_config
} 2>&1 )" || rc=$?
if [ "$rc" -ne 0 ] && echo "$out" | grep -q 'allow-ip-hostname'; then
  ok "resolve_host_config() dies (non-zero) and names --allow-ip-hostname as the fix, when non-interactive with no domain and no opt-in"
else
  fail "resolve_host_config() did not refuse as expected (rc=$rc): $out"
fi

echo
echo "--- functional: resolve_host_config() succeeds with --allow-ip-hostname (SINGBOX_VPN_ALLOW_IP_HOSTNAME=1) even non-interactively ---"
rc=0
out="$( {
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  PUBLIC_HOST=""
  NONINTERACTIVE=1
  ALLOW_IP_HOSTNAME=1
  DEPLOYMENT_TOML="$TMPDIR_TEST/does-not-exist-deployment.toml"
  preflight_detect_public_ip() { echo "203.0.113.1"; }
  resolve_host_config
  echo "PUBLIC_HOST=$PUBLIC_HOST"
} 2>&1 )" || rc=$?
if [ "$rc" -eq 0 ] && echo "$out" | grep -q 'PUBLIC_HOST=203-0-113-1.sslip.io'; then
  ok "resolve_host_config() with SINGBOX_VPN_ALLOW_IP_HOSTNAME=1 proceeds to the sslip.io fallback and succeeds"
else
  fail "resolve_host_config() with the explicit opt-in did not behave as expected (rc=$rc): $out"
fi

echo
echo "--- functional: resolve_host_config() with an operator-supplied domain never touches ALLOW_IP_HOSTNAME at all ---"
rc=0
out="$( {
  # shellcheck disable=SC1090
  source "$INSTALL_SH"
  PUBLIC_HOST="vpn.example.com"
  NONINTERACTIVE=1
  ALLOW_IP_HOSTNAME=0
  DEPLOYMENT_TOML="$TMPDIR_TEST/does-not-exist-deployment.toml"
  resolve_host_config
  echo "PUBLIC_HOST=$PUBLIC_HOST"
} 2>&1 )" || rc=$?
if [ "$rc" -eq 0 ] && echo "$out" | grep -q 'PUBLIC_HOST=vpn.example.com'; then
  ok "an operator-supplied --domain always works non-interactively, no opt-in needed"
else
  fail "operator-supplied domain path regressed (rc=$rc): $out"
fi

echo
echo "--- static: idempotent rerun never regenerates REALITY secrets outside an explicit --rotate ---"
if grep -q 'Commands::Init { rotate }' apps/admin/src/main.rs 2>/dev/null || grep -q 'Commands::Init { rotate }' "$REPO_ROOT/apps/admin/src/main.rs"; then
  ok "vpn-admin init takes an explicit --rotate flag (not implied by re-running init)"
else
  fail "could not confirm vpn-admin init's --rotate gating"
fi

echo
echo "--- static: idempotency — install.sh's persist/render helpers refuse to clobber an already-committed config ---"
if grep -q 'refuses to rewrite an existing deployment.toml' "$INSTALL_SH"; then
  ok "install.sh documents/enforces that render_deployment_toml never rewrites an existing deployment.toml"
else
  fail "no evidence install.sh protects an already-committed deployment.toml from being rewritten"
fi

if [ "$failures" -eq 0 ]; then
  echo
  echo "all installer-hardening tests passed"
else
  echo
  echo "$failures test(s) FAILED"
  exit 1
fi
