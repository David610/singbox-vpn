#!/usr/bin/env bash
# Regression (I-54): the host egress-isolation nftables table must not break
# IPv6 neighbour discovery. `meta skuid != UID accept` does not match
# kernel-originated packets (they have no socket owner), so ND to fe80::/10 and
# ff00::/8 fell through to the reject rules and the whole host lost IPv6
# (observed on the staging node after install: gateway FAILED, no IPv6 egress).
# Behavioural test in two network namespaces; needs root, ip, nft, setpriv.
# Also proves the isolation still applies to the sing-box uid only.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/deploy/almalinux/nftables-egress-isolation.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
need() { command -v "$1" >/dev/null 2>&1 || { echo "FAIL: required tool '$1' missing (this test is not skippable)"; exit 1; }; }
[ "$(id -u)" -eq 0 ] || { echo "FAIL: must run as root"; exit 1; }
for t in ip nft setpriv python3; do need "$t"; done

N1="egr-t1-$$"; N2="egr-t2-$$"
cleanup() { ip netns del "$N1" 2>/dev/null; ip netns del "$N2" 2>/dev/null; }
trap cleanup EXIT
ip netns add "$N1" && ip netns add "$N2" || { echo "FAIL: cannot create netns"; exit 1; }
ip link add vA type veth peer name vB
ip link set vA netns "$N1"; ip link set vB netns "$N2"
for pair in "$N1 vA" "$N2 vB"; do set -- $pair; ip -n "$1" link set lo up; ip -n "$1" link set "$2" up; done
ip -n "$N1" -6 addr add fd00:1::1/64 dev vA nodad
ip -n "$N2" -6 addr add fd00:1::2/64 dev vB nodad
sleep 1

# Apply the REAL script's ruleset inside N1 for uid 'nobody' (stands in for sing-box).
ip netns exec "$N1" env SING_BOX_USER=nobody bash "$SCRIPT" >/dev/null 2>&1 \
  && ok "real egress-isolation script applied inside namespace" || { bad "script failed to apply"; }

ip -n "$N1" -6 neigh flush dev vA
if ip netns exec "$N1" ping -6 -c2 -W2 fd00:1::2 >/dev/null 2>&1; then
  ok "root traffic and IPv6 neighbour discovery work with the table loaded"
else
  bad "IPv6 neighbour discovery / host traffic is broken by the isolation table"
fi

# Scope: the sing-box uid must still be refused towards denied ranges (ULA fc00::/7 here).
cnt() { ip netns exec "$N1" nft list table inet arcana_egress_isolation | awk '/fc00::\/7/ && /counter packets/ {for(i=1;i<=NF;i++) if($i=="packets") print $(i+1)}' | head -1 || true; }
before="$(cnt)"; before="${before:-NONE}"
ip netns exec "$N1" python3 - <<'PY' >/dev/null 2>&1
import socket
s = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM); s.settimeout(1)
try: s.sendto(b"x", ("fd00:1::2", 9))
except OSError: pass
PY
mid="$(cnt)"
if [ "$before" = NONE ]; then bad "reject rule has no counter, cannot prove uid scoping"; else
  [ "$mid" = "$before" ] && ok "traffic from root to a denied range is NOT touched (rule scoped to the sing-box uid)" || bad "root traffic was rejected: counter $before -> $mid"
  ip netns exec "$N1" setpriv --reuid="$(id -u nobody)" --regid="$(id -g nobody)" --clear-groups python3 - <<'PY' >/dev/null 2>&1
import socket
s = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM); s.settimeout(1)
try: s.sendto(b"x", ("fd00:1::2", 9))
except OSError: pass
PY
  after="$(cnt)"
  [ "${after:-0}" -gt "${mid:-0}" ] 2>/dev/null && ok "traffic from the sing-box uid to a denied range IS rejected (counter $mid -> $after)" || bad "sing-box uid traffic not rejected (counter $mid -> $after)"
fi

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
