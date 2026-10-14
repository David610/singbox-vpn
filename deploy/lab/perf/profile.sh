#!/usr/bin/env bash
# profile.sh reality|hy2 [SECONDS] -> `go tool pprof -top` text (class B, lab only)
#
# CPU profile of the exit's sing-box during one single-flow download through
# the given transport (run with netem off). Restarts the exit and both
# standard clients with a lab-only debug listener (127.0.0.1:6060 inside the
# exit namespace, never on a real server), then restarts them without it.
# Requires lab.sh standard_up and `go` on PATH.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
transport="${1:-}"; secs="${2:-8}"
case "$transport" in
  reality) port=15201; client=cR ;;
  hy2) port=15301; client=cH ;;
  *) echo "usage: $0 reality|hy2 [SECONDS]" >&2; exit 2 ;;
esac

restart_all() { # restart_all [SERVER_OVERRIDE_JSON]
  "$HERE/lab.sh" stop server-exit; "$HERE/lab.sh" stop cR; "$HERE/lab.sh" stop cH; sleep 0.5
  "$HERE/lab.sh" server_up exit exit 1 "$@"
  "$HERE/lab.sh" client_up cR Reality 1080 15201 4 2 >/dev/null
  "$HERE/lab.sh" client_up cH Hysteria2 1081 15301 4 2 >/dev/null
}
trap 'restart_all' EXIT

echo '{"experimental":{"debug":{"listen":"127.0.0.1:6060"}}}' >"$RUN/debug-override.json"
restart_all "$RUN/debug-override.json"
(CLIENT_PID_NAME="$client" "$HERE/tput.sh" "$transport" "$port" "$((secs + 4))" 1 1 >"$RUN/profile-tput.json") &
tput_pid=$!
sleep 2
ip netns exec exit curl -s --noproxy '*' -o "$RUN/profile-$transport.pb.gz" \
  "http://127.0.0.1:6060/debug/pprof/profile?seconds=$secs"
wait "$tput_pid"
echo "# transfer: $(cat "$RUN/profile-tput.json")"
go tool pprof -top -nodecount=30 "$RUN/profile-$transport.pb.gz" 2>&1
