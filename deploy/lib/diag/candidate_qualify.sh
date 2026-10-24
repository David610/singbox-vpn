#!/usr/bin/env bash
# Qualify a candidate Google/YouTube egress VPS BEFORE any VPN stack is installed on it.
#
#   candidate_qualify.sh setup                         create the probe venv (Playwright + Chromium), once
#   candidate_qualify.sh check                         verify every prerequisite, change nothing
#   candidate_qualify.sh run --target USER@HOST --label NAME  (--host-key-sha256 FP | --known-hosts FILE)
#                            [--ssh-key FILE] [--port N] [--sustained-video ID] [options]   (see --help)
#   candidate_qualify.sh TARGET LABEL [SSH_KEY] [flags]   legacy form, maps to `run`
#
# What it measures (candidate = the VPS under test; "here" = the machine running this script):
#   1. network: TLS/SNI survey from the candidate over the Google/YouTube address set (pinned with
#      --ips-file, otherwise the union of what here and the candidate resolve), IPv6 and QUIC notes.
#   2. media: a headless-browser Shorts probe from here THROUGH a plain `ssh -D` tunnel to the candidate
#      (no VPN protocol, TCP only), after proving that tunnel really exits at the candidate's IP.
#   3. sustained: one long-form video played for --sustained-dwell seconds.
#
# Result: <evidence-dir>/verdict.json, a machine-readable PASS / FAIL / INCONCLUSIVE with reason codes
# (exit 0 / 1 / 2; 64 usage error; 130/143 interrupted). The tool never parses mixed stdout: every stage
# writes its own file. A PASS covers only the measured workload on this run. It is NOT DEVICE_VERIFIED.
# See deploy/lib/diag/README.md for the reason codes, thresholds and the clean-VPS setup.
set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="$HERE/qualify_lib.py"
PROBE_SCRIPT="${QUALIFY_PROBE_SCRIPT:-$HERE/shorts_media_probe.py}"
VENV="${QUALIFY_VENV:-${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/singbox-vpn-qualify/venv}"
PROBE_PY="${QUALIFY_PROBE_PYTHON:-$VENV/bin/python}"
REMOTE_PY="${QUALIFY_REMOTE_PYTHON:-python3}"
LOCAL_PY="${QUALIFY_LOCAL_PYTHON:-python3}"
DEFAULT_IDS="sAcElROnYIE,0HAETg7L15o,0uWzj4AiiZ8,0hkt2-EwMl4,5mU6SRS2Bxo,3II9AFi5U0M,147Jql-pxqk,1yfCBYdjLHU,19IX8zSEJXg"
EXIT_USAGE=64

# ---------------------------------------------------------------- output helpers
log() { printf '[qualify] %s\n' "$*" >&2; }
usage_error() {
  printf 'candidate_qualify.sh: %s\n(see --help)\n' "$*" >&2
  exit "$EXIT_USAGE"
}

usage() {
  cat <<'EOF'
usage:
  candidate_qualify.sh setup
  candidate_qualify.sh check
  candidate_qualify.sh run --target USER@HOST --label NAME  IDENTITY  [options]
  candidate_qualify.sh USER@HOST NAME [SSH_KEY] IDENTITY [options]      (legacy positional form)

IDENTITY (exactly one; the candidate's SSH host identity is never trusted silently):
  --host-key-sha256 SHA256:...   fingerprint you obtained out of band (provider console); verified
  --known-hosts FILE             a known_hosts file that already pins the candidate; verified
  --accept-unverified-host-key   trust what the network presents; the verdict can then be at most INCONCLUSIVE

options:
  --ssh-key FILE                 SSH identity for the candidate
  --port N                       candidate SSH port (default 22)
  --evidence-dir DIR             base directory for results (default $EVIDENCE_DIR or ./egress-qualification)
  --ips-file FILE                pin the Google address set (JSON array of IPv4) instead of discovering it
  --shorts-ids ID,ID,...         11-character YouTube Short ids (default: nine historical ids)
  --dwell S                      seconds to watch each Short (default 25)
  --min-shorts-played N          Shorts that must play per round (default: all of them)
  --rounds N                     repeat the Shorts set N times (default 2)
  --round-gap S                  pause between rounds (default 60)
  --sustained-video ID           long-form video id for the sustained check (required for PASS)
  --sustained | -s ID            legacy alias of --sustained-video
  --sustained-dwell S            seconds to play it (default 900)
  --min-sustained-s S            seconds of progress that count as sustained (default 600)
  --timeout S                    per remote step timeout (default 120)
  --probe-timeout S              hard limit for each browser run (default derived from the workload)

exit status: 0 PASS, 1 FAIL, 2 INCONCLUSIVE, 64 usage error, 130/143 interrupted
environment: QUALIFY_VENV, QUALIFY_PROBE_PYTHON, QUALIFY_REMOTE_PYTHON, EVIDENCE_DIR
EOF
}

# ---------------------------------------------------------------- validation
is_pos_int() { [[ "${1:-}" =~ ^[1-9][0-9]*$ ]]; }
is_uint() { [[ "${1:-}" =~ ^(0|[1-9][0-9]*)$ ]]; }
need_value() { [ "$2" -ge 2 ] || usage_error "$1 needs a value"; }

# ---------------------------------------------------------------- run-time state (all cleaned by cleanup_resources)
EVDIR=""
WORK=""
KH=""
TUNNEL_PID=""
CUR_PID=""
PROBE_PID=""
SLEEP_PID=""
REMOTE_DIR=""
SSH_READY=0
FINISHED=0
SSH_OPTS=()
SCP_OPTS=()

stage() { # stage NAME STATUS [CODE]
  [ -n "$EVDIR" ] || return 0
  printf '%s\t%s\t%s\n' "$1" "$2" "${3:-}" >>"$EVDIR/stages.tsv"
}

kill_pid() { # kill_pid PID : TERM, short wait, KILL; only ever a PID this script started
  local pid="${1:-}" n=0
  [ -n "$pid" ] || return 0
  kill -TERM "$pid" 2>/dev/null || return 0
  while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 20 ]; do
    sleep 0.1
    n=$((n + 1))
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

kill_group() { # kill_group PID : the probe runs in its own session, so PID == its process group
  local pid="${1:-}" pgid n=0
  [ -n "$pid" ] || return 0
  pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
  if [ -n "$pgid" ] && [ "$pgid" = "$pid" ]; then
    kill -TERM -- "-$pgid" 2>/dev/null || true
    while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 30 ]; do
      sleep 0.1
      n=$((n + 1))
    done
    kill -KILL -- "-$pgid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  else
    kill_pid "$pid"
  fi
}

cleanup_resources() {
  trap '' INT TERM HUP
  kill_pid "$CUR_PID"
  CUR_PID=""
  kill_group "$PROBE_PID"
  PROBE_PID=""
  kill_pid "$SLEEP_PID"
  SLEEP_PID=""
  kill_pid "$TUNNEL_PID"
  TUNNEL_PID=""
  if [ -n "$REMOTE_DIR" ] && [ "$SSH_READY" -eq 1 ]; then
    timeout 25 ssh "${SSH_OPTS[@]}" "$TARGET" "rm -rf $REMOTE_DIR" >/dev/null 2>&1 \
      || log "warning: could not remove $REMOTE_DIR on the candidate; delete it by hand"
    REMOTE_DIR=""
  fi
  if [ "$SSH_READY" -eq 1 ]; then
    timeout 10 ssh -O exit "${SSH_OPTS[@]}" "$TARGET" >/dev/null 2>&1 || true
  fi
  if [ -n "$WORK" ] && [ -d "$WORK" ]; then
    rm -rf "$WORK"
    WORK=""
  fi
}

# finish_and_exit [EXIT_CODE_OVERRIDE]: always leaves a verdict.json behind, then exits.
finish_and_exit() {
  local override="${1:-}" rc=0 line
  cleanup_resources
  if [ -n "$EVDIR" ]; then
    line="$("$LOCAL_PY" -I "$LIB" verdict --dir "$EVDIR")" || rc=$?
    log "verdict: $line (evidence: $EVDIR)"
    printf 'RESULT %s %s\n' "$line" "$EVDIR"
  fi
  FINISHED=1
  exit "${override:-$rc}"
}

on_signal() { # on_signal SIGNAME EXITCODE
  trap '' INT TERM HUP
  log "received SIG$1; cleaning up"
  stage run interrupted interrupted
  finish_and_exit "$2"
}

on_exit() {
  local rc=$?
  if [ "$FINISHED" -eq 0 ] && [ -n "$EVDIR" ]; then
    stage run failed internal_error
    cleanup_resources
    "$LOCAL_PY" -I "$LIB" verdict --dir "$EVDIR" >/dev/null 2>&1 || true
    log "internal error (exit $rc); a verdict.json was still written to $EVDIR"
  fi
}

# run_tracked STDOUT_FILE STDERR_FILE CMD...: background + wait so a signal is handled immediately
run_tracked() {
  local out="$1" err="$2" rc=0
  shift 2
  "$@" >"$out" 2>>"$err" &
  CUR_PID=$!
  wait "$CUR_PID" || rc=$?
  CUR_PID=""
  return "$rc"
}

# ---------------------------------------------------------------- prerequisites
PREREQ_MISSING=()
check_prereqs() {
  local c
  PREREQ_MISSING=()
  for c in ssh scp ssh-keyscan ssh-keygen timeout setsid ps; do
    command -v "$c" >/dev/null 2>&1 || PREREQ_MISSING+=("command:$c")
  done
  if ! command -v "$LOCAL_PY" >/dev/null 2>&1; then
    PREREQ_MISSING+=("command:$LOCAL_PY")
  elif ! "$LOCAL_PY" -I -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' 2>/dev/null; then
    PREREQ_MISSING+=("python>=3.9")
  fi
  [ -f "$LIB" ] || PREREQ_MISSING+=("file:qualify_lib.py")
  if [ ! -f "$PROBE_SCRIPT" ]; then
    PREREQ_MISSING+=("file:shorts_media_probe.py")
  elif [ ! -x "$PROBE_PY" ] && ! command -v "$PROBE_PY" >/dev/null 2>&1; then
    PREREQ_MISSING+=("probe-python:$PROBE_PY (run: candidate_qualify.sh setup)")
  elif ! "$PROBE_PY" "$PROBE_SCRIPT" --self-check >/dev/null 2>&1; then
    PREREQ_MISSING+=("probe-runtime: Playwright/Chromium not usable (run: candidate_qualify.sh setup)")
  fi
  if [ "$(id -u)" -eq 0 ] && [ "$PROBE_SCRIPT" = "$HERE/shorts_media_probe.py" ]; then
    PREREQ_MISSING+=("non-root user (Chromium's sandbox is left on and refuses to run as root)")
  fi
  [ "${#PREREQ_MISSING[@]}" -eq 0 ]
}

cmd_check() {
  [ $# -eq 0 ] || usage_error "check takes no arguments"
  if check_prereqs; then
    log "all prerequisites present"
    return 0
  fi
  printf 'missing prerequisite: %s\n' "${PREREQ_MISSING[@]}" >&2
  return 2
}

cmd_setup() {
  [ $# -eq 0 ] || usage_error "setup takes no arguments"
  command -v "$LOCAL_PY" >/dev/null 2>&1 || { log "python3 is required"; return 2; }
  log "creating probe venv at $VENV"
  "$LOCAL_PY" -m venv "$VENV"
  "$VENV/bin/python" -m pip install --quiet --require-virtualenv -r "$HERE/requirements-probe.txt"
  "$VENV/bin/python" -m playwright install chromium
  log "Linux needs Chromium's shared libraries once, as root:  $VENV/bin/python -m playwright install-deps chromium"
  "$VENV/bin/python" "$PROBE_SCRIPT" --self-check
  log "probe runtime ready; run: candidate_qualify.sh check"
}

# ---------------------------------------------------------------- run
TARGET="" LABEL="" KEY="" PORT=22 EV_BASE="${EVIDENCE_DIR:-./egress-qualification}"
FP="" KNOWN_HOSTS_FILE="" ACCEPT_UNVERIFIED=0 IPS_FILE=""
IDS="$DEFAULT_IDS" DWELL=25 MIN_PLAYED="" ROUNDS=2 ROUND_GAP=60
SUST_ID="" SUST_DWELL=900 MIN_SUST=600 STEP_TIMEOUT=120 PROBE_TIMEOUT=""
HOST_IDENTITY="unverified"

parse_run_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --target) need_value "$1" $#; TARGET="$2"; shift 2 ;;
      --label) need_value "$1" $#; LABEL="$2"; shift 2 ;;
      --ssh-key) need_value "$1" $#; KEY="$2"; shift 2 ;;
      --port) need_value "$1" $#; PORT="$2"; shift 2 ;;
      --evidence-dir) need_value "$1" $#; EV_BASE="$2"; shift 2 ;;
      --host-key-sha256) need_value "$1" $#; FP="$2"; shift 2 ;;
      --known-hosts) need_value "$1" $#; KNOWN_HOSTS_FILE="$2"; shift 2 ;;
      --accept-unverified-host-key) ACCEPT_UNVERIFIED=1; shift ;;
      --ips-file) need_value "$1" $#; IPS_FILE="$2"; shift 2 ;;
      --shorts-ids) need_value "$1" $#; IDS="$2"; shift 2 ;;
      --dwell) need_value "$1" $#; DWELL="$2"; shift 2 ;;
      --min-shorts-played) need_value "$1" $#; MIN_PLAYED="$2"; shift 2 ;;
      --rounds) need_value "$1" $#; ROUNDS="$2"; shift 2 ;;
      --round-gap) need_value "$1" $#; ROUND_GAP="$2"; shift 2 ;;
      --sustained-video | --sustained | -s) need_value "$1" $#; SUST_ID="$2"; shift 2 ;;
      --sustained-dwell) need_value "$1" $#; SUST_DWELL="$2"; shift 2 ;;
      --min-sustained-s) need_value "$1" $#; MIN_SUST="$2"; shift 2 ;;
      --timeout) need_value "$1" $#; STEP_TIMEOUT="$2"; shift 2 ;;
      --probe-timeout) need_value "$1" $#; PROBE_TIMEOUT="$2"; shift 2 ;;
      -h | --help) usage; exit 0 ;;
      *) usage_error "unknown option: $1" ;;
    esac
  done
}

validate_run_args() {
  local id n_ids=0 modes=0
  [ -n "$TARGET" ] || usage_error "--target USER@HOST is required"
  [[ "$TARGET" =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._:-]+$ ]] || usage_error "--target must look like user@host (got an unsafe value)"
  [ -n "$LABEL" ] || usage_error "--label is required"
  [[ "$LABEL" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,39}$ ]] || usage_error "--label must be 1-40 chars of [A-Za-z0-9._-], starting alphanumeric"
  is_pos_int "$PORT" && [ "$PORT" -le 65535 ] || usage_error "--port must be 1-65535"
  [ -z "$FP" ] || modes=$((modes + 1))
  [ -z "$KNOWN_HOSTS_FILE" ] || modes=$((modes + 1))
  [ "$ACCEPT_UNVERIFIED" -eq 0 ] || modes=$((modes + 1))
  [ "$modes" -eq 1 ] || usage_error "give exactly one of --host-key-sha256, --known-hosts, --accept-unverified-host-key"
  if [ -n "$FP" ]; then
    [[ "$FP" =~ ^SHA256:[A-Za-z0-9+/]{43}$ ]] || usage_error "--host-key-sha256 must look like SHA256:<43 base64 characters>"
  fi
  if [ -n "$KNOWN_HOSTS_FILE" ]; then
    [ -s "$KNOWN_HOSTS_FILE" ] || usage_error "--known-hosts file is missing or empty"
  fi
  if [ -n "$KEY" ]; then
    [ -r "$KEY" ] || usage_error "--ssh-key is not a readable file"
  fi
  if [ -n "$IPS_FILE" ]; then
    [ -s "$IPS_FILE" ] || usage_error "--ips-file is missing or empty"
  fi
  IFS=',' read -r -a _ids <<<"$IDS"
  for id in "${_ids[@]}"; do
    [[ "$id" =~ ^[A-Za-z0-9_-]{11}$ ]] || usage_error "--shorts-ids: '$id' is not an 11-character YouTube id"
    n_ids=$((n_ids + 1))
  done
  [ "$n_ids" -ge 1 ] || usage_error "--shorts-ids is empty"
  is_pos_int "$DWELL" || usage_error "--dwell must be a positive integer"
  is_pos_int "$ROUNDS" || usage_error "--rounds must be a positive integer"
  is_uint "$ROUND_GAP" || usage_error "--round-gap must be a non-negative integer"
  is_pos_int "$SUST_DWELL" || usage_error "--sustained-dwell must be a positive integer"
  is_pos_int "$MIN_SUST" || usage_error "--min-sustained-s must be a positive integer"
  is_pos_int "$STEP_TIMEOUT" || usage_error "--timeout must be a positive integer"
  if [ -n "$PROBE_TIMEOUT" ]; then is_pos_int "$PROBE_TIMEOUT" || usage_error "--probe-timeout must be a positive integer"; fi
  if [ -z "$MIN_PLAYED" ]; then MIN_PLAYED="$n_ids"; fi
  is_pos_int "$MIN_PLAYED" && [ "$MIN_PLAYED" -le "$n_ids" ] || usage_error "--min-shorts-played must be between 1 and the number of ids ($n_ids)"
  if [ -n "$SUST_ID" ]; then
    [[ "$SUST_ID" =~ ^[A-Za-z0-9_-]{11}$ ]] || usage_error "--sustained-video must be an 11-character YouTube id"
  fi
  N_IDS="$n_ids"
}

build_ssh_options() {
  local common=(-o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=3
    -o StrictHostKeyChecking=yes "-o" "UserKnownHostsFile=$KH" -o PasswordAuthentication=no
    -o KbdInteractiveAuthentication=no
    # ONE connection for the whole run: hosts rate-limit new SSH connections (ufw "LIMIT" allows 6 per
    # 30 s per source), and this tool needs a dozen steps plus the tunnel.
    -o ControlMaster=auto "-o" "ControlPath=$WORK/cm" -o ControlPersist=600)
  SSH_OPTS=("${common[@]}" -p "$PORT")
  SCP_OPTS=("${common[@]}" -P "$PORT")
  if [ -n "$KEY" ]; then
    SSH_OPTS+=(-i "$KEY" -o IdentitiesOnly=yes)
    SCP_OPTS+=(-i "$KEY" -o IdentitiesOnly=yes)
  fi
}

# establish_identity: sets KH and HOST_IDENTITY, or records the failing stage and finishes
establish_identity() {
  local host="${TARGET#*@}" scanned got
  if [ -n "$KNOWN_HOSTS_FILE" ]; then
    KH="$KNOWN_HOSTS_FILE"
    HOST_IDENTITY="verified"
    stage identity ok pinned_known_hosts
    return 0
  fi
  scanned="$WORK/hostkey.scan"
  if ! timeout 30 ssh-keyscan -T 10 -t ed25519 -p "$PORT" "$host" >"$scanned" 2>"$WORK/keyscan.err" || [ ! -s "$scanned" ]; then
    stage identity failed host_key_unavailable
    finish_and_exit
  fi
  if [ -n "$FP" ]; then
    got="$(ssh-keygen -lf "$scanned" | awk 'NR==1 {print $2}')"
    if [ "$got" != "$FP" ]; then
      log "host key mismatch: the candidate presents $got, expected $FP; not connecting"
      stage identity failed host_key_mismatch
      finish_and_exit
    fi
    HOST_IDENTITY="verified"
    stage identity ok fingerprint_matched
  else
    log "WARNING: trusting the host key the network presents; the verdict can be at most INCONCLUSIVE"
    HOST_IDENTITY="unverified"
    stage identity ok trust_on_first_use
  fi
  KH="$scanned"
}

remote_step() { # remote_step STAGE CODE OUTFILE REMOTE_COMMAND
  local st="$1" code="$2" out="$3" cmd="$4" rc=0
  run_tracked "$out" "$EVDIR/remote.log" timeout "$STEP_TIMEOUT" ssh "${SSH_OPTS[@]}" "$TARGET" "$cmd" || rc=$?
  if [ "$rc" -ne 0 ]; then
    log "remote step '$st' failed (exit $rc)"
    stage "$st" failed "$code"
    finish_and_exit
  fi
}

upload() { # upload STAGE FILES...
  local st="$1" rc=0
  shift
  run_tracked /dev/null "$EVDIR/remote.log" timeout "$STEP_TIMEOUT" scp -q "${SCP_OPTS[@]}" "$@" "$TARGET:$REMOTE_DIR/" || rc=$?
  if [ "$rc" -ne 0 ]; then
    log "upload for '$st' failed (exit $rc)"
    stage "$st" failed remote_stage_failed
    finish_and_exit
  fi
}

download() { # download STAGE REMOTE_NAME LOCAL_PATH
  local st="$1" rc=0
  run_tracked /dev/null "$EVDIR/remote.log" timeout "$STEP_TIMEOUT" scp -q "${SCP_OPTS[@]}" "$TARGET:$REMOTE_DIR/$2" "$3" || rc=$?
  if [ "$rc" -ne 0 ] || [ ! -s "$3" ]; then
    log "download of $2 failed (exit $rc)"
    stage "$st" failed remote_stage_failed
    finish_and_exit
  fi
}

# run_probe OUT IDS DWELL LABEL HARD_TIMEOUT -> returns the probe's status (124 = timed out)
run_probe() {
  local out="$1" ids="$2" dwell="$3" plabel="$4" hard="$5" rc=0
  setsid -w timeout --kill-after=5 "$hard" "$PROBE_PY" "$PROBE_SCRIPT" --proxy "socks5://127.0.0.1:$LOCAL_PORT" \
    --ids "$ids" --dwell "$dwell" --label "$plabel" --out "$out" >"$out.log" 2>&1 &
  PROBE_PID=$!
  wait "$PROBE_PID" || rc=$?
  kill_group "$PROBE_PID"
  PROBE_PID=""
  return "$rc"
}

probe_stage() { # probe_stage STAGE OUT IDS DWELL HARD_TIMEOUT
  local st="$1" rc=0
  log "stage $st: playing $3 for ${4}s each"
  run_probe "$2" "$3" "$4" "cand_${LABEL}_$st" "$5" || rc=$?
  case "$rc" in
    0) stage "$st" ok ;;
    124 | 137) stage "$st" timeout probe_timeout ;;
    *) stage "$st" failed probe_failed ;;
  esac
}

cmd_run() {
  local n now_ts rc=0 egress hard_shorts hard_sust
  parse_run_args "$@"
  validate_run_args

  now_ts="$(date -u +%Y%m%dT%H%M%SZ)"
  mkdir -p "$EV_BASE"
  EVDIR="$EV_BASE/$LABEL-$now_ts"
  n=1
  while ! mkdir -m 700 "$EVDIR" 2>/dev/null; do
    n=$((n + 1))
    EVDIR="$EV_BASE/$LABEL-$now_ts-$n"
  done
  EVDIR="$(cd "$EVDIR" && pwd)"
  WORK="$(mktemp -d)"
  chmod 700 "$WORK"
  : >"$EVDIR/stages.tsv"
  : >"$EVDIR/remote.log"
  trap 'on_signal INT 130' INT
  trap 'on_signal TERM 143' TERM
  trap 'on_signal HUP 129' HUP
  trap on_exit EXIT

  "$LOCAL_PY" -I - "$EVDIR/config.json" "$LABEL" "${TARGET#*@}" "$IDS" "$DWELL" "$MIN_PLAYED" "$ROUNDS" \
    "$SUST_ID" "$SUST_DWELL" "$MIN_SUST" <<'PY'
import json, sys
out, label, host, ids, dwell, minp, rounds, sid, sdw, mins = sys.argv[1:11]
json.dump({"label": label, "target_host": host, "shorts_ids": ids.split(","), "dwell": float(dwell),
           "min_shorts_played": int(minp), "rounds": int(rounds), "sustained_video": sid,
           "sustained_dwell": float(sdw), "min_sustained_s": float(mins), "host_identity": "unverified"},
          open(out, "w"))
PY

  if ! check_prereqs; then
    printf 'missing prerequisite: %s\n' "${PREREQ_MISSING[@]}" >&2
    stage prereq failed prereq_missing
    finish_and_exit
  fi
  stage prereq ok

  establish_identity
  "$LOCAL_PY" -I "$LIB" set-config "$EVDIR/config.json" host_identity "$HOST_IDENTITY"
  build_ssh_options

  # ---- candidate: private tempdir, helper scripts, address collection
  REMOTE_DIR="$(timeout "$STEP_TIMEOUT" ssh "${SSH_OPTS[@]}" "$TARGET" 'umask 077; mktemp -d "${TMPDIR:-/tmp}/qualify.XXXXXXXX"' 2>>"$EVDIR/remote.log")" || rc=$?
  if [ "$rc" -ne 0 ] || [[ ! "$REMOTE_DIR" =~ ^/[A-Za-z0-9._/-]+$ ]]; then
    REMOTE_DIR=""
    stage collect failed remote_stage_failed
    finish_and_exit
  fi
  SSH_READY=1
  upload collect "$HERE/qualify_remote.py" "$HERE/collect_ips.py" "$HERE/tls_survey.py" "$HERE/v6_check.py" "$HERE/quic_vn_probe.py"
  remote_step collect remote_stage_failed /dev/null "$REMOTE_PY -I $REMOTE_DIR/qualify_remote.py collect --out $REMOTE_DIR/collect.json"
  download collect collect.json "$EVDIR/collect.json"
  egress="$("$LOCAL_PY" -I "$LIB" get "$EVDIR/collect.json" egress_ipv4)"
  if [[ ! "$egress" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]]; then
    stage collect failed egress_ip_unknown
    finish_and_exit
  fi
  stage collect ok

  # ---- the address set: pinned, or the union of both vantage points (no /tmp dependency)
  if [ -n "$IPS_FILE" ]; then
    "$LOCAL_PY" -I "$LIB" merge-ips "$EVDIR/ips_union.json" "$IPS_FILE" || { stage survey failed ips_file_invalid; finish_and_exit; }
  else
    if run_tracked "$WORK/local_ips.json" "$EVDIR/remote.log" timeout 60 "$LOCAL_PY" -I "$HERE/collect_ips.py"; then
      "$LOCAL_PY" -I "$LIB" merge-ips "$EVDIR/ips_union.json" "$WORK/local_ips.json" "$EVDIR/collect.json" \
        || { stage survey failed ips_collection_failed; finish_and_exit; }
    else
      "$LOCAL_PY" -I "$LIB" merge-ips "$EVDIR/ips_union.json" "$EVDIR/collect.json" \
        || { stage survey failed ips_collection_failed; finish_and_exit; }
    fi
  fi
  upload survey "$EVDIR/ips_union.json"
  remote_step survey remote_stage_failed /dev/null "$REMOTE_PY -I $REMOTE_DIR/qualify_remote.py survey --ips $REMOTE_DIR/ips_union.json --out $REMOTE_DIR/survey.json"
  download survey survey.json "$EVDIR/survey.json"
  stage survey ok

  # a conclusive network failure ends the run: a browser test over a blocked network adds nothing
  rc=0
  "$LOCAL_PY" -I "$LIB" network-verdict --dir "$EVDIR" || rc=$?
  if [ "$rc" -ne 0 ]; then
    log "network stage is conclusive (exit $rc); skipping the browser stages"
    finish_and_exit
  fi

  # ---- tunnel: start, then PROVE it exits at the candidate before any media test uses it
  LOCAL_PORT="$("$LOCAL_PY" -I "$LIB" free-port)"
  ssh "${SSH_OPTS[@]}" -N -D "127.0.0.1:$LOCAL_PORT" -o ExitOnForwardFailure=yes "$TARGET" >/dev/null 2>>"$EVDIR/remote.log" &
  TUNNEL_PID=$!
  rc=0
  "$LOCAL_PY" -I "$LIB" socks-check --port "$LOCAL_PORT" --expect-ip "$egress" --deadline 25 --pid "$TUNNEL_PID" 2>>"$EVDIR/remote.log" || rc=$?
  case "$rc" in
    0) stage tunnel ok ;;
    3) stage tunnel failed tunnel_egress_mismatch; finish_and_exit ;;
    *) stage tunnel failed tunnel_not_ready; finish_and_exit ;;
  esac

  # ---- media rounds
  hard_shorts="${PROBE_TIMEOUT:-$((N_IDS * (DWELL + 15) + 90))}"
  hard_sust="${PROBE_TIMEOUT:-$((SUST_DWELL + 120))}"
  for ((n = 1; n <= ROUNDS; n++)); do
    probe_stage "shorts_round_$n" "$EVDIR/round_$n.json" "$IDS" "$DWELL" "$hard_shorts"
    if [ "$n" -lt "$ROUNDS" ] && [ "$ROUND_GAP" -gt 0 ]; then
      sleep "$ROUND_GAP" &
      SLEEP_PID=$!
      wait "$SLEEP_PID" || true
      SLEEP_PID=""
    fi
  done
  if [ -n "$SUST_ID" ]; then
    probe_stage sustained "$EVDIR/sustained.json" "watch:$SUST_ID" "$SUST_DWELL" "$hard_sust"
  else
    log "no --sustained-video: the run cannot reach PASS"
  fi
  finish_and_exit
}

# ---------------------------------------------------------------- dispatch
main() {
  case "${1:-}" in
    "") usage >&2; exit "$EXIT_USAGE" ;;
    -h | --help | help) usage; exit 0 ;;
    setup) shift; cmd_setup "$@" ;;
    check) shift; cmd_check "$@" ;;
    run) shift; cmd_run "$@" ;;
    -*) usage_error "unknown option or subcommand: $1" ;;
    *)
      # legacy: TARGET LABEL [SSH_KEY] [flags]; a leading "--" argument is never taken for the key
      [ $# -ge 2 ] || usage_error "legacy form needs TARGET and LABEL"
      local t="$1" l="$2"
      shift 2
      if [ $# -ge 1 ] && [[ "$1" != -* ]]; then
        local k="$1"
        shift
        cmd_run --target "$t" --label "$l" --ssh-key "$k" "$@"
      else
        cmd_run --target "$t" --label "$l" "$@"
      fi
      ;;
  esac
}

main "$@"
