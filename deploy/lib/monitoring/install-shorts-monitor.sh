#!/usr/bin/env bash
# Deploy AND verify the Shorts media-health monitor from the installed tree (I-70). Idempotent.
#
#   install-shorts-monitor.sh [--label L ...]            install/refresh the shipped units, enable the timer(s)
#   install-shorts-monitor.sh --check [--label L ...]    read-only verification of what is ACTUALLY deployed; exit 1 on any FAIL
#
# Default label: exit-direct. Verified: unit files byte-identical to the shipped ones (so ExecStart points at the installed tree, not
# a scratch checkout), probe prerequisites (/etc/vpn/shorts-health.env: PROBE_PYTHON imports playwright, browsers path present),
# timer enabled and active, last run recent, and the monitor not blind (UNVERIFIED/HARNESS = the probe cannot run).
# A CRITICAL/DEGRADED MEDIA verdict is reported (INFO) but is not a deployment failure: it is the monitor doing its job.
# Install refuses, installing nothing, when the prerequisites do not hold. There is deliberately no automatic re-routing.
#
# I-83: install and --check also run the classifier that the units will execute against the real 155-byte UMP failure
# signature. It must answer MEDIA_PLAYBACK_FAILED / UNKNOWN. A repair or downgrade that restored the old classifier, which
# called that signature EGRESS_REFUSED, therefore fails visibly instead of silently.
#
# Test/override environment: SHORTS_UNIT_DIR SHORTS_STATE_DIR SHORTS_ENV_FILE SYSTEMCTL SHORTS_MAX_AGE_S SHORTS_CLASSIFIER
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNIT_DIR="${SHORTS_UNIT_DIR:-/etc/systemd/system}"
STATE_DIR="${SHORTS_STATE_DIR:-/var/lib/singbox-vpn/shorts-health}"
ENV_FILE="${SHORTS_ENV_FILE:-/etc/vpn/shorts-health.env}"
SYSTEMCTL="${SYSTEMCTL:-systemctl}"
MAX_AGE_S="${SHORTS_MAX_AGE_S:-7200}"   # the timer runs every 30 min; 4 missed runs = the monitor has stopped
CLASSIFIER="${SHORTS_CLASSIFIER:-$HERE/shorts_health.py}"
UNITS=(vpn-shorts-health@.service vpn-shorts-health@.timer)
CHECK=0; LABELS=()
while [ "$#" -gt 0 ]; do case "$1" in
  --check) CHECK=1; shift;; --label) LABELS+=("$2"); shift 2;;
  -h|--help) sed -n 2,14p "${BASH_SOURCE[0]}"; exit 0;; *) echo "unknown argument: $1" >&2; exit 2;; esac; done
[ "${#LABELS[@]}" -gt 0 ] || LABELS=(exit-direct)
bad=0; pass() { echo "PASS  $1"; }; fail() { echo "FAIL  $1"; bad=$((bad + 1)); }; info() { echo "INFO  $1"; }

prereq() { # prints FAIL lines; used by install (refuse) and check
  local py="" br=""
  if [ ! -r "$ENV_FILE" ]; then fail "probe prerequisites: $ENV_FILE is missing (PROBE_PYTHON / PLAYWRIGHT_BROWSERS_PATH; see deploy/lib/monitoring/README.md)"; return; fi
  py="$(sed -n 's/^PROBE_PYTHON=//p' "$ENV_FILE" | tail -1)"; br="$(sed -n 's/^PLAYWRIGHT_BROWSERS_PATH=//p' "$ENV_FILE" | tail -1)"
  if [ -n "$py" ] && "$py" -c 'import playwright' >/dev/null 2>&1; then pass "PROBE_PYTHON can import playwright"; else fail "PROBE_PYTHON ('${py:-unset}') cannot import playwright: the probe cannot run"; fi
  if [ -n "$br" ] && [ -d "$br" ] && [ -n "$(ls -A "$br" 2>/dev/null)" ]; then pass "PLAYWRIGHT_BROWSERS_PATH present and non-empty"; else fail "PLAYWRIGHT_BROWSERS_PATH ('${br:-unset}') missing or empty: no browser for the probe"; fi
}

classifier_contract() { # I-83: outcome and explanation are separate; the 155-byte signature alone is not an IP refusal
  local got
  got="$(python3 -I - "$CLASSIFIER" 2>/dev/null <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("sh", sys.argv[1]); sh = importlib.util.module_from_spec(spec); spec.loader.exec_module(sh)
r = {"kind": "short", "counts_as_short": True, "play": False, "playabilityStatus": "OK", "ui_unavailable": True,
     "media": {"responses": 1, "bytes": 155, "content_types": ["application/vnd.yt-ump"]}}
e = sh.evaluate(0, {"results": [r, r, r]})
print(e.get("cause"), e.get("explanation"))
PY
)" || got=""
  if [ "$got" = "MEDIA_PLAYBACK_FAILED UNKNOWN" ]; then pass "classifier $CLASSIFIER separates outcome from explanation (I-83)"
  else fail "classifier $CLASSIFIER answers '${got:-error}' for the 155-byte UMP signature, want 'MEDIA_PLAYBACK_FAILED UNKNOWN': pre-I-83 classifier (repair/downgrade restored it?)"; fi
}

if [ "$CHECK" -eq 0 ]; then
  prereq_out="$(prereq; classifier_contract)"; if grep -q '^FAIL' <<<"$prereq_out"; then
    echo "$prereq_out"; echo "REFUSING to install: prerequisites do not hold (nothing was installed)."; exit 1; fi
  for u in "${UNITS[@]}"; do install -m0644 "$HERE/$u" "$UNIT_DIR/$u" || { echo "could not install $u" >&2; exit 1; }; done
  "$SYSTEMCTL" daemon-reload || exit 1
  for l in "${LABELS[@]}"; do "$SYSTEMCTL" enable --now "vpn-shorts-health@$l.timer" || { echo "could not enable the timer for $l" >&2; exit 1; }; done
  echo "installed ${UNITS[*]} from $HERE and enabled: ${LABELS[*]}"
  exit 0
fi

# ---- --check (read-only)
for u in "${UNITS[@]}"; do
  if [ ! -e "$UNIT_DIR/$u" ]; then fail "$u not installed in $UNIT_DIR (run install-shorts-monitor.sh)"
  elif cmp -s "$HERE/$u" "$UNIT_DIR/$u"; then pass "$u identical to the shipped unit"
  else fail "$u differs from the shipped unit (edited by hand, or an older release's copy): re-run install-shorts-monitor.sh"; fi
done
prereq
classifier_contract
now="$(date +%s)"
for l in "${LABELS[@]}"; do
  t="vpn-shorts-health@$l.timer"
  "$SYSTEMCTL" is-enabled --quiet "$t" 2>/dev/null && pass "$t enabled" || fail "$t is not enabled"
  "$SYSTEMCTL" is-active --quiet "$t" 2>/dev/null && pass "$t active" || fail "$t is not active (the monitor will not run)"
  sf="$STATE_DIR/$l.state.json"
  if [ ! -r "$sf" ]; then fail "$l: no state file: the monitor has never completed a run"; continue; fi
  lr="$(python3 -I -c 'import json,sys;print(int(json.load(open(sys.argv[1])).get("last_run",0)))' "$sf" 2>/dev/null || echo 0)"
  st="$(python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1])).get("status",""))' "$sf" 2>/dev/null || echo "")"
  age=$((now - lr))
  if [ "$lr" -gt 0 ] && [ "$age" -le "$MAX_AGE_S" ]; then pass "$l: last run ${age}s ago"; else fail "$l: last run ${age}s ago (limit ${MAX_AGE_S}s): the monitor has stopped or never ran"; fi
  case "$st" in
    UNVERIFIED|PATH_DOWN) fail "$l: the monitor is blind (status $st): the probe cannot produce media evidence, so nothing is being verified";;
    OK) pass "$l: last verdict OK";;
    *) info "$l: media verdict $st (the monitor is working; this is about the egress, not about the deployment)";;
  esac
done
[ "$bad" -eq 0 ] && echo "SHORTS MONITOR: deployed and operating" || { echo "SHORTS MONITOR: $bad problem(s)"; exit 1; }
