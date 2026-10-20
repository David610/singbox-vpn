#!/usr/bin/env bash
# Run deploy/lib/tests/*.sh as if no singbox-vpn were installed, even on a host
# that has one (e.g. the staging VPS). Several tests source install.sh, whose
# paths are fixed constants (/etc/vpn, /opt/singbox-vpn, /var/lib/singbox-vpn);
# on an installed host they read the live deployment and fail for reasons that
# are not defects. A private mount namespace hides those paths with empty
# tmpfs mounts; nothing on the real host is touched or modified.
#   sudo deploy/lib/run-shell-tests-clean-host.sh [test-script ...]
set -Eeuo pipefail
[ "$(id -u)" -eq 0 ] || { echo "must run as root (needs unshare -m)" >&2; exit 1; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ "${SINGBOX_VPN_CLEAN_HOST_NS:-0}" != "1" ]; then
  exec env SINGBOX_VPN_CLEAN_HOST_NS=1 unshare --mount --propagation private bash "$0" "$@"
fi
# Hide paths: one overlay per parent directory, with a tmpfs upper layer holding
# a whiteout for every hidden entry, so the path does not exist at all inside
# this namespace (several tests assert absence, not just emptiness).
WORK="$(mktemp -d)"
mount -t tmpfs tmpfs "$WORK"
declare -A UPPER
n=0
hide() {
  local path="$1" parent base
  parent="$(dirname "$path")"; base="$(basename "$path")"
  [ -e "$path" ] || return 0
  if [ -z "${UPPER[$parent]:-}" ]; then
    n=$((n + 1)); UPPER[$parent]="$WORK/up$n"; mkdir -p "$WORK/up$n" "$WORK/wk$n"
  fi
  mknod "${UPPER[$parent]}/$base" c 0 0
}
hide /etc/vpn
hide /opt/singbox-vpn
hide /var/lib/singbox-vpn
hide /run/lock/singbox-vpn-installer.lock
for b in vpn vpn-admin vpn-subscription-svc vpn-provisioning-agent sing-box; do hide "/usr/local/bin/$b"; done
for parent in "${!UPPER[@]}"; do
  up="${UPPER[$parent]}"
  mount -t overlay overlay -o "lowerdir=$parent,upperdir=$up,workdir=${up/up/wk}" "$parent"
done
cd "$REPO_ROOT"
if [ "$#" -gt 0 ]; then tests=("$@"); else tests=(deploy/lib/tests/*.sh); fi
fails=0
for t in "${tests[@]}"; do
  if out="$(bash "$t" 2>&1)"; then echo "PASS $t"; else echo "FAIL $t"; printf '%s\n' "$out" | grep -E 'FAIL|UNEXPECTED|rror' | head -8 | sed 's/^/    | /'; fails=$((fails + 1)); fi
done
echo "clean-host shell test failures: $fails of ${#tests[@]}"
[ "$fails" -eq 0 ]
