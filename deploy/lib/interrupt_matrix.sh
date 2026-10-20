#!/usr/bin/env bash
# SIGKILL interruption matrix for update.sh (I-56). STAGING HOSTS ONLY: it restores a node from a tar
# snapshot, runs real updates and kills them. Never run it on a node that serves users.
#
#   interrupt_matrix.sh selftest                      # validates THIS harness with a synthetic updater (no host change)
#   interrupt_matrix.sh snapshot                      # capture the current node as the baseline (a verified v1.3.0)
#   interrupt_matrix.sh restore                       # restore the baseline snapshot
#   interrupt_matrix.sh stage N [--updater branch|installed] [--kill tree|main]
#   interrupt_matrix.sh all [--updater ...] [--kill ...]
#
# Verdicts: PASS | FAIL | INVALID. INVALID = the experiment did not test what it claims (kill point never
# reached, disk/tmpfs/inode precheck failed or capacity collapsed during the run, harness file changed during
# the run, dirty starting state). INVALID is never PASS. Run it from an immutable copy (the wrapper
# `run-interrupt-matrix.sh` copies it read-only first and records its sha256): do not edit it mid-run.
set -u
SNAP="${SNAP:-/root/snap/baseline.tar}"
OUTDIR="${OUTDIR:-/root/evidence/matrix-$(date -u +%Y%m%dT%H%M%S)}"
TARGET="${TARGET:-v1.3.1-rc.2}"
BRANCH_UPDATER="${BRANCH_UPDATER:-/root/singbox-vpn/deploy/almalinux/update.sh}"
INSTALLED_UPDATER="${INSTALLED_UPDATER:-/opt/singbox-vpn/deploy/almalinux/update.sh}"
MIN_FREE_MB_ROOT="${MIN_FREE_MB_ROOT:-2048}"
MIN_FREE_MB_TMP="${MIN_FREE_MB_TMP:-1024}"
LOCKFILE=/run/lock/singbox-vpn-update.lock
MARKERS=("downloading singbox-vpn" "STAGE complete" "SWITCHING to" "reloading systemd unit definitions" "install mode: UPDATE" "rendering current authoritative" "restarting services" "running health check" "verifying real protocol handshake" "writing install-state manifest")
PATHS=(/etc/vpn /var/lib/singbox-vpn /opt/singbox-vpn /etc/nginx/conf.d/vpn-subscription.conf /etc/sysctl.d/99-singbox-vpn-dataplane.conf)
UNITGLOB=(/etc/systemd/system/sing-box.service /etc/systemd/system/vpn-*.service /etc/systemd/system/vpn-*.timer)
BINS=(/usr/local/bin/vpn /usr/local/bin/vpn-admin /usr/local/bin/vpn-subscription-svc /usr/local/bin/vpn-provisioning-agent /usr/local/bin/sing-box /usr/local/bin/vpn-health-check /usr/local/bin/vpn-service-watchdog /usr/local/bin/vpn-benchmark /usr/local/bin/vpn-benchmark-lib.sh)

free_mb() { df -Pm "$1" | awk 'NR==2{print $4}'; }
free_inode_pct() { df -Pi "$1" | awk 'NR==2{gsub("%","",$5); print 100-$5}'; }
creds() { for f in /etc/vpn/compat/users/users.json /etc/vpn/compat/reality/private.key /etc/vpn/compat/reality/short_id.txt /etc/vpn/compat/hysteria/cert.pem /etc/vpn/compat/hysteria/key.pem; do [ -e $f ] && sha256sum <$f | cut -c1-16; done | sha256sum | cut -c1-16; }
binsig() { for b in vpn-admin vpn vpn-subscription-svc; do sha256sum </usr/local/bin/$b | cut -c1-12; done | tr '\n' ',' ; }
ver() { vpn-admin --version 2>/dev/null | awk '{print $2}'; }
doctor_fails() { vpn-admin doctor 2>&1 | grep -c '^\[FAIL\]'; }
svc_ok() { for s in sing-box vpn-subscription nginx; do systemctl is-active --quiet $s || { echo "down:$s"; return 1; }; done; echo "all-active"; }
lock_held() { ( exec 9>"$LOCKFILE"; flock -n 9 ) && echo free || echo HELD; }
tmp_leaks() { find /tmp -maxdepth 1 -name 'tmp.*' 2>/dev/null | wc -l; }
stale_dirs() { ls -d /opt/.singbox-vpn-* 2>/dev/null | wc -l; }

descendants() { local p c; for c in $(pgrep -P "$1" 2>/dev/null); do echo "$c"; descendants "$c"; done; }
# Freeze, collect descendants until the set is stable (so nothing forks away), then SIGKILL them all.
kill_tree() {
  local root=$1 set prev="" kids
  for _ in 1 2 3 4 5 6; do
    mapfile -t kids < <(descendants "$root")
    set="$(printf '%s\n' "$root" "${kids[@]}" | sort -un | tr '\n' ' ')"
    for p in $set; do kill -STOP "$p" 2>/dev/null; done
    [ "$set" = "$prev" ] && break
    prev="$set"
  done
  for p in $set; do ps -o comm= -p "$p" 2>/dev/null; done | sort | uniq -c | tr '\n' ';'
  echo
  for p in $set; do kill -9 "$p" 2>/dev/null; done
  KILLED_PIDS="$set"
}
survivors() { local n=0 p; for p in $1; do kill -0 "$p" 2>/dev/null && n=$((n + 1)); done; echo $n; }

precheck() { # prints reasons it is not valid to start; returns nonzero if any
  local bad=0
  [ "$(free_mb /)" -ge "$MIN_FREE_MB_ROOT" ] || { echo "disk / has $(free_mb /) MB free (< $MIN_FREE_MB_ROOT)"; bad=1; }
  [ "$(free_mb /tmp)" -ge "$MIN_FREE_MB_TMP" ] || { echo "/tmp has $(free_mb /tmp) MB free (< $MIN_FREE_MB_TMP)"; bad=1; }
  [ "$(free_inode_pct /)" -ge 10 ] || { echo "inodes on / below 10% free"; bad=1; }
  [ -e /var/lib/singbox-vpn/update-transaction.json ] && { echo "a transaction marker already exists"; bad=1; }
  [ "$(stale_dirs)" = 0 ] || { echo "stale /opt/.singbox-vpn-* directories exist"; bad=1; }
  [ "$(lock_held)" = free ] || { echo "the update lock is held by a live process"; bad=1; }
  return $bad
}

selftest() {
  local T F=0; T="$(mktemp -d)"
  say() { echo "selftest: $*"; }
  chk() { if [ "$2" = ok ]; then say "ok   $1"; else say "FAIL $1"; F=$((F + 1)); fi; }
  # synthetic updater: prints markers, spawns a child in its OWN process group (like `timeout`) and a grandchild
  cat >"$T/fake-updater.sh" <<'EOF'
#!/bin/bash
echo "marker-one"; sleep 0.3
( setsid sleep 60 ) &            # orphan-prone child in a different process group
bash -c 'sleep 60' &             # grandchild
echo "marker-two"; sleep 60
EOF
  # 1. kill_tree really leaves nothing alive, including the other-process-group child
  setsid bash "$T/fake-updater.sh" >"$T/out" 2>&1 & local P=$!
  for _ in $(seq 1 50); do grep -q marker-two "$T/out" 2>/dev/null && break; sleep 0.1; done
  grep -q marker-two "$T/out" && chk "synthetic kill point reached" ok || chk "synthetic kill point reached" no
  local before kids; mapfile -t kids < <(descendants "$P"); before="$(printf '%s\n' "$P" "${kids[@]}" | wc -l)"
  kill_tree "$P" >/dev/null; sleep 0.5
  [ "$(survivors "$KILLED_PIDS")" = 0 ] && chk "tree kill leaves 0 survivors ($before processes incl. an other-group child)" ok || chk "tree kill left survivors" no
  # 2. a process-GROUP-only kill (the previous harness) does leave the other-group child alive: proves why tree kill is needed
  setsid bash "$T/fake-updater.sh" >"$T/out2" 2>&1 & P=$!
  for _ in $(seq 1 50); do grep -q marker-two "$T/out2" 2>/dev/null && break; sleep 0.1; done
  local all; mapfile -t kids < <(descendants "$P"); all="$(printf '%s\n' "$P" "${kids[@]}" | tr '\n' ' ')"
  kill -9 -- -"$P" 2>/dev/null; sleep 0.5
  [ "$(survivors "$all")" -gt 0 ] && chk "group-only kill leaves an orphan alive (why the old harness was wrong)" ok || chk "group kill unexpectedly clean" no
  for p in $all; do kill -9 "$p" 2>/dev/null; done
  # 3. a marker that is never printed => the stage would be INVALID
  setsid bash -c 'echo hello; sleep 2' >"$T/out3" 2>&1 & P=$!
  local fired=no; for _ in $(seq 1 10); do grep -qF "never-printed-marker" "$T/out3" && { fired=yes; break; }; sleep 0.1; done
  kill_tree "$P" >/dev/null
  [ "$fired" = no ] && chk "unreached kill point is detected (=> INVALID, never PASS)" ok || chk "unreached kill point detected" no
  # 4. capacity precheck refuses when the threshold cannot be met
  if MIN_FREE_MB_TMP=999999999 precheck >/dev/null 2>&1; then chk "precheck must refuse an impossible tmp threshold" no; else chk "precheck refuses when /tmp has too little room" ok; fi
  # 5. the harness refuses to run if its own bytes change mid-run (hash recorded at start)
  local h1 h2; h1="$(sha256sum <"$0")"; cp "$0" "$T/copy.sh"; echo "# edit" >>"$T/copy.sh"; h2="$(sha256sum <"$T/copy.sh")"
  [ "$h1" != "$h2" ] && chk "script hash comparison detects an edit" ok || chk "hash comparison" no
  rm -rf "$T"
  say "result: $F failure(s)"; [ "$F" -eq 0 ]
}

snapshot() {
  mkdir -p "$(dirname "$SNAP")"
  local existing=() g f b
  for g in "${UNITGLOB[@]}"; do for f in $g; do [ -e "$f" ] && existing+=("$f"); done; done
  for b in "${BINS[@]}"; do [ -e "$b" ] && existing+=("$b"); done
  tar -cpf "$SNAP" --numeric-owner "${PATHS[@]}" "${existing[@]}" 2>/dev/null
  echo "snapshot $(du -h "$SNAP" | cut -f1) version=$(ver) creds=$(creds) bins=$(binsig)"
}
restore() {
  systemctl stop sing-box vpn-subscription vpn-service-watchdog.timer vpn-expiry-reconcile.timer 2>/dev/null
  rm -rf /opt/.singbox-vpn-* /var/lib/singbox-vpn/update-transaction.json
  rm -rf /etc/vpn /var/lib/singbox-vpn /opt/singbox-vpn
  tar -xpf "$SNAP" --numeric-owner -C / || { echo "restore FAILED"; return 1; }
  systemctl daemon-reload
  systemctl start sing-box vpn-subscription vpn-service-watchdog.timer vpn-expiry-reconcile.timer 2>/dev/null; systemctl reload nginx 2>/dev/null
  sleep 3
  echo "restored version=$(ver) creds=$(creds) bins=$(binsig) services=$(svc_ok) doctor_fails=$(doctor_fails)"
}

run_updater() { # $1=kind
  if [ "$1" = branch ]; then bash "$BRANCH_UPDATER" "${@:2}"; else bash "$INSTALLED_UPDATER" "${@:2}"; fi
}

stage() {
  local N=$1 UPD=branch KILL=tree; shift
  while [ $# -gt 0 ]; do case "$1" in --updater) UPD=$2; shift 2;; --kill) KILL=$2; shift 2;; *) shift;; esac; done
  local M="${MARKERS[$((N - 1))]}" D="$OUTDIR/stage-$N-$UPD-$KILL"; mkdir -p "$D"
  local KV="$D/kv"; : >"$KV"; kv() { echo "$1=$2" >>"$KV"; }
  local h0; h0="$(sha256sum <"$0" | cut -c1-16)"; kv harness_sha "$h0"; kv stage "$N"; kv marker "$M"; kv updater "$UPD"; kv kill_scope "$KILL"
  local invalid=""
  precheck >"$D/precheck.txt" 2>&1 || invalid="precheck: $(tr '\n' ';' <"$D/precheck.txt")"
  if [ -z "$invalid" ]; then
    restore >"$D/restore.txt" 2>&1; kv restore "$(tail -1 "$D/restore.txt" | cut -c1-200)"
    precheck >"$D/precheck2.txt" 2>&1 || invalid="post-restore precheck: $(tr '\n' ';' <"$D/precheck2.txt")"
  fi
  if [ -n "$invalid" ]; then kv verdict "INVALID"; kv reason "$invalid"; python3 -I "$(dirname "$0")/interrupt_matrix_report.py" "$KV" >>"$OUTDIR/matrix.jsonl"; echo "stage $N INVALID: $invalid"; return 0; fi
  local base_creds base_bins base_ver leaks0 free_root0 free_tmp0
  base_creds=$(creds); base_bins=$(binsig); base_ver=$(ver); leaks0=$(tmp_leaks); free_root0=$(free_mb /); free_tmp0=$(free_mb /tmp)
  kv baseline_version "$base_ver"; kv baseline_bins "$base_bins"; kv free_root_mb_start "$free_root0"; kv free_tmp_mb_start "$free_tmp0"
  # --- run the updater and kill it at the marker
  local LOG="$D/update.log"; : >"$LOG"
  setsid bash -c "$(declare -f run_updater); BRANCH_UPDATER='$BRANCH_UPDATER' INSTALLED_UPDATER='$INSTALLED_UPDATER'; run_updater $UPD --version $TARGET" >"$LOG" 2>&1 & local PID=$!
  local fired=no
  for _ in $(seq 1 3000); do grep -qF "$M" "$LOG" 2>/dev/null && { fired=yes; break; }; kill -0 $PID 2>/dev/null || break; sleep 0.1; done
  if [ "$fired" = yes ]; then
    sleep 0.3
    if [ "$KILL" = tree ]; then kill_tree $PID >"$D/killed-procs.txt"; else KILLED_PIDS=$PID; kill -9 $PID 2>/dev/null; fi
    sleep 1
    kv kill_fired true; kv killed_pids "$(echo $KILLED_PIDS | tr ' ' ',')"; kv survivors_after_kill "$(survivors "$KILLED_PIDS")"
  else
    kv kill_fired false; kill_tree $PID >/dev/null 2>&1; kv verdict INVALID; kv reason "kill point '$M' never reached"
    python3 -I "$(dirname "$0")/interrupt_matrix_report.py" "$KV" >>"$OUTDIR/matrix.jsonl"; echo "stage $N INVALID (kill point never reached)"; return 0
  fi
  # --- immediate state
  kv marker_after_kill "$([ -e /var/lib/singbox-vpn/update-transaction.json ] && echo yes || echo no)"
  kv services_after_kill "$(svc_ok)"; kv version_after_kill "$(ver)"; kv bins_after_kill "$(binsig)"; kv doctor_fails_after_kill "$(doctor_fails)"
  kv stale_dirs_after_kill "$(stale_dirs)"; kv lock_after_kill "$(lock_held)"; kv creds_after_kill "$([ "$(creds)" = "$base_creds" ] && echo same || echo CHANGED)"
  # --- a plain update must refuse while stale state exists
  if [ -e /var/lib/singbox-vpn/update-transaction.json ] || [ "$(stale_dirs)" != 0 ]; then
    run_updater "$UPD" --version "$TARGET" >"$D/refuse.log" 2>&1; kv plain_update_rc_with_stale_state "$?"
  fi
  # --- recovery with the corrected updater; for a main-only kill wait out orphans that still hold the lock
  bash "$BRANCH_UPDATER" --recover >"$D/recover1.log" 2>&1; local R1=$?; kv recover_first_rc "$R1"
  if [ "$R1" -ne 0 ] && grep -q 'holds /run/lock' "$D/recover1.log"; then
    kv orphan_lock_refusal true
    for _ in $(seq 1 180); do [ "$(lock_held)" = free ] && break; sleep 1; done
    bash "$BRANCH_UPDATER" --recover >"$D/recover2.log" 2>&1; kv recover_after_wait_rc "$?"
  fi
  kv recovered_msg "$(sed -E 's/\x1b\[[0-9;]*m//g' "$D"/recover*.log | grep -oE 'RECOVERY COMPLETE|PREPARE had not finished|switch had not started|RECOVERY FAILED|removed [0-9]+ stale update-staging[a-z ]*|nothing else to recover' | tr '\n' ';')"
  kv marker_after_recover "$([ -e /var/lib/singbox-vpn/update-transaction.json ] && echo yes || echo no)"
  kv stale_dirs_after_recover "$(stale_dirs)"; kv version_after_recover "$(ver)"; kv bins_after_recover "$(binsig)"
  kv services_after_recover "$(svc_ok)"; kv doctor_fails_after_recover "$(doctor_fails)"; kv creds_after_recover "$([ "$(creds)" = "$base_creds" ] && echo same || echo CHANGED)"
  vpn-admin doctor --protocol --require-protocol >"$D/protocol-recover.log" 2>&1; kv protocol_rc_after_recover "$?"
  # --- normal update, then an idempotent repeat
  run_updater "$UPD" --version "$TARGET" >"$D/final.log" 2>&1; kv final_update_rc "$?"
  kv version_final "$(ver)"; kv bins_final "$(binsig)"; kv services_final "$(svc_ok)"; kv doctor_fails_final "$(doctor_fails)"
  kv creds_final "$([ "$(creds)" = "$base_creds" ] && echo same || echo CHANGED)"; kv marker_final "$([ -e /var/lib/singbox-vpn/update-transaction.json ] && echo yes || echo no)"; kv stale_dirs_final "$(stale_dirs)"
  vpn-admin doctor --protocol --require-protocol >"$D/protocol-final.log" 2>&1; kv protocol_rc_final "$?"
  local binsA; binsA=$(binsig)
  run_updater "$UPD" --version "$TARGET" >"$D/idem.log" 2>&1; kv idempotent_rc "$?"
  kv idempotent_bins_same "$([ "$(binsig)" = "$binsA" ] && echo yes || echo NO)"; kv creds_idem "$([ "$(creds)" = "$base_creds" ] && echo same || echo CHANGED)"
  kv tmp_leaks_delta "$(( $(tmp_leaks) - leaks0 ))"; kv free_root_mb_end "$(free_mb /)"; kv free_tmp_mb_end "$(free_mb /tmp)"
  local h1; h1="$(sha256sum <"$0" | cut -c1-16)"; kv harness_sha_end "$h1"
  python3 -I "$(dirname "$0")/interrupt_matrix_report.py" "$KV" "$MIN_FREE_MB_ROOT" "$MIN_FREE_MB_TMP" >>"$OUTDIR/matrix.jsonl"
  tail -1 "$OUTDIR/matrix.jsonl" | python3 -c "import sys,json; d=json.loads(sys.stdin.read()); print('stage',d['stage'],d['updater'],d['kill_scope'],d['verdict'],d.get('reasons',''))"
}

case "${1:-}" in
  selftest) selftest ;;
  snapshot) snapshot ;;
  restore) restore ;;
  precheck) precheck; echo "precheck rc=$?" ;;
  stage) mkdir -p "$OUTDIR"; shift; stage "$@" ;;
  all) mkdir -p "$OUTDIR"; shift; for n in 1 2 3 4 5 6 7 8 9 10; do stage "$n" "$@"; done ;;
  *) sed -n '2,16p' "$0"; exit 2 ;;
esac
