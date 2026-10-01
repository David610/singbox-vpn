#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cat >"$TMP/id" <<'MOCK'
#!/usr/bin/env bash
if [ "$1" = "-u" ] && [ "$#" -eq 1 ]; then echo 0; else echo 991; fi
MOCK
cat >"$TMP/nft" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$NFT_LOG"
if [ "$1" = "list" ]; then
  if [ "$4" = "arcana_egress_isolation" ]; then [ -f "$NFT_PRESENT" ]; else [ -f "$NFT_UNRELATED" ]; fi
  exit
fi
if [ "$1" = "delete" ]; then rm -f "$NFT_PRESENT"; exit; fi
if [ "$1" = "--check" ] && [ "$2" = "-f" ]; then
  grep -q '^table inet arcana_egress_isolation' "$3"
  exit
fi
if [ "$1" = "-f" ]; then
  cp "$2" "$NFT_LAST_BATCH"
  [ "${NFT_FAIL_APPLY:-0}" != 1 ] || exit 1
  grep -q '^table inet arcana_egress_isolation' "$2" && touch "$NFT_PRESENT"
  exit
fi
exit 1
MOCK
chmod +x "$TMP/id" "$TMP/nft"
export PATH="$TMP:$PATH" NFT_LOG="$TMP/log" NFT_LAST_BATCH="$TMP/batch" \
  NFT_PRESENT="$TMP/arcana-present" NFT_UNRELATED="$TMP/unrelated-present"
touch "$NFT_UNRELATED"
SCRIPT="$ROOT/deploy/almalinux/nftables-egress-isolation.sh"

# First apply, idempotent replacement, unrelated state preservation.
"$SCRIPT"
test -f "$NFT_PRESENT"
test -f "$NFT_UNRELATED"
"$SCRIPT"
grep -q '^delete table inet arcana_egress_isolation$' "$NFT_LAST_BATCH"
grep -q '^table inet arcana_egress_isolation' "$NFT_LAST_BATCH"
grep -q 'ip daddr' "$NFT_LAST_BATCH"
grep -q 'ip6 daddr' "$NFT_LAST_BATCH"
test -f "$NFT_UNRELATED"
! grep -Eq 'flush ruleset|delete table (ip|ip6) ' "$NFT_LAST_BATCH" "$NFT_LOG"

# A rejected replacement leaves the previously-valid table and unrelated state.
if NFT_FAIL_APPLY=1 "$SCRIPT"; then
  echo 'invalid replacement unexpectedly succeeded' >&2
  exit 1
fi
test -f "$NFT_PRESENT"
test -f "$NFT_UNRELATED"

# Removal is bounded and idempotent.
"$SCRIPT" --remove
"$SCRIPT" --remove
test ! -f "$NFT_PRESENT"
test -f "$NFT_UNRELATED"

# Every service executable/documentation path maps to the persisted repo and
# exists in the release layout; no historical /opt/vpn path may remain.
UNIT="$ROOT/deploy/almalinux/systemd/vpn-egress-isolation.service"
! grep -q '/opt/vpn/' "$UNIT"
while IFS= read -r installed; do
  relative="${installed#/opt/singbox-vpn/}"
  test "$relative" != "$installed"
  test -x "$ROOT/$relative"
done < <(sed -n -E 's#^Exec(Start|Reload|Stop)=([^ ]+).*#\2#p' "$UNIT" | sort -u)
grep -q '^Documentation=file:///opt/singbox-vpn/deploy/almalinux/nftables-egress-isolation.sh$' "$UNIT"

# Ordering alone is insufficient: sing-box must require successful isolation.
SING_UNIT="$ROOT/deploy/almalinux/systemd/sing-box.service"
grep -q '^Requires=vpn-egress-isolation.service$' "$SING_UNIT"
grep -Eq '^After=.*vpn-egress-isolation.service' "$SING_UNIT"
grep -q '^Before=sing-box.service$' "$UNIT"

# Lifecycle reconciliation reloads an active unit rather than restart/ExecStop.
grep -q 'reload-or-restart vpn-egress-isolation.service' "$ROOT/deploy/almalinux/install.sh"
grep -q 'reload-or-restart vpn-egress-isolation.service' "$ROOT/deploy/almalinux/update.sh"
! grep -q 'restart vpn-egress-isolation.service' "$ROOT/deploy/almalinux/update.sh"

echo 'nftables egress lifecycle tests passed'
