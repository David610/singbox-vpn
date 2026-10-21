#!/usr/bin/env bash
# Runs the interruption matrix against an IMMUTABLE export of one committed revision (updater + harness + report),
# so editing the working tree mid-run cannot change what is tested, and records a verified baseline first.
# Usage: run-interrupt-matrix.sh [interrupt_matrix.sh args...]   (default: all)   STAGING HOSTS ONLY.
set -Eeuo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
[ -z "$(git -C "$REPO" status --porcelain --untracked-files=no)" ] || { echo "refusing: tracked changes are uncommitted; the run must name one committed revision" >&2; exit 2; }
REV="$(git -C "$REPO" rev-parse HEAD)"
RUN="$(mktemp -d /root/run-matrix.XXXXXX)"
git -C "$REPO" archive "$REV" | tar -x -C "$RUN"
chmod -R a-w "$RUN"
export BRANCH_UPDATER="$RUN/deploy/almalinux/update.sh"
H="$RUN/deploy/lib"
export OUTDIR="${OUTDIR:-/root/evidence/matrix-$(date -u +%Y%m%dT%H%M%S)}"
mkdir -p "$OUTDIR"
SNAP="${SNAP:-/root/snap/baseline.tar}"; export SNAP
echo "revision=$REV harness=$(sha256sum <"$H/interrupt_matrix.sh" | cut -c1-16) updater_sha=$(sha256sum <"$BRANCH_UPDATER" | cut -c1-16) tree=$RUN"
# verified baseline: restore the snapshot and record what it is, BEFORE any stage
bash "$H/interrupt_matrix.sh" restore >"$OUTDIR/baseline-restore.txt" 2>&1 || true
python3 -I - "$OUTDIR/baseline.json" "$REV" "$SNAP" "$(tail -1 "$OUTDIR/baseline-restore.txt")" <<'PY'
import hashlib, json, sys, time
out, rev, snap, line = sys.argv[1:5]
h = hashlib.sha256()
with open(snap, "rb") as f:
    for c in iter(lambda: f.read(1 << 20), b""): h.update(c)
json.dump({"revision": rev, "snapshot": snap, "snapshot_sha256": h.hexdigest(), "restore_line": line, "recorded_utc": time.strftime("%FT%TZ", time.gmtime())}, open(out, "w"), indent=1)
PY
grep -q 'services=all-active doctor_fails=0' "$OUTDIR/baseline-restore.txt" || { echo "baseline is not healthy: $(tail -1 "$OUTDIR/baseline-restore.txt")" >&2; exit 3; }
[ "$#" -gt 0 ] || set -- all
exec bash "$H/interrupt_matrix.sh" "$@"
