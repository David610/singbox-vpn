#!/usr/bin/env bash
# Safe recovery after an update.sh run was killed mid-transaction (I-56).
# update.sh refuses to start on top of an unfinished transaction (correct:
# unknown state) and, until now, left recovery as hand-editing. This helper:
#   1. refuses unless the host is healthy (services active, doctor has no FAIL);
#   2. lists exactly what it would remove: the marker, and only the stale
#      /opt/.singbox-vpn-prev-* and /opt/.singbox-vpn-update-staging.* paths
#      that the marker itself names;
#   3. with --yes archives the marker and removes those paths.
# It never restores binaries or config. If the host is NOT healthy, restore
# from the marker's backup_dir as update.sh's error text describes.
#   recover-interrupted-update.sh [--yes]
set -Eeuo pipefail
MARKER="${RECOVER_MARKER:-/var/lib/singbox-vpn/update-transaction.json}"
OPT_DIR="${RECOVER_OPT_DIR:-/opt}"
ARCHIVE_DIR="${RECOVER_ARCHIVE_DIR:-/var/lib/singbox-vpn}"
HEALTH_CMD="${RECOVER_HEALTH_CMD:-vpn-admin doctor}"
SERVICES="${RECOVER_SERVICES:-sing-box vpn-subscription}"
YES=0; [ "${1:-}" = "--yes" ] && YES=1

die() { echo "recover-interrupted-update: ERROR: $*" >&2; exit 1; }
log() { echo "recover-interrupted-update: $*"; }

[ -e "$MARKER" ] || { log "no interrupted-update marker at $MARKER: nothing to recover."; exit 0; }

mapfile -t TARGETS < <(python3 -I - "$MARKER" "$OPT_DIR" <<'PY'
import json, os, sys
marker, opt = sys.argv[1], os.path.realpath(sys.argv[2])
d = json.load(open(marker))
out = []
for key in ("prev_opt_dir", "staged_src_dir"):
    p = d.get(key)
    if not p:
        continue
    if key == "staged_src_dir":
        p = os.path.dirname(p)           # the staging dir that contains it
    real = os.path.realpath(p)
    base = os.path.basename(real)
    ok = os.path.dirname(real) == opt and (base.startswith(".singbox-vpn-prev-") or base.startswith(".singbox-vpn-update-staging."))
    if not ok:
        print("REFUSE " + p); continue
    if os.path.isdir(real):
        print(real)
PY
)
for t in "${TARGETS[@]}"; do
  [ "${t#REFUSE }" = "$t" ] || die "marker names a path outside the expected /opt/.singbox-vpn-* pattern (${t#REFUSE }); refusing to delete anything. Inspect $MARKER by hand."
done

for s in $SERVICES; do
  systemctl is-active --quiet "$s" || die "service $s is not active: the host is not healthy. Restore from the marker's backup_dir instead (see update.sh's error text). Nothing was changed."
done
fails="$($HEALTH_CMD 2>&1 | grep -c '^\[FAIL\]' || true)"
[ "$fails" = "0" ] || die "health check reports $fails FAIL line(s): the host is not healthy. Nothing was changed."

log "host is healthy. Would remove: marker $MARKER ${TARGETS[*]:-(no stale directories present)}"
if [ "$YES" != 1 ]; then log "dry run. Re-run with --yes to apply."; exit 0; fi

install -d -m 0700 "$ARCHIVE_DIR"
cp -p "$MARKER" "$ARCHIVE_DIR/interrupted-update-$(date -u +%Y%m%dT%H%M%SZ).json"
for t in "${TARGETS[@]}"; do rm -rf -- "$t"; done
rm -f -- "$MARKER"
log "recovered: marker archived in $ARCHIVE_DIR; stale directories removed. Re-run update.sh."
