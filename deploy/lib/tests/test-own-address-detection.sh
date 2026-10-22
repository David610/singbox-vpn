#!/usr/bin/env bash
# I-15: the installer must record the node's own public address(es) in
# deployment.toml for every install shape (domain-based, IPv4-only,
# dual-stack, auto-detected), as TOP-LEVEL keys, so the renderer's own-address
# reject rule (crates/compat-config server.rs, own_address_deny_rule) exists.
# Fixture-only: `ip` and the HTTPS detectors are stubbed, nothing touches the
# host or the network.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Runs $1 with install.sh's functions loaded (same harness shape as
# test-node-identity.sh) and the detectors stubbed by the $STUBS snippet.
run_fixture() {
  local stubs="$1" body="$2"
  (
    set +e
    # shellcheck source=/dev/null
    . "$INSTALL_SH"
    trap - ERR INT TERM
    set +e
    die() { echo "DIE: $*"; exit 1; }
    log() { echo "LOG: $*"; }
    warn() { echo "WARN: $*"; }
    eval "$stubs"
    eval "$body"
  )
}

PY=""
for cand in python3 python; do
  if command -v "$cand" >/dev/null 2>&1 && "$cand" -c 'import tomllib' >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "SKIP: no Python >= 3.11 (tomllib) available"; exit 0; }
pypath() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
toml_value() { "$PY" -c 'import sys,tomllib;d=tomllib.load(open(sys.argv[1],"rb"));print(d.get(sys.argv[2],""))' "$(pypath "$1")" "$2"; }

STUB_V4_ONLY='
preflight_local_public_addrs() { [ "$1" = 4 ] && echo 203.0.113.10 || true; }
preflight_detect_egress_ip_confirmed() { [ "$1" = 4 ] && echo 203.0.113.10 || return 1; }
preflight_resolve_host_addrs() { echo 203.0.113.10; }'
STUB_DUAL='
preflight_local_public_addrs() { [ "$1" = 4 ] && echo 203.0.113.10 || echo 2001:db8:1::10; }
preflight_detect_egress_ip_confirmed() { [ "$1" = 4 ] && echo 203.0.113.10 || echo 2001:db8:1::10; }
preflight_resolve_host_addrs() { printf "203.0.113.10\n2001:db8:1::10\n"; }'
STUB_NAT='
preflight_local_public_addrs() { true; }
preflight_detect_egress_ip_confirmed() { [ "$1" = 4 ] && echo 198.51.100.7 || return 1; }
preflight_resolve_host_addrs() { echo 198.51.100.7; }'
STUB_NONE='
preflight_local_public_addrs() { true; }
preflight_detect_egress_ip_confirmed() { return 1; }
preflight_resolve_host_addrs() { true; }'
STUB_DNS_LIES='
preflight_local_public_addrs() { [ "$1" = 4 ] && echo 203.0.113.10 || true; }
preflight_detect_egress_ip_confirmed() { [ "$1" = 4 ] && echo 203.0.113.10 || return 1; }
preflight_resolve_host_addrs() { echo 192.0.2.99; }'

render() {
  local stubs="$1" dest="$2" env_prefix="${3:-}"
  run_fixture "$stubs" "
    $env_prefix
    DEPLOYMENT_TOML='$dest'
    PUBLIC_HOST=vpn.example.test SUBSCRIPTION_HOST=vpn.example.test SUBSCRIPTION_PORT=8443
    REALITY_HANDSHAKE_SERVER=www.example.com
    NODE_ROLE=exit NODE_ID=de1
    resolve_own_public_addresses
    render_deployment_toml
  " 2>&1
}

echo "--- address classification ---"
classify() { run_fixture "" "type preflight_is_public_ip >/dev/null 2>&1 || { echo MISSING; exit 0; }; if preflight_is_public_ip '$1'; then echo public; else echo private; fi"; }
for a in 10.0.0.5 172.16.0.1 172.31.255.1 192.168.1.1 192.88.99.2 192.88.99.255 127.0.0.1 169.254.1.1 100.64.0.1 0.0.0.0 224.0.0.1 255.255.255.255 fe80::1 fc00::1 fd12::1 ::1 ::; do
  [ "$(classify "$a")" = private ] && ok "$a is not a public address" || fail "$a was classified public"
done
for a in 91.244.71.165 8.8.8.8 172.32.0.1 2a01:4f8::1 2606:4700::1111; do
  [ "$(classify "$a")" = public ] && ok "$a is a public address" || fail "$a was classified private"
done
[ "$(classify 'not-an-ip')" = private ] && ok "garbage is rejected" || fail "garbage accepted"
[ "$(classify '999.1.1.1')" = private ] && ok "out-of-range octet rejected" || fail "999.1.1.1 accepted"

echo "--- ip(8) parser ---"
out="$(run_fixture '' '
ip() { cat <<EOF
2: eth0    inet 203.0.113.10/24 brd 203.0.113.255 scope global eth0
2: eth0    inet 10.1.2.3/24 scope global eth1
EOF
}
preflight_local_public_addrs 4')"
[ "$out" = "203.0.113.10" ] && ok "IPv4 parser keeps only public global addresses" || fail "IPv4 parser output: $out"
out="$(run_fixture '' '
ip() { cat <<EOF
2: eth0    inet6 2001:db8:1::10/64 scope global dynamic
2: eth0    inet6 2001:db8:1::abcd/64 scope global temporary dynamic
2: eth0    inet6 fd00::5/64 scope global
2: eth0    inet6 2001:db8:1::dead/64 scope global deprecated
EOF
}
preflight_local_public_addrs 6')"
[ "$out" = "2001:db8:1::10" ] && ok "IPv6 parser drops temporary, deprecated and ULA addresses" || fail "IPv6 parser output: $out"

echo "--- domain-based install (the reported bug) ---"
out="$(render "$STUB_V4_ONLY" "$WORK/v4.toml")" || true
if [ "$(toml_value "$WORK/v4.toml" public_ipv4)" = "203.0.113.10" ] && [ -z "$(toml_value "$WORK/v4.toml" public_ipv6)" ]; then
  ok "domain install on an IPv4-only host records public_ipv4 as a top-level key"
else fail "IPv4-only domain install: $(cat "$WORK/v4.toml" 2>/dev/null) :: $out"; fi
render "$STUB_DUAL" "$WORK/dual.toml" >/dev/null || true
if [ "$(toml_value "$WORK/dual.toml" public_ipv4)" = "203.0.113.10" ] && [ "$(toml_value "$WORK/dual.toml" public_ipv6)" = "2001:db8:1::10" ]; then
  ok "dual-stack domain install records both addresses"
else fail "dual-stack install: $(cat "$WORK/dual.toml" 2>/dev/null)"; fi
render "$STUB_NAT" "$WORK/nat.toml" >/dev/null || true
[ "$(toml_value "$WORK/nat.toml" public_ipv4)" = "198.51.100.7" ] \
  && ok "NAT/floating-IP host (no local public address) uses the confirmed egress address" \
  || fail "NAT install: $(cat "$WORK/nat.toml" 2>/dev/null)"
grep -q '^\[egress\]' "$WORK/dual.toml" && "$PY" - "$(pypath "$WORK/dual.toml")" <<'PY' && ok "own-address keys are top-level, ahead of every table" || fail "own-address keys not top-level"
import sys, tomllib
d = tomllib.load(open(sys.argv[1], "rb"))
assert "public_ipv4" in d and "public_ipv4" not in d.get("subscription", {}) and "public_ipv4" not in d.get("egress", {})
PY

echo "--- DNS is never trusted as proof of ownership ---"
render "$STUB_DNS_LIES" "$WORK/dns.toml" >/dev/null || true
[ "$(toml_value "$WORK/dns.toml" public_ipv4)" = "203.0.113.10" ] \
  && ok "an address that only DNS claims (192.0.2.99) is not recorded" \
  || fail "DNS-only address leaked into deployment.toml: $(cat "$WORK/dns.toml" 2>/dev/null)"

echo "--- fail safely when the invariant cannot be established ---"
out="$(render "$STUB_NONE" "$WORK/none.toml" || true)"
if grep -q "DIE:.*PUBLIC_IPV4" <<<"$out" && [ ! -e "$WORK/none.toml" ]; then
  ok "no detectable address: install refuses with an actionable PUBLIC_IPV4 hint and writes nothing"
else fail "undetectable address did not fail safely: $out"; fi
out="$(render "$STUB_NONE" "$WORK/none-ok.toml" 'SINGBOX_VPN_ALLOW_UNKNOWN_OWN_ADDRESS=1')" || true
if grep -q "WARN:" <<<"$out" && [ -f "$WORK/none-ok.toml" ] && [ -z "$(toml_value "$WORK/none-ok.toml" public_ipv4)" ]; then
  ok "explicit opt-out proceeds with a loud warning and records nothing"
else fail "opt-out path: $out"; fi

echo "--- operator-supplied addresses win and are validated ---"
render "$STUB_V4_ONLY" "$WORK/op.toml" 'PUBLIC_IPV4=198.51.100.50 PUBLIC_IPV6=2001:db8:9::1' >/dev/null || true
if [ "$(toml_value "$WORK/op.toml" public_ipv4)" = "198.51.100.50" ] && [ "$(toml_value "$WORK/op.toml" public_ipv6)" = "2001:db8:9::1" ]; then
  ok "PUBLIC_IPV4/PUBLIC_IPV6 override detection"
else fail "operator override: $(cat "$WORK/op.toml" 2>/dev/null)"; fi
out="$(render "$STUB_V4_ONLY" "$WORK/bad.toml" 'PUBLIC_IPV4=not-an-ip')" || true
grep -q "DIE:.*PUBLIC_IPV4" <<<"$out" && [ ! -e "$WORK/bad.toml" ] \
  && ok "invalid PUBLIC_IPV4 is refused" || fail "invalid override accepted: $out"
out="$(render "$STUB_V4_ONLY" "$WORK/bad6.toml" 'PUBLIC_IPV4=2001:db8::1')" || true
grep -q "DIE:.*PUBLIC_IPV4" <<<"$out" && ok "an IPv6 literal in PUBLIC_IPV4 is refused" || fail "family mismatch accepted: $out"

echo "--- repair/update never overwrites configured addresses ---"
cat >"$WORK/existing.toml" <<'EOF'
schema_version = 2
node_id = "de1"
role = "exit"
public_host = "vpn.example.test"
subscription_host = "vpn.example.test"
public_ipv4 = "198.51.100.77"

[reality]
listen_port = 443
EOF
cp "$WORK/existing.toml" "$WORK/existing.orig"
run_fixture "$STUB_DUAL" "DEPLOYMENT_TOML='$WORK/existing.toml'; backfill_own_public_addresses" >/dev/null 2>&1 || true
cmp -s "$WORK/existing.toml" "$WORK/existing.orig" \
  && ok "a configured public_ipv4 is left byte-identical (no IPv6 added behind the operator's back)" \
  || fail "configured deployment.toml was modified: $(diff "$WORK/existing.orig" "$WORK/existing.toml")"
cat >"$WORK/legacy.toml" <<'EOF'
schema_version = 2
node_id = "de1"
role = "exit"
public_host = "vpn.example.test"
subscription_host = "vpn.example.test"

[reality]
listen_port = 443

[subscription]
listen_port = 9100
EOF
run_fixture "$STUB_DUAL" "DEPLOYMENT_TOML='$WORK/legacy.toml'; backfill_own_public_addresses" >/dev/null 2>&1 || true
if [ "$(toml_value "$WORK/legacy.toml" public_ipv4)" = "203.0.113.10" ] && [ "$(toml_value "$WORK/legacy.toml" public_ipv6)" = "2001:db8:1::10" ] \
    && [ "$("$PY" -c 'import sys,tomllib;print(tomllib.load(open(sys.argv[1],"rb"))["subscription"]["listen_port"])' "$(pypath "$WORK/legacy.toml")")" = 9100 ]; then
  ok "a legacy deployment with no address keys is backfilled; its other settings are untouched"
else fail "backfill: $(cat "$WORK/legacy.toml")"; fi
run_fixture "$STUB_NONE" "DEPLOYMENT_TOML='$WORK/legacy2.toml'; cp '$WORK/existing.orig' '$WORK/legacy2.toml'; sed -i '/^public_ipv4/d' '$WORK/legacy2.toml'; backfill_own_public_addresses; echo rc=\$?" >"$WORK/bf.out" 2>&1 || true
grep -q "rc=0" "$WORK/bf.out" && grep -q "WARN:" "$WORK/bf.out" \
  && ok "backfill that cannot detect warns and does not abort a repair" || fail "backfill failure path: $(cat "$WORK/bf.out")"

echo
if [ "$failures" -ne 0 ]; then echo "$failures failure(s)"; exit 1; fi
echo "all own-address detection checks passed"
