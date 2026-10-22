#!/usr/bin/env bash
# deploy/lib/monitoring/install-shorts-monitor.sh: source-controlled deployment AND verification of the Shorts monitor (I-70).
# Hermetic: temp unit/state dirs, a fake systemctl, a fake python. Nothing touches the host.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
S="$ROOT/deploy/lib/monitoring/install-shorts-monitor.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fail=0; ok() { echo "ok: $1"; }; bad() { echo "FAIL: $1"; fail=$((fail + 1)); }

mkenv() { # fresh sandbox
  rm -rf "${T:?}/u" "${T:?}/st" "${T:?}/bin" "${T:?}/e" "${T:?}/fs"; mkdir -p "$T/u" "$T/st" "$T/bin" "$T/e/browsers/chromium"
  cat >"$T/bin/systemctl" <<'F'
#!/usr/bin/env bash
echo "$*" >>"$FAKE_SYSTEMCTL_LOG"
case "$1" in
  is-enabled) [ -e "$FAKE_STATE/${*: -1}.enabled" ] && exit 0 || exit 1 ;;
  is-active)  [ -e "$FAKE_STATE/${*: -1}.active" ] && exit 0 || exit 3 ;;
  enable) shift; [ "$1" = "--now" ] && shift; touch "$FAKE_STATE/$1.enabled" "$FAKE_STATE/$1.active"; exit 0 ;;
  daemon-reload) exit 0 ;;
esac
exit 0
F
  chmod +x "$T/bin/systemctl"; mkdir -p "$T/fs"
  printf '#!/usr/bin/env bash\n[ "$1" = "-c" ] && exit "${FAKE_PLAYWRIGHT_RC:-0}"\nexit 0\n' >"$T/bin/python-ok"; chmod +x "$T/bin/python-ok"
  printf 'PROBE_PYTHON=%s\nPLAYWRIGHT_BROWSERS_PATH=%s\n' "$T/bin/python-ok" "$T/e/browsers" >"$T/e/env"
  : >"$T/log"
}
run() { SHORTS_UNIT_DIR="$T/u" SHORTS_STATE_DIR="$T/st" SHORTS_ENV_FILE="$T/e/env" SYSTEMCTL="$T/bin/systemctl" \
        FAKE_SYSTEMCTL_LOG="$T/log" FAKE_STATE="$T/fs" FAKE_PLAYWRIGHT_RC="${FAKE_PLAYWRIGHT_RC:-0}" bash "$S" "$@" 2>&1; }
now() { date +%s; }
state() { printf '{"streak": 0, "status": "%s", "last_run": %s}\n' "$1" "$2" >"$T/st/exit-direct.state.json"; }

# I-83: a pre-I-83 classifier (the df47737 one) must make install refuse and --check fail.
# A shallow CI checkout has no df47737 object. Falling back to an empty file
# tested only import failure, never the actual wrong EGRESS_REFUSED classifier.
# Preserve the real historical source as data, independent of Git history.
cp "$ROOT/deploy/lib/tests/fixtures/shorts-health-pre-i83.py.txt" "$T/old_classifier.py"
old_cause="$(python3 -I - "$T/old_classifier.py" <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("old_shorts", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
r = {"kind": "short", "counts_as_short": True, "play": False, "playabilityStatus": "OK", "ui_unavailable": True,
     "media": {"responses": 1, "bytes": 155, "content_types": ["application/vnd.yt-ump"]}}
print(m.evaluate(0, {"results": [r, r, r]}).get("cause"))
PY
)"
[ "$old_cause" = EGRESS_REFUSED ] || { echo "FAIL: historical classifier did not reproduce EGRESS_REFUSED: $old_cause"; exit 1; }
mkenv; out="$(SHORTS_CLASSIFIER="$T/old_classifier.py" run --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && [ ! -e "$T/u/vpn-shorts-health@.timer" ] && grep -q 'pre-I-83 classifier' <<<"$out"; } && ok "install refuses a pre-I-83 classifier and installs nothing" || bad "old classifier accepted: rc=$rc $out"
mkenv; run --label exit-direct >/dev/null; out="$(SHORTS_CLASSIFIER="$T/old_classifier.py" run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*pre-I-83' <<<"$out"; } && ok "--check fails when the installed classifier reverted" || bad "revert not detected: rc=$rc"
mkenv; out="$(run --check --label exit-direct)"
grep -q 'PASS  classifier .* (I-83)' <<<"$out" && ok "--check passes the shipped classifier" || bad "shipped classifier not accepted: $out"

mkenv
out="$(run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*not installed' <<<"$out"; } && ok "--check on an empty host fails and says the monitor is not installed" || bad "empty host not reported: rc=$rc $out"

mkenv; out="$(run --label exit-direct)"; rc=$?
[ $rc -eq 0 ] && ok "install succeeds when prerequisites hold" || bad "install failed rc=$rc: $out"
cmp -s "$ROOT/deploy/lib/monitoring/vpn-shorts-health@.service" "$T/u/vpn-shorts-health@.service" && cmp -s "$ROOT/deploy/lib/monitoring/vpn-shorts-health@.timer" "$T/u/vpn-shorts-health@.timer" \
  && ok "installed units are byte-identical to the shipped ones" || bad "installed units differ from shipped"
grep -q 'enable --now vpn-shorts-health@exit-direct.timer' "$T/log" && grep -q daemon-reload "$T/log" && ok "daemon-reload + enable --now of the timer" || bad "timer not enabled: $(cat "$T/log")"
state OK "$(now)"
out="$(run --check --label exit-direct)"; rc=$?
[ $rc -eq 0 ] && ok "--check passes on a fresh install with a recent healthy run" || bad "fresh install not accepted: $out"

before="$(cat "$T/u/vpn-shorts-health@.service" "$T/u/vpn-shorts-health@.timer" | sha256sum)"
run --label exit-direct >/dev/null; rc=$?
after="$(cat "$T/u/vpn-shorts-health@.service" "$T/u/vpn-shorts-health@.timer" | sha256sum)"
{ [ $rc -eq 0 ] && [ "$before" = "$after" ]; } && ok "re-running the installer is idempotent (succeeds, units unchanged)" || bad "second install failed or changed the units"

# drift: someone edits the unit to run from a scratch checkout (the staging incident)
sed -i 's#/opt/singbox-vpn#/root/monitor-src/f4fec7a#' "$T/u/vpn-shorts-health@.service"
out="$(run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*differs from the shipped' <<<"$out"; } && ok "a unit edited to run from a scratch checkout is detected" || bad "unit drift missed: $out"
run --label exit-direct >/dev/null
cmp -s "$ROOT/deploy/lib/monitoring/vpn-shorts-health@.service" "$T/u/vpn-shorts-health@.service" && ok "re-install restores the shipped unit" || bad "drift not repaired"

# prerequisites
out="$(FAKE_PLAYWRIGHT_RC=1 run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*playwright' <<<"$out"; } && ok "a PROBE_PYTHON that cannot import playwright is a FAIL" || bad "missing playwright not detected: $out"
mkenv; rm "$T/e/env"; out="$(run --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && [ ! -e "$T/u/vpn-shorts-health@.timer" ]; } && ok "install refuses (and installs nothing) when the env file is missing" || bad "installed without prerequisites: rc=$rc"

# runtime health
mkenv; run --label exit-direct >/dev/null
state OK "$(( $(now) - 3 * 3600 ))"; out="$(run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*last run' <<<"$out"; } && ok "a stale last run (monitor silently stopped) is a FAIL" || bad "stale run not detected: $out"
rm -f "$T/fs/vpn-shorts-health@exit-direct.timer.active"; state OK "$(now)"; out="$(run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*timer.*not active' <<<"$out"; } && ok "an inactive timer is a FAIL" || bad "inactive timer not detected: $out"
touch "$T/fs/vpn-shorts-health@exit-direct.timer.active"
state UNVERIFIED "$(now)"; out="$(run --check --label exit-direct)"; rc=$?
{ [ $rc -ne 0 ] && grep -q 'FAIL.*blind' <<<"$out"; } && ok "UNVERIFIED (the probe cannot run) is reported as a blind monitor, never as healthy" || bad "blind monitor not flagged: $out"
state CRITICAL "$(now)"; out="$(run --check --label exit-direct)"; rc=$?
{ [ $rc -eq 0 ] && grep -qi 'INFO.*CRITICAL' <<<"$out"; } && ok "CRITICAL media health is reported (INFO) but is not a deployment failure" || bad "CRITICAL mishandled: rc=$rc $out"
[ "$fail" -eq 0 ] && echo "PASS: test-shorts-monitor-install.sh"
