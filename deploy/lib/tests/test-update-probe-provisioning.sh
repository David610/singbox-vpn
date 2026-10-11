#!/usr/bin/env bash
# Regression test: update.sh must provision the reserved probe principal
# BEFORE each post-update L5-6 protocol self-test, refuse the update if it
# cannot, and do so WITHOUT waiting on the transaction lock update.sh holds.
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
# The first fix (v1.2.0-rc.2) called `vpn user create-probe` plainly and
# DEADLOCKED a real host: update.sh holds the global lock
# (/run/lock/singbox-vpn.lock) for the whole transaction, which is why its other
# vpn calls run with SINGBOX_VPN_LOCK_PATH pointing at a private inner lock; a
# plain call waits forever on the lock its own parent holds. This test therefore
# also EXECUTES each real create-probe stanza against a mock vpn that refuses to
# wait on a lock that is already held.
#
# update.sh runs at top level and cannot be sourced, so the stanzas are checked
# structurally and extracted verbatim from the real file.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="${UPDATE_SH_UNDER_TEST:-$REPO_ROOT/deploy/almalinux/update.sh}"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

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
    fail "update.sh:$nearest 'user create-probe' is $gap lines from the self-test at :$d - too far to be its precondition"
    continue
  fi
  # The provisioning command and its die-on-failure guard sit on adjacent lines.
  guard="$(sed -n "${nearest},$((nearest + 1))p" "$UPDATE_SH")"
  if grep -q '|| die' <<<"$guard"; then
    ok "update.sh:$d protocol self-test is preceded by a create-probe that dies on failure (:$nearest)"
  else
    fail "update.sh:$nearest 'user create-probe' does not die when provisioning fails"
  fi
  # update.sh holds the global lock here: the call must use the inner lock path.
  env_line="$(sed -n "$((nearest - 1))p" "$UPDATE_SH")"
  if grep -q 'SINGBOX_VPN_LOCK_PATH="\$BACKUP_DIR/update-inner.lock"' <<<"$env_line"; then
    ok "update.sh:$nearest create-probe runs under SINGBOX_VPN_LOCK_PATH (the inner lock), not the global lock"
  else
    fail "update.sh:$nearest create-probe lacks SINGBOX_VPN_LOCK_PATH=\$BACKUP_DIR/update-inner.lock and would wait forever on update.sh's own lock"
  fi
done

echo
echo "--- functional: each real create-probe stanza must not wait on the lock update.sh holds ---"
mkdir -p "$work/bin" "$work/backup"
# Mock vpn: takes the lock named by SINGBOX_VPN_LOCK_PATH (or the global default)
# with an atomic mkdir, and fails FAST (exit 99) if that lock is already held,
# which is what a real vpn would wait on forever.
cat >"$work/bin/vpn" <<'EOF'
#!/usr/bin/env bash
lock="${SINGBOX_VPN_LOCK_PATH:-$MOCK_GLOBAL_LOCK}"
if ! mkdir "$lock.held" 2>/dev/null; then
  echo "WAITED-ON-HELD-LOCK: $lock" >&2
  exit 99
fi
rmdir "$lock.held"
exit 0
EOF
chmod +x "$work/bin/vpn"
# update.sh holds the global transaction lock for the whole update.
mkdir "$work/global.lock.held"

n=0
for p in "${probe_lines[@]}"; do
  n=$((n + 1))
  {
    echo 'die() { echo "DIE: $*" >&2; exit 1; }'
    sed -n "$((p - 1)),$((p + 1))p" "$UPDATE_SH"
    echo 'echo STANZA-OK'
  } >"$work/stanza.$n.sh"
  out="$(BIN_DIR="$work/bin" DEPLOYMENT_TOML=/dev/null BACKUP_DIR="$work/backup" \
    MOCK_GLOBAL_LOCK="$work/global.lock" timeout 20 bash "$work/stanza.$n.sh" 2>&1)" && rc=0 || rc=$?
  if [ "$rc" -eq 0 ] && grep -q 'STANZA-OK' <<<"$out"; then
    ok "create-probe stanza #$n (update.sh:$p) completes while the global lock is held"
  else
    fail "create-probe stanza #$n (update.sh:$p) did not complete (rc=$rc): $out"
  fi
done

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "PASS: update acceptance provisions the probe principal before the protocol self-test, without waiting on its own lock"
