#!/usr/bin/env bash
# Soak tooling: the evaluator's fixed criteria (synthetic runs) and the
# sampler's read-only, secret-free CSV output (fake systemctl and /proc).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

python3 "$ROOT/deploy/lib/tests/test-soak-evaluate.py" >"$T/py.log" 2>&1 \
  && ok "evaluator criteria: $(tail -3 "$T/py.log" | tr '\n' ' ')" \
  || { fail "evaluator tests failed"; cat "$T/py.log" >&2; }

# ---- the sampler against a fake host --------------------------------------------
mkdir -p "$T/bin" "$T/proc/4242/fd"
touch "$T/proc/4242/fd/0" "$T/proc/4242/fd/1" "$T/proc/4242/fd/2"
printf 'Name:\tsing-box\nVmRSS:\t   61234 kB\n' >"$T/proc/4242/status"
cat >"$T/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  is-active) exit 0 ;;
  show)
    case "$3" in
      MainPID) echo 4242 ;;
      NRestarts) echo 0 ;;
      Result) echo success ;;
      InactiveEnterTimestampMonotonic) echo "${FAKE_STAMP:-123456}" ;;
    esac ;;
esac
EOF
chmod +x "$T/bin/systemctl"
cat >"$T/bin/vpn-admin" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$T/bin/vpn-admin"

out="$T/soak.csv"
SOAK_SYSTEMCTL="$T/bin/systemctl" SOAK_VPN_ADMIN="$T/bin/vpn-admin" SOAK_PROC_ROOT="$T/proc" \
  bash "$ROOT/deploy/lib/soak-sampler.sh" --duration 2 --interval 1 --output "$out" --selftest-every 1 >/dev/null
rows="$(($(wc -l <"$out") - 1))"
[ "$rows" -ge 2 ] && ok "the sampler wrote $rows rows" || fail "too few rows: $rows"
head -1 "$out" | grep -q '^epoch,singbox_active,singbox_pid,singbox_restarts,singbox_rss_kb,singbox_fds,subscription_active,reconcile_result,reconcile_runs,watchdog_active,selftest$' \
  && ok "the CSV header is the one the evaluator expects" || fail "unexpected CSV header"
sed -n 2p "$out" | grep -Eq '^[0-9]+,1,4242,0,61234,3,1,success,1,1,pass$' \
  && ok "a row carries only counters and states (pid, restarts, rss, fds, activity, selftest)" || fail "unexpected row: $(sed -n 2p "$out")"
# the file must hold nothing but those columns: no hostnames, users, tokens or addresses
grep -Eiq '[a-z]{3,}\.[a-z]{2,}|uuid|token|password|vless|hysteria' <(tail -n +2 "$out") \
  && fail "the sample file contains identifying text" || ok "no identifying text in the samples"
stat_mode="$(stat -c %a "$out")"
[ "$stat_mode" = 600 ] && ok "the sample file is private (0600)" || fail "sample file mode is $stat_mode"
if SOAK_SYSTEMCTL="$T/bin/systemctl" bash "$ROOT/deploy/lib/soak-sampler.sh" --duration 1 --interval 1 --output "$out" >/dev/null 2>&1; then
  fail "the sampler overwrote an existing result file"
else
  ok "the sampler refuses to overwrite an existing result file"
fi
python3 "$ROOT/deploy/lib/soak_evaluate.py" "$out" >/dev/null 2>&1 \
  && fail "a 2-second sample passed the 90-minute criteria" || ok "a short sample FAILS the fixed criteria (no duration shortcut)"

[ "$failures" -eq 0 ] && { echo "PASS: test-soak-evaluation.sh"; exit 0; }
echo "FAIL: test-soak-evaluation.sh ($failures failure(s))" >&2
exit 1
