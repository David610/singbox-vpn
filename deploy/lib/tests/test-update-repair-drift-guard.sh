#!/usr/bin/env bash
# update.sh --repair re-installs the RECORDED release. If the installed executables are not that release, repair must
# refuse (it would restore older, possibly defective material: I-54 came back this way). Hermetic: private mount namespace.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [ "${REPAIR_GUARD_NS:-0}" != "1" ]; then
  [ "$(id -u)" -eq 0 ] || { echo "FAIL: must run as root"; exit 1; }
  exec env REPAIR_GUARD_NS=1 unshare --mount --propagation private bash "$0" "$@"
fi
failures=0; ok() { echo "ok: $*"; }; bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
U="$REPO_ROOT/deploy/almalinux/update.sh"; OUT="/run/rg.$$.out"
for d in /opt /etc/vpn /var/lib/singbox-vpn /run/lock /usr/local/bin /tmp; do mkdir -p "$d"; mount -t tmpfs tmpfs "$d"; done
printf 'schema_version = 2\nrole = "exit"\n' >/etc/vpn/deployment.toml
mkdir -p /opt/singbox-vpn/deploy/almalinux
printf 'adm\n' >/usr/local/bin/vpn-admin; printf 'vpn\n' >/usr/local/bin/vpn; printf 'sub\n' >/usr/local/bin/vpn-subscription-svc
h() { sha256sum "$1" | awk '{print $1}'; }
manifest() { printf '{"singbox_vpn_version":"v1.3.1-rc.2","singbox_vpn_repo":"David610/singbox-vpn","vpn_admin_sha256":"%s","vpn_sha256":"%s","subscription_sha256":"%s","arch":"amd64","sing_box_version_pinned":"1.14.1"}' "$1" "$2" "$3" >/var/lib/singbox-vpn/install-state.json; }
# curl stub: any network use fails loudly but harmlessly (the guard must trigger BEFORE any fetch)
printf '#!/bin/sh\necho NETWORK-USED >&2; exit 22\n' >/usr/local/bin/curl; chmod +x /usr/local/bin/curl; export PATH="/usr/local/bin:$PATH"
# 1. executables differ from the recorded release -> refuse, nothing changed
manifest "deadbeef" "$(h /usr/local/bin/vpn)" "$(h /usr/local/bin/vpn-subscription-svc)"
bash "$U" --repair >"$OUT" 2>&1; rc=$?
{ [ "$rc" -ne 0 ] && grep -q 'refusing --repair' "$OUT" && grep -q 'vpn-admin' "$OUT" && ! grep -q NETWORK-USED "$OUT"; } && ok "repair refuses when vpn-admin differs from the recorded release (no network use)" || bad "drift not refused (rc=$rc): $(tail -3 "$OUT")"
manifest "$(h /usr/local/bin/vpn-admin)" "$(h /usr/local/bin/vpn)" "bad"
bash "$U" --repair >"$OUT" 2>&1; { grep -q 'refusing --repair' "$OUT" && grep -q 'vpn-subscription-svc' "$OUT"; } && ok "repair refuses on a subscription-binary mismatch" || bad "subscription drift not refused"
# 2. all three match -> the guard passes (repair proceeds to its own work; it then needs the network, which the stub refuses)
manifest "$(h /usr/local/bin/vpn-admin)" "$(h /usr/local/bin/vpn)" "$(h /usr/local/bin/vpn-subscription-svc)"
bash "$U" --repair >"$OUT" 2>&1; ! grep -q 'refusing --repair' "$OUT" && ok "repair is not refused when the installed executables are the recorded release" || bad "guard refused a consistent install"
# 3. explicit override proceeds past the guard
manifest "deadbeef" "$(h /usr/local/bin/vpn)" "$(h /usr/local/bin/vpn-subscription-svc)"
bash "$U" --repair --force-repair-recorded-release >"$OUT" 2>&1; ! grep -q 'refusing --repair' "$OUT" && ok "--force-repair-recorded-release overrides the guard deliberately" || bad "override ignored"
# 4. --dev-rebuild from a tree other than the installed one is refused before anything is touched
manifest "$(h /usr/local/bin/vpn-admin)" "$(h /usr/local/bin/vpn)" "$(h /usr/local/bin/vpn-subscription-svc)"
bash "$U" --dev-rebuild >"$OUT" 2>&1; rc=$?
{ [ "$rc" -ne 0 ] && grep -q 'must run from the installed source tree' "$OUT" && ! grep -q 'cargo\|running tests' "$OUT"; } && ok "--dev-rebuild outside the installed tree is refused (no tests, no build)" || bad "dev-rebuild from a foreign tree not refused (rc=$rc): $(tail -2 "$OUT")"
rm -f "$OUT"; [ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
