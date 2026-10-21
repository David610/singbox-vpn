#!/usr/bin/env bash
# Dynamic proof of update.sh --recover's safety properties for the paths that need no live services.
# Runs the REAL update.sh as root inside a private mount namespace with tmpfs over /opt, /etc/vpn,
# /var/lib/singbox-vpn and /run/lock, so nothing on the host (including a real updater's lock or a
# real staging dir) can be touched or observed. Needs root + unshare + flock.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [ "${UPDATE_RECOVER_DYN_NS:-0}" != "1" ]; then
  [ "$(id -u)" -eq 0 ] || { echo "FAIL: must run as root (this test is not skippable)"; exit 1; }
  for t in unshare flock python3; do command -v "$t" >/dev/null || { echo "FAIL: $t missing"; exit 1; }; done
  exec env UPDATE_RECOVER_DYN_NS=1 unshare --mount --propagation private bash "$0" "$@"
fi
REC="/run/update-recover-test.$$.out"; trap 'rm -f "$REC"' EXIT
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
U="$REPO_ROOT/deploy/almalinux/update.sh"
for d in /opt /etc/vpn /var/lib/singbox-vpn /run/lock /usr/local/bin /tmp; do mkdir -p "$d"; mount -t tmpfs tmpfs "$d"; done
mkdir -p /etc/vpn/backups; printf 'schema_version = 2\nrole = "exit"\npublic_host = "x.example.com"\n' >/etc/vpn/deployment.toml
MARK=/var/lib/singbox-vpn/update-transaction.json
run() { bash "$U" --recover >$REC 2>&1; RC=$?; }

# --- 1. no marker, no stale dirs: clean no-op
run; [ "$RC" -eq 0 ] && ok "nothing to recover: exit 0" || bad "no-op exit $RC: $(tail -2 $REC)"

# --- 2. no marker, stale staging dir (SIGKILL during download): removed; unrelated dirs untouched
mkdir -p /opt/.singbox-vpn-update-staging.AbC123/x /opt/singbox-vpn /opt/other-dir /opt/.singbox-vpn-update-staging-evil.d
echo keep >/opt/singbox-vpn/keep; echo keep >/opt/other-dir/keep
run
{ [ "$RC" -eq 0 ] && [ ! -d /opt/.singbox-vpn-update-staging.AbC123 ] && [ -f /opt/singbox-vpn/keep ] && [ -f /opt/other-dir/keep ] && [ -d /opt/.singbox-vpn-update-staging-evil.d ]; } \
  && ok "stale staging dir removed; live tree, unrelated dirs and a look-alike name untouched" || bad "staging cleanup wrong (rc=$RC): $(tail -3 $REC)"
run; [ "$RC" -eq 0 ] && ok "idempotent: second run is a clean no-op" || bad "second run failed"

# --- 3. a symlink named like a staging dir is never followed or removed
mkdir -p /opt/victim; echo precious >/opt/victim/f; ln -s /opt/victim /opt/.singbox-vpn-update-staging.LinK99
run; { [ -f /opt/victim/f ] && [ -L /opt/.singbox-vpn-update-staging.LinK99 ]; } && ok "symlink named like a staging dir: skipped, target intact" || bad "followed or removed a symlink"
rm -f /opt/.singbox-vpn-update-staging.LinK99

# --- 4. -prev- directories (possible last known-good tree) are reported, never deleted
mkdir -p /opt/.singbox-vpn-prev-777; echo old >/opt/.singbox-vpn-prev-777/f
run; { [ -f /opt/.singbox-vpn-prev-777/f ] && grep -q 'leaving /opt/.singbox-vpn-prev-777 untouched' $REC; } && ok "-prev- dir left untouched and reported" || bad "-prev- dir handling wrong"
rm -rf /opt/.singbox-vpn-prev-777

# --- 4b. leaked cosign dirs from OLD updaters are reported, never deleted
mkdir -p /tmp/tmp.LEAK1; : >/tmp/tmp.LEAK1/cosign-linux-amd64; mkdir -p /tmp/tmp.OTHER; echo x >/tmp/tmp.OTHER/f
run; { [ -f /tmp/tmp.LEAK1/cosign-linux-amd64 ] && [ -f /tmp/tmp.OTHER/f ] && grep -q 'leaked cosign download' "$REC"; } && ok "leaked cosign dir reported (not deleted); unrelated /tmp dirs untouched" || bad "cosign leak handling wrong"
rm -rf /tmp/tmp.LEAK1 /tmp/tmp.OTHER

# --- 5. liveness: a running updater (or ORPHANED child of a killed one) holds the update lock => refuse, delete nothing
mkdir -p /opt/.singbox-vpn-update-staging.Live01/x
( exec 202>/run/lock/singbox-vpn-update.lock; flock -n 202 && exec sleep 30 ) &
HOLD=$!; sleep 1
run
{ [ "$RC" -ne 0 ] && [ -d /opt/.singbox-vpn-update-staging.Live01 ] && grep -qi 'another update' $REC; } \
  && ok "lock held by another updater/orphan: refused with an explicit error, staging dir NOT deleted" || bad "ignored a live updater (rc=$RC): $(tail -2 $REC)"
kill "$HOLD" 2>/dev/null; wait "$HOLD" 2>/dev/null
run; { [ "$RC" -eq 0 ] && [ ! -d /opt/.singbox-vpn-update-staging.Live01 ]; } && ok "after the holder exits, recovery proceeds and cleans" || bad "did not recover after the holder died (rc=$RC)"
# a SIGKILLed holder releases the lock by itself (no stale lock file problem)
mkdir -p /opt/.singbox-vpn-update-staging.Kill01
( exec 202>/run/lock/singbox-vpn-update.lock; flock -n 202 && exec sleep 30 ) & HOLD=$!; sleep 1; kill -9 "$HOLD"; wait "$HOLD" 2>/dev/null
run; { [ "$RC" -eq 0 ] && [ ! -d /opt/.singbox-vpn-update-staging.Kill01 ]; } && ok "SIGKILL of the holder frees the lock (no stale lock)" || bad "lock stuck after SIGKILL (rc=$RC)"

# --- 6. markers: unsafe or malformed => refuse and change NOTHING
mkdir -p /etc/vpn/backups/20261020-100000-1 /opt/.singbox-vpn-update-staging.M1
refuse() { # $1=label ; marker already written
  run; if [ "$RC" -ne 0 ] && [ -e "$MARK" ] && [ -d /opt/singbox-vpn ] && [ -d /etc/vpn/backups/20261020-100000-1 ]; then ok "$1: refused, marker and directories untouched"; else bad "$1: rc=$RC marker=$([ -e "$MARK" ] && echo kept || echo GONE): $(tail -2 $REC)"; fi; }
printf 'not json' >"$MARK"; refuse "malformed marker"
printf '[1,2]' >"$MARK"; refuse "marker is not an object"
printf '{"backup_dir":"/etc/vpn/backups/../..","prev_opt_dir":"/opt/.singbox-vpn-prev-1","staged_src_dir":"","from_version":"v1","to_version":"v2"}' >"$MARK"; refuse "traversal in backup_dir"
printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/../etc","staged_src_dir":"","from_version":"v1","to_version":"v2"}' >"$MARK"; refuse "traversal in prev_opt_dir"
printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/.singbox-vpn-prev-1","staged_src_dir":"/opt/singbox-vpn/src","from_version":"v1","to_version":"v2"}' >"$MARK"; refuse "staged_src_dir whose parent is the live tree"
printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/.singbox-vpn-prev-1","staged_src_dir":"","from_version":"v1; rm -rf /","to_version":"v2"}' >"$MARK"; refuse "shell metacharacters in a version field"
ln -s /opt/victim /opt/.singbox-vpn-prev-2; printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/.singbox-vpn-prev-2","staged_src_dir":"","from_version":"v1","to_version":"v2"}' >"$MARK"; refuse "prev_opt_dir is a symlink"; rm -f /opt/.singbox-vpn-prev-2
[ -f /opt/victim/f ] && ok "symlink target never touched by any refused marker" || bad "victim damaged"

# --- 7. valid marker whose backup has no deployment.toml snapshot => PREPARE never finished: only the marker + staging are cleared, backup left
printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/.singbox-vpn-prev-1","staged_src_dir":"/opt/.singbox-vpn-update-staging.M1/src","from_version":"v1.3.0","to_version":"v1.3.1-rc.2"}' >"$MARK"
: >/var/lib/singbox-vpn/install-state.json.tmp   # a kill just before the commit rename leaves this behind
run
[ ! -e /var/lib/singbox-vpn/install-state.json.tmp ] && ok "an uncommitted manifest .tmp is removed by recovery" || bad "install-state.json.tmp survived recovery"
{ [ "$RC" -eq 0 ] && [ ! -e "$MARK" ] && [ ! -d /opt/.singbox-vpn-update-staging.M1 ] && [ -d /opt/singbox-vpn ] && [ ! -d /etc/vpn/backups/20261020-100000-1 ]; } \
  && ok "no snapshot in the backup => nothing was mutated => marker, staging AND the useless transaction backup cleared, live tree kept" || bad "prepare-incomplete case wrong (rc=$RC): $(tail -3 $REC)"
mkdir -p /etc/vpn/backups/20261020-100000-1   # the next cases reuse the same fixture backup
# ...but if the source tree WAS moved aside while the backup is incomplete => inconsistent => refuse
mkdir -p /opt/.singbox-vpn-prev-1; cp "$MARK" /dev/null 2>&1; printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/.singbox-vpn-prev-1","staged_src_dir":"","from_version":"v1.3.0","to_version":"v1.3.1-rc.2"}' >"$MARK"
run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ] && [ -d /opt/.singbox-vpn-prev-1 ] && grep -q 'inconsistent transaction' $REC; } && ok "tree moved aside but backup incomplete: refused as inconsistent, nothing deleted" || bad "inconsistent case wrong (rc=$RC)"

# --- 8. COMMIT-BOUNDARY kills (found by matrix stage 10): the manifest rename is the commit point; the backup, previous tree and
#        staging are deleted afterwards and the marker last. A kill in that gap leaves a marker naming a backup that is gone.
printf '#!/bin/sh\nexit 0\n' >/usr/local/bin/systemctl; chmod +x /usr/local/bin/systemctl
export PATH="/usr/local/bin:$PATH"
printf 'fake vpn-admin build\n' >/usr/local/bin/vpn-admin; chmod +x /usr/local/bin/vpn-admin
printf 'fake vpn build\n' >/usr/local/bin/vpn; printf 'fake subscription build\n' >/usr/local/bin/vpn-subscription-svc; chmod +x /usr/local/bin/vpn /usr/local/bin/vpn-subscription-svc
SHA="$(sha256sum /usr/local/bin/vpn-admin | cut -d' ' -f1)"
SHA_VPN="$(sha256sum /usr/local/bin/vpn | cut -d' ' -f1)"; SHA_SUB="$(sha256sum /usr/local/bin/vpn-subscription-svc | cut -d' ' -f1)"
MAN=/var/lib/singbox-vpn/install-state.json
START="$(date +%s)"
mkdir -p /opt/singbox-vpn/deploy/almalinux; : >/opt/singbox-vpn/deploy/almalinux/update.sh
fixture() { # $1=manifest version  $2=installed_at offset (s, relative to START)  $3=vpn-admin sha  $4=vpn sha  $5=subscription sha ; backup dir is absent
  rm -rf /etc/vpn/backups/20261020-100000-1 /opt/.singbox-vpn-prev-9 /opt/.singbox-vpn-update-staging.C9
  printf '{"singbox_vpn_version":"%s","installed_at_unix":%s,"vpn_admin_sha256":"%s","vpn_sha256":"%s","subscription_sha256":"%s"}' "$1" "$((START + $2))" "$3" "${4:-$SHA_VPN}" "${5:-$SHA_SUB}" >"$MAN"
  printf '{"backup_dir":"/etc/vpn/backups/20261020-100000-1","prev_opt_dir":"/opt/.singbox-vpn-prev-9","staged_src_dir":"/opt/.singbox-vpn-update-staging.C9/src","from_version":"v1.3.0","to_version":"v1.3.1-rc.2","started_at_unix":%s}' "$START" >"$MARK"
}
# 8a. committed + cleanup half done (prev dir PARTIALLY deleted, backup gone): roll FORWARD, never restore the partial tree
fixture v1.3.1-rc.2 5 "$SHA"; mkdir -p /opt/.singbox-vpn-prev-9/deploy; echo half >/opt/.singbox-vpn-prev-9/deploy/x
run; { [ "$RC" -eq 0 ] && [ ! -e "$MARK" ] && [ ! -d /opt/.singbox-vpn-prev-9 ] && [ -f /opt/singbox-vpn/deploy/almalinux/update.sh ] && grep -q 'committed' $REC; } \
  && ok "kill after the manifest commit: rolled FORWARD (marker + leftovers cleared, live tree kept, partial prev tree NOT restored)" || bad "commit-boundary roll-forward wrong (rc=$RC): $(tail -3 $REC)"
# 8b. committed, nothing left but the marker
fixture v1.3.1-rc.2 5 "$SHA"; run; { [ "$RC" -eq 0 ] && [ ! -e "$MARK" ]; } && ok "only the marker left after a completed update: cleared" || bad "marker-only case (rc=$RC): $(tail -2 $REC)"
# 8c. NOT committed evidence => refuse (backup is gone and nothing proves the update finished)
fixture v1.3.0 5 "$SHA"; run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ]; } && ok "manifest still at the OLD version and the backup is gone: refused (inconsistent)" || bad "accepted an uncommitted transaction"
fixture v1.3.1-rc.2 -50 "$SHA"; run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ]; } && ok "manifest older than the transaction start: refused (not written by this transaction)" || bad "accepted a stale manifest"
fixture v1.3.1-rc.2 5 "0000000000000000000000000000000000000000000000000000000000000000"; run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ]; } && ok "manifest hash does not match the installed vpn-admin: refused" || bad "accepted a hash mismatch"
fixture v1.3.1-rc.2 5 "$SHA" "0000000000000000000000000000000000000000000000000000000000000000"; run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ]; } && ok "manifest hash of vpn does not match: refused" || bad "accepted a vpn hash mismatch"
fixture v1.3.1-rc.2 5 "$SHA" "$SHA_VPN" "0000000000000000000000000000000000000000000000000000000000000000"; run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ]; } && ok "manifest hash of vpn-subscription-svc does not match: refused" || bad "accepted a subscription hash mismatch"
# 8d. committed but the live source tree is missing / services down => explicit refusal, nothing deleted
fixture v1.3.1-rc.2 5 "$SHA"; mkdir -p /opt/.singbox-vpn-prev-9/keep; mv /opt/singbox-vpn /opt/singbox-vpn.away
run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ] && [ -d /opt/.singbox-vpn-prev-9/keep ]; } && ok "committed but /opt/singbox-vpn is missing: refused, previous tree kept" || bad "rolled forward without a live tree (rc=$RC)"
mv /opt/singbox-vpn.away /opt/singbox-vpn
printf '#!/bin/sh\n[ "$1" = is-active ] && exit 3\nexit 0\n' >/usr/local/bin/systemctl
fixture v1.3.1-rc.2 5 "$SHA"; mkdir -p /opt/.singbox-vpn-prev-9/keep
run; { [ "$RC" -ne 0 ] && [ -e "$MARK" ] && [ -d /opt/.singbox-vpn-prev-9/keep ]; } && ok "committed but a service is not active: refused, nothing deleted" || bad "rolled forward on an unhealthy host (rc=$RC)"
printf '#!/bin/sh\nexit 0\n' >/usr/local/bin/systemctl
rm -rf /opt/.singbox-vpn-prev-9

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
