#!/usr/bin/env bash
# update.sh --recover (I-56): after the updater is killed mid-transaction the host used to be stuck
# (binaries of one release, state of another; every tool refusing). Static/structural guarantees;
# the dynamic proof is the staged SIGKILL experiment (evidence/interrupt/).
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
U="$REPO_ROOT/deploy/almalinux/update.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
has() { grep -qF -- "$1" "$U"; }

has '--recover) RECOVER=1' && ok "--recover flag exists" || bad "no --recover flag"
has 'update.sh --recover' && ok "help text documents --recover" || bad "help missing"
n="$(grep -c 'perform_rollback_from_backup' "$U")"
[ "$n" -ge 3 ] && ok "one restoration code path: perform_rollback_from_backup is defined once and used by both rollback_update and --recover ($n refs)" || bad "rollback logic is not shared ($n refs)"
# the in-process rollback must still go through the shared function and still remove the marker/staging
body="$(sed -n '/^rollback_update() {/,/^}/p' "$U")"
grep -q 'perform_rollback_from_backup' <<<"$body" && grep -q 'rm -f "\$TRANSACTION_MARKER"' <<<"$body" && ok "rollback_update delegates to the shared function and still clears the marker" || bad "rollback_update lost its steps"
# recovery safety properties
rec="$(sed -n '/^recover_interrupted_update() {/,/^}/p' "$U")"
grep -q 'flock -x 201' <<<"$rec" && ok "recovery takes the same lock as vpn-admin mutations" || bad "recovery does not lock"
grep -q "if \[ -L \"\$TRANSACTION_MARKER\" \]\|\[ -L \"\$TRANSACTION_MARKER\" \]" <<<"$rec" && ok "a symlinked marker is refused" || bad "symlinked marker not refused"
for needle in 'normalized absolute path' 'direct child of' 'unexpected name' 'symlink, not a directory, or not root-owned' 'resolves elsewhere' 'control chars\|clean string'; do
  grep -q "$needle" <<<"$rec" && ok "marker validation: $needle" || bad "marker validation missing: $needle"
done
grep -q 'deployment.toml' <<<"$rec" && grep -q 'PREPARE had not finished' <<<"$rec" && ok "no snapshot => nothing was mutated => only the marker is cleared (never restore from an incomplete backup)" || bad "incomplete-backup case not handled"
grep -q 'inconsistent transaction' <<<"$rec" && ok "inconsistent state (source tree moved but backup incomplete) refuses" || bad "inconsistent state not refused"
grep -q 'backup is incomplete' <<<"$rec" && ok "restore refuses when binaries/units are missing from the backup" || bad "no completeness check before restoring"
grep -q 'switch had not started' <<<"$rec" && ok "healthy + previous release untouched => just clear" || bad "healthy-unmodified case missing"
grep -q '^clear_stale_staging_dirs() {' "$U" && ok "--recover clears stale update-staging dirs even when no marker exists (SIGKILL during download)" || bad "no stale staging cleanup"
sc="$(sed -n '/^clear_stale_staging_dirs() {/,/^}/p' "$U")"
grep -q 'holds the update lock' <<<"$sc" && ok "staging cleanup documents that it relies on the held update lock (cannot race a live updater)" || bad "cleanup could race a running updater"
grep -q 'leaving .* untouched' <<<"$sc" && ok "-prev- directories (possible last known-good tree) are never auto-deleted" || bad "prev dirs not protected"
grep -q 'one-file-system' <<<"$sc" && grep -q '\-L "\$d"' <<<"$sc" && ok "cleanup skips symlinks and stays on one filesystem" || bad "cleanup symlink/fs safety missing"
grep -q 'run: update.sh --recover removes stale update-staging' "$U" && ok "the stale-staging refusal tells the operator to run --recover" || bad "stale refusal text"
# liveness lock + cosign scratch (found by SIGKILL experiments: orphans survive a group kill; cosign leaked 135 MB per run)
grep -q 'exec 202>"\$UPDATE_LOCK"' "$U" && grep -q 'flock -n 202' "$U" && ok "every run (incl. --recover) holds a kernel liveness lock on fd 202" || bad "no liveness lock"
! grep -q 'pgrep' <(sed -n '/^clear_stale_staging_dirs() {/,/^}/p' "$U") && ok "no pgrep command-line heuristic in the staging cleanup" || bad "pgrep heuristic is back"
grep -q 'mktemp -d "\${STAGING_ROOT:-/tmp}/cosign' "$U" && ok "cosign download lives under the cleaned staging root (no /tmp leak)" || bad "cosign scratch leaks again"
grep -q "! -name 'cosign.\*'" "$U" && ok "staged-source selection excludes the cosign scratch dir" || bad "find could select the cosign dir as the source tree"
# ordering: the helper functions the shared rollback needs are defined before it
for fn in quiesce_new_egress_unit_for_rollback restore_arcana_egress_table restore_egress_unit_runtime_baseline restore_deployment_toml_snapshot; do
  a="$(grep -n "^$fn() {" "$U" | head -1 | cut -d: -f1 || true)"; b="$(grep -n '^perform_rollback_from_backup() {' "$U" | cut -d: -f1 || true)"
  [ -n "$a" ] && [ "$a" -lt "$b" ] && ok "$fn is defined before perform_rollback_from_backup" || bad "$fn ordering"
done
# the refusal message points at the new mode
grep -q 'update.sh --recover (validates the marker' "$U" && ok "the 'unfinished transaction' refusal tells the operator to run --recover" || bad "refusal text not updated"
[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
