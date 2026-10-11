#!/usr/bin/env bash
# Regression test: update.sh must provision the reserved probe principal
# BEFORE each post-update L5-6 protocol self-test, and refuse the update if it
# cannot.
#
# Since C-16 (egress isolation) the exit's own loopback listener is reachable
# only by a RESERVED PROBE principal (`vpn-admin user create-probe`). Without
# one the protocol self-test falls back to an ordinary user, is rejected by
# design, comes back INCONCLUSIVE, and `doctor --protocol --require-protocol`
# rolls the update back. install.sh already calls `user create-probe` before its
# acceptance (test-acceptance-probe-provisioning.sh); update.sh did not, so a
# host updating from a release that predates the principal (stable v1.1.0)
# rolled back every attempt to v1.2.0-rc.1 on a real VPS.
#
# update.sh runs at top level and cannot be sourced, so this checks its
# structure: every real (non-comment) protocol self-test call is preceded, within
# a few lines, by a `user create-probe` that dies on failure, and the counts match
# so no call site is left uncovered.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

mapfile -t doctor_lines < <(grep -n -E '^[^#]*doctor --protocol --require-protocol' "$UPDATE_SH" | cut -d: -f1)
mapfile -t probe_lines < <(grep -n -E '^[[:space:]]*"\$BIN_DIR/vpn" --config "\$DEPLOYMENT_TOML" user create-probe' "$UPDATE_SH" | cut -d: -f1)

echo "--- static: update.sh protocol self-test call sites and probe provisioning ---"
if [ "${#doctor_lines[@]}" -ge 2 ]; then
  ok "update.sh has ${#doctor_lines[@]} protocol self-test call sites (release path and dev-rebuild path)"
else
  fail "expected at least 2 protocol self-test call sites in update.sh, found ${#doctor_lines[@]}"
fi

if [ "${#probe_lines[@]}" -eq "${#doctor_lines[@]}" ]; then
  ok "every protocol self-test call site has exactly one 'user create-probe' (${#probe_lines[@]} each)"
else
  fail "update.sh has ${#probe_lines[@]} 'user create-probe' calls for ${#doctor_lines[@]} protocol self-test calls"
fi

for d in "${doctor_lines[@]}"; do
  nearest=""
  for p in "${probe_lines[@]}"; do
    if [ "$p" -lt "$d" ]; then nearest="$p"; fi
  done
  if [ -z "$nearest" ]; then
    fail "protocol self-test at update.sh:$d has no 'user create-probe' before it"
    continue
  fi
  gap=$((d - nearest))
  if [ "$gap" -gt 12 ]; then
    fail "update.sh:$nearest 'user create-probe' is $gap lines from the self-test at :$d — too far to be its precondition"
    continue
  fi
  # The provisioning command and its die-on-failure guard sit on adjacent lines.
  guard="$(sed -n "${nearest},$((nearest + 1))p" "$UPDATE_SH")"
  if grep -q '|| die' <<<"$guard"; then
    ok "update.sh:$d protocol self-test is preceded by a create-probe that dies on failure (:$nearest)"
  else
    fail "update.sh:$nearest 'user create-probe' does not die when provisioning fails"
  fi
done

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "PASS: update acceptance provisions the probe principal before the protocol self-test"
