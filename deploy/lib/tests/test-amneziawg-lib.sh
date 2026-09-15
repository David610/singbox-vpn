#!/usr/bin/env bash
# deploy/lib/amneziawg.sh against mocked tools in a throwaway directory:
# pinned-version loading, fail-closed commit/checksum verification, hook
# behaviour (setconf, addresses, forwarding, NAT), firewall ownership and a
# reversible uninstall. Never touches the host: every tool the script calls
# is a recording mock first on PATH, and every path is redirected.
# The library is sourced from a variable path in subshells on purpose.
# shellcheck disable=SC1090,SC2034
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LIB="$REPO_ROOT/deploy/lib/amneziawg.sh"
VERSIONS="$REPO_ROOT/deploy/lib/versions.env"
RUST_PINS="$REPO_ROOT/crates/compat-config/src/amneziawg.rs"
UNIT="$REPO_ROOT/deploy/almalinux/systemd/vpn-amneziawg.service"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
MOCK="$WORK/bin"
CALLS="$WORK/calls.log"
mkdir -p "$MOCK" "$WORK/state" "$WORK/lib" "$WORK/uapi" "$WORK/usrbin" "$WORK/sysctl.d"
: > "$CALLS"

mock() { # name [body]
  printf '#!/usr/bin/env bash\necho "%s $*" >> "%s"\n%s\n' "$1" "$CALLS" "${2:-exit 0}" > "$MOCK/$1"
  chmod +x "$MOCK/$1"
}
for tool in ip sysctl systemctl iptables ip6tables firewall-cmd ufw; do mock "$tool"; done
mock nft 'cat >/dev/null; exit 0'

run_lib() { # subcommand...
  env PATH="$MOCK:$PATH" \
    AWG_STATE_DIR="$WORK/state" AWG_RUNTIME_ENV="$WORK/state/runtime.env" \
    AWG_LIB_STATE="$WORK/lib" AWG_BIN_DIR="$WORK/usrbin" AWG_UAPI_DIR="$WORK/uapi" \
    AWG_SYSCTL_DROPIN="$WORK/sysctl.d/99-awg.conf" AWG_UNIT_DST="$WORK/unit.service" \
    AWG_SOCKET_WAIT_SECONDS=1 SYSTEMCTL="$MOCK/systemctl" "$@"
}

echo "--- static: pins agree across versions.env, Rust and the unit ---"
for key in AMNEZIAWG_GO_VERSION AMNEZIAWG_GO_COMMIT AMNEZIAWG_TOOLS_VERSION AMNEZIAWG_TOOLS_COMMIT; do
  value="$(sed -n "s/^${key}=//p" "$VERSIONS")"
  if [ -n "$value" ] && grep -q "pub const ${key}: &str = \"${value}\";" "$RUST_PINS"; then
    ok "$key=$value matches crates/compat-config/src/amneziawg.rs"
  else
    fail "$key missing or differs between versions.env and amneziawg.rs"
  fi
done
grep -q '^Environment=LOG_LEVEL=error$' "$UNIT" && ok "unit keeps upstream error-level logging" || fail "unit must set LOG_LEVEL=error"
grep -q -- '--foreground' "$UNIT" && ok "unit runs amneziawg-go in the foreground" || fail "unit must use --foreground"
if grep -qiE 'curl[^|]*\|[[:space:]]*(ba)?sh' "$LIB"; then fail "amneziawg.sh pipes a download into a shell"; else ok "no curl|sh in amneziawg.sh"; fi
(
  . "$REPO_ROOT/deploy/lib/node-identity.sh"
  rust_marker="$(sed -n 's/^pub const AMNEZIAWG_CAPABILITY: &str = "\(.*\)";$/\1/p' "$REPO_ROOT/crates/compat-config/src/deployment.rs")"
  [ -n "$rust_marker" ] && [ "$rust_marker" = "$AMNEZIAWG_CAPABILITY" ]
) && ok "AMNEZIAWG_CAPABILITY agrees between deployment.rs and node-identity.sh" || fail "AMNEZIAWG_CAPABILITY differs between Rust and shell"
grep -q 'admin_output_declares_amneziawg "\$precheck_output"' "$REPO_ROOT/deploy/almalinux/update.sh" \
  && ok "update.sh refuses to switch an AmneziaWG node to a build without AmneziaWG support" \
  || fail "update.sh lacks the AmneziaWG capability guard"
grep -q 'deploy/lib/amneziawg.sh' "$REPO_ROOT/deploy/almalinux/uninstall.sh" \
  && ok "uninstall.sh removes the AmneziaWG data plane" || fail "uninstall.sh does not call amneziawg.sh uninstall"
(
  . "$REPO_ROOT/deploy/lib/node-identity.sh"
  printf 'schema_version = 3\n\n[amneziawg]\nlisten_port = 51820\n' > "$WORK/awg-on.toml"
  printf 'schema_version = 2\n# [amneziawg] disabled\n' > "$WORK/awg-off.toml"
  deployment_enables_amneziawg "$WORK/awg-on.toml" && ! deployment_enables_amneziawg "$WORK/awg-off.toml"
) && ok "deployment_enables_amneziawg detects only a real [amneziawg] table" || fail "deployment_enables_amneziawg misdetects"

echo
echo "--- functional: load_versions fails closed on missing or short pins ---"
bad="$WORK/versions-bad.env"
grep -v '^AMNEZIAWG_GO_COMMIT=' "$VERSIONS" > "$bad"
if (AWG_VERSIONS_ENV="$bad"; source "$LIB"; load_versions) >/dev/null 2>&1; then
  fail "missing AMNEZIAWG_GO_COMMIT accepted"
else
  ok "missing AMNEZIAWG_GO_COMMIT refused"
fi
sed 's/^AMNEZIAWG_TOOLS_COMMIT=.*/AMNEZIAWG_TOOLS_COMMIT=v3.1/' "$VERSIONS" > "$bad"
if (AWG_VERSIONS_ENV="$bad"; source "$LIB"; load_versions) >/dev/null 2>&1; then
  fail "non-commit AMNEZIAWG_TOOLS_COMMIT accepted"
else
  ok "tag name in place of a commit refused"
fi

echo
echo "--- functional: clone_pinned refuses a tag that resolves elsewhere ---"
mock git 'case "$1" in -c) mkdir -p "${@: -1}"; exit 0 ;; -C) echo 0000000000000000000000000000000000000000; exit 0 ;; esac'
if (PATH="$MOCK:$PATH"; source "$LIB"; clone_pinned https://example.invalid/x v1 1111111111111111111111111111111111111111 "$WORK/clone") >/dev/null 2>&1; then
  fail "moved tag accepted"
else
  ok "tag resolving to a different commit is refused"
fi

echo
echo "--- functional: verify_sha256 refuses tampered bytes ---"
printf 'toolchain' > "$WORK/go.tgz"
good_sum="$(sha256sum "$WORK/go.tgz" | awk '{print $1}')"
if (source "$LIB"; verify_sha256 "$WORK/go.tgz" "$good_sum") >/dev/null 2>&1; then ok "matching checksum accepted"; else fail "matching checksum refused"; fi
if (source "$LIB"; verify_sha256 "$WORK/go.tgz" "${good_sum/a/b}") >/dev/null 2>&1; then fail "mismatching checksum accepted"; else ok "mismatching checksum refused"; fi

echo
echo "--- functional: pre-start refuses missing runtime env and unsafe config ---"
if run_lib bash "$LIB" pre-start >/dev/null 2>&1; then fail "pre-start without runtime.env succeeded"; else ok "pre-start requires runtime.env"; fi
cat > "$WORK/state/runtime.env" <<EOF
AWG_INTERFACE=awg0
AWG_LISTEN_PORT=51820
AWG_ADDRESS_V4=10.66.0.1/16
AWG_SUBNET_V4=10.66.0.0/16
AWG_ADDRESS_V6=fd66::1/64
AWG_SUBNET_V6=fd66::/64
AWG_MTU=1380
AWG_SETCONF=$WORK/state/awg0.conf
EOF
printf '[Interface]\nPrivateKey = x\n' > "$WORK/state/awg0.conf"
chmod 0644 "$WORK/state/awg0.conf"
for b in amneziawg-go awg; do printf '#!/bin/sh\necho "%s $*" >> "%s"\n' "$b" "$CALLS" > "$WORK/usrbin/$b"; chmod +x "$WORK/usrbin/$b"; done
if run_lib bash "$LIB" pre-start >/dev/null 2>&1; then fail "world-readable server config accepted"; else ok "config with mode 0644 refused"; fi
chmod 0600 "$WORK/state/awg0.conf"
if run_lib bash "$LIB" pre-start >/dev/null 2>&1; then ok "pre-start accepts a 0600 config with binaries present"; else fail "pre-start refused a valid setup"; fi
printf 'AWG_INTERFACE=$(reboot)\n' > "$WORK/bad-runtime.env"
if run_lib env AWG_RUNTIME_ENV="$WORK/bad-runtime.env" bash "$LIB" pre-start >/dev/null 2>&1; then fail "hostile interface name accepted"; else ok "hostile interface name refused"; fi

echo
echo "--- functional: post-start waits for the UAPI socket, then configures ---"
: > "$CALLS"
if run_lib bash "$LIB" post-start >/dev/null 2>&1; then fail "post-start without a UAPI socket succeeded"; else ok "post-start fails when amneziawg-go never created its socket"; fi
python3 - "$WORK/uapi/awg0.sock" <<'PY'
import socket, sys
s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])
PY
: > "$CALLS"
if run_lib bash "$LIB" post-start >/dev/null 2>&1; then ok "post-start succeeds once the socket exists"; else fail "post-start failed with a socket present"; fi
grep -q "^awg setconf awg0 $WORK/state/awg0.conf$" "$CALLS" && ok "awg setconf applied the rendered file" || fail "awg setconf not called with the rendered file"
grep -q '^ip address replace 10.66.0.1/16 dev awg0$' "$CALLS" && ok "IPv4 address set" || fail "IPv4 address not set"
grep -q '^ip -6 address replace fd66::1/64 dev awg0$' "$CALLS" && ok "IPv6 address set" || fail "IPv6 address not set"
grep -q '^ip link set dev awg0 mtu 1380 up$' "$CALLS" && ok "MTU applied and link up" || fail "MTU/link not applied"
grep -q 'net.ipv4.ip_forward=1' "$WORK/sysctl.d/99-awg.conf" && grep -q 'net.ipv6.conf.all.forwarding=1' "$WORK/sysctl.d/99-awg.conf" \
  && ok "forwarding drop-in written" || fail "forwarding drop-in missing"
[ -f "$WORK/lib/amneziawg-sysctl-baseline.env" ] && ok "sysctl baseline captured for rollback" || fail "no sysctl baseline"
grep -q '^nft -f -$' "$CALLS" && ok "NAT installed via nftables table" || fail "nftables NAT not installed"

echo
echo "--- functional: iptables fallback appends each rule once, with our comment ---"
rm -f "$MOCK/nft"
mock iptables 'case "$*" in *" -C "*) exit 1 ;; *" -S"*) exit 0 ;; esac; exit 0'
: > "$CALLS"
run_lib bash -c "source '$LIB'; load_runtime_env; nat_add" >/dev/null 2>&1 && ok "iptables NAT add ran" || fail "iptables NAT add failed"
[ "$(grep -c '^iptables -t nat -A POSTROUTING -s 10.66.0.0/16 ! -o awg0 -j MASQUERADE -m comment --comment singbox-vpn-amneziawg$' "$CALLS")" = 1 ] \
  && ok "MASQUERADE appended once with our comment" || fail "MASQUERADE rule not appended exactly once"
mock iptables 'case "$*" in *" -C "*) exit 0 ;; esac; exit 0'
: > "$CALLS"
run_lib bash -c "source '$LIB'; load_runtime_env; nat_add" >/dev/null 2>&1
if grep -q ' -A ' "$CALLS"; then fail "existing rules were appended again"; else ok "existing rules are not duplicated"; fi

echo
echo "--- functional: firewall ownership — close only what we opened ---"
mock firewall-cmd 'case "$*" in --state) exit 0 ;; *--query-port*) exit 1 ;; esac; exit 0'
: > "$CALLS"
run_lib bash "$LIB" firewall-open >/dev/null 2>&1 && ok "firewall-open ran" || fail "firewall-open failed"
grep -q '^firewall-cmd --permanent --add-port=51820/udp$' "$CALLS" && ok "firewalld port added" || fail "firewalld port not added"
[ -f "$WORK/lib/amneziawg-firewall-owned" ] && ok "ownership recorded" || fail "ownership not recorded"
: > "$CALLS"
run_lib bash "$LIB" firewall-close >/dev/null 2>&1
grep -q '^firewall-cmd --permanent --remove-port=51820/udp$' "$CALLS" && ok "owned port removed" || fail "owned port not removed"
mock firewall-cmd 'case "$*" in --state) exit 0 ;; *--query-port*) exit 0 ;; esac; exit 0'
: > "$CALLS"
run_lib bash "$LIB" firewall-open >/dev/null 2>&1
run_lib bash "$LIB" firewall-close >/dev/null 2>&1
if grep -q 'remove-port' "$CALLS"; then fail "a pre-existing operator rule was removed"; else ok "pre-existing operator rule left alone"; fi

echo
echo "--- functional: uninstall removes only recorded binaries and restores sysctls ---"
: > "$CALLS"
touch "$WORK/unit.service"
run_lib bash -c "id() { echo 0; }; export -f id; bash '$LIB' uninstall" >/dev/null 2>&1 || true
if [ -x "$WORK/usrbin/awg" ]; then ok "binaries without a build record are left untouched"; else fail "unrecorded binaries were deleted"; fi
[ ! -e "$WORK/unit.service" ] && ok "unit removed" || fail "unit not removed"
[ ! -e "$WORK/sysctl.d/99-awg.conf" ] && ok "forwarding drop-in removed" || fail "forwarding drop-in left behind"
grep -q "^sysctl -q -p $WORK/lib/amneziawg-sysctl-baseline.env$" "$CALLS" && ok "baseline sysctls re-applied" || fail "baseline not restored"

echo
if [ "$failures" -eq 0 ]; then
  echo "test-amneziawg-lib: PASS"
else
  echo "test-amneziawg-lib: $failures FAILURE(S)"
  exit 1
fi
