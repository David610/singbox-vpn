#!/usr/bin/env bash
# Regression (found by the SIGKILL matrix): update/rollback/recovery restart vpn-subscription and sing-box
# several times within seconds; systemd's start-rate limiter (StartLimitBurst) then refuses further starts
# ("Start request repeated too quickly") and the rollback's health verification fails. Every explicit restart in
# update.sh must clear the limiter first (restart_unit / reload_or_restart_unit).
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
U="$REPO_ROOT/deploy/almalinux/update.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
raw="$(grep -nE '^\s*(systemctl (restart|reload-or-restart|start) (vpn-subscription|sing-box|vpn-egress-isolation))' "$U" | grep -vE '^[0-9]+:(restart_unit|reload_or_restart_unit)\(\)' || true)"
raw="$(grep -vE 'systemctl start vpn-egress-isolation.service >/dev/null' <<<"$raw" || true)"
[ -z "$(tr -d '[:space:]' <<<"$raw")" ] && ok "no raw 'systemctl restart|reload-or-restart' of the VPN units remains in update.sh" || bad "raw restart(s) bypass the limiter reset: $raw"
grep -q '^restart_unit() { systemctl reset-failed' "$U" && grep -q '^reload_or_restart_unit() { systemctl reset-failed' "$U" && ok "helpers reset-failed before restarting" || bad "helpers missing reset-failed"
n="$(grep -cE '^\s*(restart_unit|reload_or_restart_unit) ' "$U" || true)"
[ "$n" -ge 9 ] && ok "$n restart sites use the helpers (rollback, recovery, update, repair)" || bad "only $n helper call sites"
# rollback and recovery specifically
body="$(sed -n '/^perform_rollback_from_backup() {/,/^}/p' "$U")"
grep -q 'restart_unit vpn-subscription' <<<"$body" && grep -q 'reload_or_restart_unit sing-box' <<<"$body" && ok "the shared rollback restarts through the helpers" || bad "rollback bypasses the helpers"
# behavioural: with a systemctl stub that refuses starts until reset-failed ran, the helper succeeds and a raw restart would not
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
cat >"$T/systemctl" <<EOF
#!/bin/sh
case "\$1" in
  reset-failed) rm -f "$T/limited"; exit 0 ;;
  restart|reload-or-restart) [ -e "$T/limited" ] && { echo "Start request repeated too quickly" >&2; exit 1; }; exit 0 ;;
esac
EOF
chmod +x "$T/systemctl"
eval "$(sed -n '/^restart_unit() {/p;/^reload_or_restart_unit() {/p' "$U")"
touch "$T/limited"; PATH="$T:$PATH" systemctl restart vpn-subscription 2>/dev/null && bad "stub did not model the limiter" || ok "model: a raw restart is refused while the limiter is tripped"
touch "$T/limited"; PATH="$T:$PATH" restart_unit vpn-subscription && ok "restart_unit succeeds while the limiter is tripped" || bad "restart_unit refused"
touch "$T/limited"; PATH="$T:$PATH" reload_or_restart_unit sing-box && ok "reload_or_restart_unit succeeds while the limiter is tripped" || bad "reload_or_restart_unit refused"
[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
