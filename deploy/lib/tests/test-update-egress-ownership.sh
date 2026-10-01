#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
log() { :; }; warn() { :; }; die() { echo "$*" >&2; exit 1; }
export OWNERSHIP_DIR="$TMP/state" OWNERSHIP_FILE="$TMP/state/ownership.env"
cat >"$TMP/systemctl" <<'MOCK'
#!/usr/bin/env bash
case "$1" in
  is-active) [ "${MOCK_ACTIVE:-0}" = 1 ] ;;
  is-enabled) [ "${MOCK_ENABLED:-0}" = 1 ] ;;
  *) exit 0 ;;
esac
MOCK
chmod +x "$TMP/systemctl"
cat >"$TMP/nft" <<'MOCK'
#!/usr/bin/env bash
if [ "$1" = list ] && [ "$2" = tables ]; then exit 0
elif [ "$1" = list ]; then [ -f "$NFT_TABLE" ] && cat "$NFT_TABLE"
else exit 1
fi
MOCK
chmod +x "$TMP/nft"
export NFT_TABLE="$TMP/table.nft"
export PATH="$TMP:$PATH"
# shellcheck source=/dev/null
. "$ROOT/deploy/lib/ownership.sh"
NEW="$ROOT/deploy/almalinux/systemd/vpn-egress-isolation.service"
DEST="$TMP/vpn-egress-isolation.service"

# Old release had no unit: update owns it and uninstall is authorized to remove.
ownership_capture_systemd_baseline_once EGRESS_ISOLATION_UNIT vpn-egress-isolation.service
ownership_capture_egress_table_baseline_once
install_fixed_path_with_ownership "$NEW" "$DEST" EGRESS_ISOLATION_UNIT
[ "$(ownership_get FIXEDPATH_EGRESS_ISOLATION_UNIT_PRE_EXISTED)" = 0 ]
[ "$(ownership_get EGRESS_TABLE_PRE_EXISTED)" = 0 ]
rm -f "$DEST"
[ ! -e "$DEST" ]

# Independent foreign-unit fixture: exact predecessor is backed up once and
# can be restored byte-for-byte after update/uninstall.
rm -rf "$OWNERSHIP_DIR"
printf '%s\n' 'table inet arcana_egress_isolation { }' >"$NFT_TABLE"
printf '%s\n' 'foreign unit contents' >"$DEST"
cp "$DEST" "$TMP/foreign.expected"
MOCK_ACTIVE=1 MOCK_ENABLED=1 ownership_capture_systemd_baseline_once EGRESS_ISOLATION_UNIT vpn-egress-isolation.service
ownership_capture_egress_table_baseline_once
install_fixed_path_with_ownership "$NEW" "$DEST" EGRESS_ISOLATION_UNIT
[ "$(ownership_get FIXEDPATH_EGRESS_ISOLATION_UNIT_PRE_EXISTED)" = 1 ]
[ "$(ownership_get SYSTEMD_EGRESS_ISOLATION_UNIT_PRE_ACTIVE)" = 1 ]
[ "$(ownership_get SYSTEMD_EGRESS_ISOLATION_UNIT_PRE_ENABLED)" = 1 ]
[ "$(ownership_get EGRESS_TABLE_PRE_EXISTED)" = 1 ]
cmp -s "$(ownership_get EGRESS_TABLE_BACKUP)" "$NFT_TABLE"
TABLE_BACKUP="$(ownership_get EGRESS_TABLE_BACKUP)"
cp "$TABLE_BACKUP" "$TMP/table.expected"
printf '%s\n' 'table inet arcana_egress_isolation { chain managed { } }' >"$NFT_TABLE"
ownership_capture_egress_table_baseline_once
cmp -s "$TABLE_BACKUP" "$TMP/table.expected"
BACKUP="$(ownership_get FIXEDPATH_EGRESS_ISOLATION_UNIT_BACKUP)"
cmp -s "$BACKUP" "$TMP/foreign.expected"
cp -a "$BACKUP" "$DEST"
cmp -s "$DEST" "$TMP/foreign.expected"

# Production update uses the shared ownership mechanism and rollback cleanup
# does not call a helper from the restored (possibly old) source tree.
grep -q 'install_fixed_path_with_ownership.*EGRESS_ISOLATION_UNIT' "$ROOT/deploy/almalinux/update.sh"
! grep -q '/opt/singbox-vpn/deploy/almalinux/nftables-egress-isolation.sh --remove' "$ROOT/deploy/almalinux/update.sh"
grep -q 'restore_arcana_egress_table' "$ROOT/deploy/almalinux/update.sh"
echo 'update egress ownership tests passed'
