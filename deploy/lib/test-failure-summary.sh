#!/usr/bin/env bash
# Reason lines for a failed shell test (used by run-shell-tests-clean-host.sh). Always prints something: matching lines
# (failure keywords AND the usual "missing prerequisite" phrasings), else the last lines of the output, else an explicit
# "no output". Bounded to 8 lines. Pure: takes the captured output as $1.
test_failure_reason() {
  local out="$1" hits
  [ -n "$out" ] || { echo "(the test failed and printed no output)"; return 0; }
  hits="$(grep -E 'FAIL|UNEXPECTED|rror|[Rr]equired|[Nn]o module|[Nn]ot found|[Nn]o such|Traceback|[Pp]ermission denied' <<<"$out" | tail -8 || true)"
  if [ -n "$hits" ]; then printf '%s\n' "$hits"; else tail -5 <<<"$out"; fi
}
