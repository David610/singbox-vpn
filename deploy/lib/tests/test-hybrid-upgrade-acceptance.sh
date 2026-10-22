#!/usr/bin/env bash
# Tests deploy/almalinux/hybrid-upgrade-acceptance.sh's OWN logic against a
# stateful mock `ssh` (no network, no real host): its safety guards, that it
# preserves the hybrid state until the update is tested (never uninstalls,
# reinstalls or repairs first), the order of the transition steps, and that it
# FAILS when the update loses credentials or silently drops the egress policy.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$ROOT/deploy/almalinux/hybrid-upgrade-acceptance.sh"
T="$(mktemp -d)"
export T
trap 'rm -rf "$T"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

mkdir -p "$T/bin"
cat >"$T/bin/ssh" <<'MOCK'
#!/usr/bin/env bash
# Stateful stand-in for the target host. State lives in files under $T.
cmd="${*: -1}"
printf '%s\n' "$cmd" >>"$T/ssh.log"
st() { cat "$T/state/$1" 2>/dev/null || true; }
set_st() { printf '%s' "$2" >"$T/state/$1"; }
case "$cmd" in
  true) exit 0 ;;
  *'uname -m'*|*os-release*) exit 0 ;;
  "[ -e /var/lib/singbox-vpn/install-state.json ]") [ -n "$(st manifest)" ] ;;
  '[ ! -e /etc/vpn ] && [ ! -e /opt/singbox-vpn ]') [ -z "$(st manifest)" ] ;;
  *'curl -fsSL --retry 5 https://raw.githubusercontent.com'*install.sh*) set_st manifest v1.2.0; set_st binver 1.2.0; echo "INSTALL" >>"$T/events"; exit 0 ;;
  *singbox_vpn_version*)
    want="$(printf '%s' "$cmd" | sed -E 's/.*singbox_vpn_version": "([^"]*)".*/\1/')"
    [ "$(st manifest)" = "$want" ] ;;
  *'--version | awk'*)
    want="$(printf '%s' "$cmd" | sed -E "s/.*= '([0-9.]+)'.*/\1/")"
    [ "$(st binver)" = "$want" ] ;;
  *'user list | grep -q hybrid-test'*) exit 0 ;;
  *'sha256sum --ignore-missing -c SHA256SUMS; tar -xzf'*'install -m 0755'*)
    [ -z "$(st fail_hybrid)" ] || exit 1
    set_st binver 1.3.0; set_st binhash handcopied; echo "HYBRID-COPY" >>"$T/events"; exit 0 ;;
  *'egress set reject_google'*) set_st policy reject_google; echo "POLICY-SET" >>"$T/events"; exit 0 ;;
  *"egress show | grep -q '^ipv6_literal: reject_google'"*) [ "$(st policy)" = reject_google ] ;;
  *"egress show | grep -q 'allow (plain exit default)'"*) [ -z "$(st policy)" ] ;;
  *'sha256sum /etc/vpn/compat/users/users.json'*) echo "$(st userfp)"; exit 0 ;;
  *'update.sh --version v1.3.0-rc.1'*)
    echo "UPDATE" >>"$T/events"; [ -z "$(st fail_update)" ] || exit 1
    set_st manifest v1.3.0-rc.1; set_st binver 1.3.0; set_st binhash release; set_st identity recorded
    [ -z "$(st change_users)" ] || set_st userfp CHANGED
    exit 0 ;;
  *'update.sh --repair'*) echo "REPAIR" >>"$T/events"; exit 0 ;;
  *'update.sh --version v1.2.0'*)
    echo "DOWNGRADE" >>"$T/events"
    if [ -n "$(st downgrade_accepts)" ]; then set_st manifest v1.2.0; set_st policy ""; set_st binhash old; exit 0; fi
    [ -z "$(st policy)" ] || exit 1
    exit 0 ;;
  *'sha256sum /usr/local/bin/vpn-admin'*) echo "$(st binhash)"; exit 0 ;;
  *"release identity UNKNOWN"*) [ -z "$(st identity)" ] ;;
  *"release identity: recorded"*) [ "$(st identity)" = recorded ] ;;
  *'doctor --protocol --require-protocol'*) exit 0 ;;
  *'sha256sum singbox-vpn-x86_64-unknown-linux-gnu/vpn-admin'*) [ "$(st binhash)" = release ] ;;
  *'ip_version'*) [ "$(st policy)" = reject_google ] ;;
  *) echo "UNHANDLED mock command: $cmd" >>"$T/unhandled"; exit 0 ;;
esac
MOCK
chmod +x "$T/bin/ssh"
printf '#!/bin/sh\nexit 0\n' >"$T/bin/sleep"; chmod +x "$T/bin/sleep"

fresh_state() { rm -rf "$T/state" "$T/ssh.log" "$T/events" "$T/unhandled"; mkdir -p "$T/state"; : >"$T/ssh.log"; : >"$T/events"; set_baseline; }
set_baseline() { printf 'v1.2.0' >"$T/state/manifest"; printf '1.2.0' >"$T/state/binver"; printf 'old' >"$T/state/binhash"; printf 'fingerprint-A' >"$T/state/userfp"; }
flag() { printf '1' >"$T/state/$1"; }
khf="$T/known_hosts"; echo 'host ssh-ed25519 AAAA' >"$khf"
run() { # run <args...>: runs the script with the mock ssh first on PATH
  PATH="$T/bin:$PATH" bash "$SCRIPT" --host root@disposable.example --candidate v1.3.0-rc.1 --known-hosts-file "$khf" --i-understand-this-modifies-the-host "$@"
}
export T

echo "--- safety guards ---"
fresh_state
if PATH="$T/bin:$PATH" bash "$SCRIPT" --host root@h --candidate v1.3.0-rc.1 --scenario normal --known-hosts-file "$khf" >/dev/null 2>&1; then fail "ran without the acknowledgement"; else ok "refuses without --i-understand-this-modifies-the-host"; fi
if PATH="$T/bin:$PATH" bash "$SCRIPT" --host root@h --candidate v1.3.0-rc.1 --scenario normal --i-understand-this-modifies-the-host >/dev/null 2>&1; then fail "ran without a host key policy"; else ok "refuses without a verified host key (or the explicit unverified flag)"; fi
for bad in "--candidate main" "--scenario weird" "--host root@127.0.0.1"; do
  # shellcheck disable=SC2086
  if run --scenario normal $bad >/dev/null 2>&1; then fail "accepted invalid input: $bad"; else ok "rejects invalid input: $bad"; fi
done
if SINGBOX_VPN_PRODUCTION_HOST=disposable.example run --scenario normal >/dev/null 2>&1; then fail "targeted the configured production host"; else ok "refuses SINGBOX_VPN_PRODUCTION_HOST"; fi
[ ! -s "$T/ssh.log" ] && ok "no ssh connection was attempted by any refused invocation" || fail "a refused invocation still contacted a host"

echo "--- hybrid-policy: the owner's real transition ---"
fresh_state
out="$(run --scenario hybrid-policy 2>&1)" && rc=0 || rc=$?
[ "$rc" -eq 0 ] && grep -q '^HYBRID UPGRADE GATE: PASS (hybrid-policy)$' <<<"$out" && ok "a correct transition passes the gate" || { fail "a correct transition did not pass (rc=$rc)"; grep -E "FAIL|UNHANDLED" <<<"$out" | head; }
ev="$(tr '\n' ' ' <"$T/events")"
[ "$ev" = "HYBRID-COPY POLICY-SET UPDATE REPAIR DOWNGRADE " ] && ok "state is created in place, then updated, then repaired, then the downgrade is attempted (events: $ev)" || fail "unexpected step order: $ev"
grep -q "singbox-vpn-src.tar.gz.*sha256sum -c -.*sudo bash deploy/almalinux/update.sh --version v1.3.0-rc.1" "$T/ssh.log"   && ! grep -q "/opt/singbox-vpn/deploy/almalinux/update.sh --version v1.3.0-rc.1" "$T/ssh.log"   && ok "the transition runs the candidate's own updater from its checksum-verified source archive, never the baseline's installed one"   || fail "the transition did not use the candidate's verified updater"
! grep -qE 'uninstall|install\.sh' "$T/ssh.log" && ok "nothing was uninstalled or reinstalled before the transition" || fail "the harness uninstalled/reinstalled the host"
[ ! -s "$T/unhandled" ] && ok "every remote command was a known, asserted one" || fail "unasserted remote commands: $(cat "$T/unhandled")"
grep -q 'sha256' <<<"$out" && fail "the report prints hash material" || ok "no credential/hash material appears in the report"

echo "--- hybrid and normal scenarios ---"
for sc in hybrid normal; do
  fresh_state; out="$(run --scenario $sc 2>&1)" && rc=0 || rc=$?
  [ "$rc" -eq 0 ] && ok "$sc passes" || { fail "$sc failed (rc=$rc)"; grep FAIL <<<"$out" | head -3; }
done
fresh_state; run --scenario normal >/dev/null 2>&1 || true
grep -q 'HYBRID-COPY' "$T/events" && fail "the normal scenario copied executables by hand" || ok "the normal scenario never creates the hybrid state"
grep -q 'DOWNGRADE' "$T/events" && fail "the downgrade refusal is only meaningful with a policy" || ok "the downgrade-refusal stage runs only when a policy is configured"

echo "--- failure injection: the gate must FAIL ---"
fresh_state; flag fail_update
run --scenario hybrid-policy >"$T/o.log" 2>&1 && fail "a failed update passed" || { grep -q 'update.sh --version v1.3.0-rc.1 succeeded' "$T/o.log" && ok "a failed update fails the gate"; }
fresh_state; flag change_users
run --scenario hybrid-policy >"$T/o.log" 2>&1 && fail "changed credentials passed" || ok "an update that changes credentials fails the gate"
fresh_state; flag downgrade_accepts
run --scenario hybrid-policy >"$T/o.log" 2>&1 && fail "a silently accepted downgrade passed" || { grep -q 'must be refused while \[egress\] is configured' "$T/o.log" && ok "an accepted downgrade that drops the policy fails the gate"; }
fresh_state; rm -f "$T/state/manifest"
run --scenario normal >"$T/o.log" 2>&1 && fail "a missing baseline passed" || { ! grep -q INSTALL "$T/events" && ok "without --prepare-baseline a host with no installation fails and nothing is installed"; }
fresh_state; flag fail_hybrid
run --scenario hybrid >"$T/o.log" 2>&1 && fail "a failed hybrid setup passed" || ok "a hybrid state that cannot be created fails the gate"

echo "--- --prepare-baseline only ever installs onto an EMPTY host ---"
fresh_state; rm -f "$T/state/manifest"
run --scenario normal --prepare-baseline --domain vpn.example.com >"$T/o.log" 2>&1 || true
grep -q INSTALL "$T/events" && ok "an empty host gets the baseline installed first" || fail "baseline was not prepared on an empty host"
fresh_state
printf 'v1.1.0' >"$T/state/manifest"
run --scenario normal --prepare-baseline --domain vpn.example.com >"$T/o.log" 2>&1 && fail "prepared over an existing installation" || ok "a host that already has an installation is never prepared over"
grep -q INSTALL "$T/events" && fail "installed over an existing installation" || ok "no install ran over the existing installation"

if [ "$failures" -ne 0 ]; then echo "FAIL: test-hybrid-upgrade-acceptance.sh ($failures failure(s))" >&2; exit 1; fi
echo "PASS: test-hybrid-upgrade-acceptance.sh"
