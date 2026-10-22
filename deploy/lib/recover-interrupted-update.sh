#!/usr/bin/env bash
# Safe recovery after an update.sh run was killed mid-transaction (I-56).
# update.sh refuses to start on top of an unfinished transaction (correct:
# unknown state). This helper:
#   1. refuses unless the host is healthy (services active, doctor has no FAIL);
#   2. parses the marker FAIL-CLOSED: any malformed field, control character,
#      relative path, `..`, symlink (the path or any component), non-directory,
#      non-root-owned directory, wrong parent, or wrong name aborts with nothing
#      deleted;
#   3. lists exactly what it would remove; with --yes archives the marker and
#      removes only those validated directories.
# It never restores binaries or config. If the host is NOT healthy, restore from
# the marker's backup_dir as update.sh's error text describes.
#   recover-interrupted-update.sh [--yes]
set -Eeuo pipefail
MARKER="${RECOVER_MARKER:-/var/lib/singbox-vpn/update-transaction.json}"
OPT_DIR="${RECOVER_OPT_DIR:-/opt}"
ARCHIVE_DIR="${RECOVER_ARCHIVE_DIR:-/var/lib/singbox-vpn}"
HEALTH_CMD="${RECOVER_HEALTH_CMD:-vpn-admin doctor}"
SERVICES="${RECOVER_SERVICES:-sing-box vpn-subscription}"
EXPECT_UID="${RECOVER_EXPECT_UID:-0}"
YES=0; [ "${1:-}" = "--yes" ] && YES=1

die() { echo "recover-interrupted-update: ERROR: $*" >&2; exit 1; }
log() { echo "recover-interrupted-update: $*"; }

[ -e "$MARKER" ] || { log "no interrupted-update marker at $MARKER: nothing to recover."; exit 0; }
[ -L "$MARKER" ] && die "the marker is a symlink; refusing."

LIST="$(mktemp)"; trap 'rm -f "$LIST"' EXIT
python3 -I - "$MARKER" "$OPT_DIR" "$EXPECT_UID" >"$LIST" <<'PY' || die "marker validation failed (nothing deleted): see the message above."
import json, os, re, stat, sys

marker, opt_arg, uid = sys.argv[1], sys.argv[2], int(sys.argv[3])
PREV = re.compile(r"^\.singbox-vpn-prev-[A-Za-z0-9._-]+$")
STAGE = re.compile(r"^\.singbox-vpn-update-staging\.[A-Za-z0-9._-]+$")

def fail(msg):
    print(f"refusing: {msg}", file=sys.stderr)
    sys.exit(3)

try:
    with open(marker) as f:
        d = json.load(f)
except (OSError, ValueError) as e:
    fail(f"marker unreadable or not JSON ({e})")
if not isinstance(d, dict):
    fail("marker is not a JSON object")
opt = os.path.realpath(opt_arg)
out = []

def check(key, path, pattern, stage_parent):
    if path in (None, ""):
        return
    if not isinstance(path, str):
        fail(f"{key} is not a string")
    if any(ord(c) < 0x20 or ord(c) == 0x7F for c in path):
        fail(f"{key} contains control characters")
    if not os.path.isabs(path) or os.path.normpath(path) != path:
        fail(f"{key} is not a normalized absolute path ({path!r})")
    target = os.path.dirname(path) if stage_parent else path   # staged_src_dir -> its staging dir
    if os.path.dirname(target) != opt:
        fail(f"{key}: {target!r} is not a direct child of {opt}")
    if not pattern.match(os.path.basename(target)):
        fail(f"{key}: unexpected name {os.path.basename(target)!r}")
    try:
        st = os.lstat(target)
    except FileNotFoundError:
        return                                # already gone: nothing to remove
    if stat.S_ISLNK(st.st_mode):
        fail(f"{key}: {target!r} is a symlink")
    if not stat.S_ISDIR(st.st_mode):
        fail(f"{key}: {target!r} is not a directory")
    if st.st_uid != uid:
        fail(f"{key}: {target!r} is not owned by uid {uid}")
    if os.path.realpath(target) != target:
        fail(f"{key}: {target!r} resolves elsewhere ({os.path.realpath(target)!r})")
    out.append(target)

check("prev_opt_dir", d.get("prev_opt_dir"), PREV, False)
check("staged_src_dir", d.get("staged_src_dir"), STAGE, True)
sys.stdout.write("\0".join(dict.fromkeys(out)))
PY
mapfile -d '' -t TARGETS <"$LIST"

for s in $SERVICES; do
  systemctl is-active --quiet "$s" || die "service $s is not active: the host is not healthy. Restore from the marker's backup_dir instead (see update.sh's error text). Nothing was changed."
done
health_rc=0
health_output="$($HEALTH_CMD 2>&1)" || health_rc=$?
[ "$health_rc" -eq 0 ] || die "health check exited $health_rc: health is unverified. Marker and backups are retained."
fails="$(grep -c '^\[FAIL\]' <<<"$health_output" || true)"
[ "$fails" = "0" ] || die "health check reports $fails FAIL line(s): the host is not healthy. Nothing was changed."

log "host is healthy. Would remove: marker $MARKER ${TARGETS[*]:-(no stale directories present)}"
if [ "$YES" != 1 ]; then log "dry run. Re-run with --yes to apply."; exit 0; fi

install -d -m 0700 "$ARCHIVE_DIR"
cp -p "$MARKER" "$ARCHIVE_DIR/interrupted-update-$(date -u +%Y%m%dT%H%M%SZ).json"
for t in "${TARGETS[@]}"; do rm -rf --one-file-system -- "$t"; done
rm -f -- "$MARKER"
log "recovered: marker archived in $ARCHIVE_DIR; stale directories removed. Re-run update.sh."
