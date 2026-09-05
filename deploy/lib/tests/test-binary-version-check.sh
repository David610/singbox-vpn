#!/usr/bin/env bash
# Regression coverage for deploy/lib/binary-version-check.sh's
# check_binary_version() — the shared classifier both install.sh's
# fetch_release_binaries() and update.sh's staged-binary check now use.
#
# Real-world incident this fixes (v1.0.0-rc.3): a binary that could not
# execute at all (GLIBC_2.39 not found on an AlmaLinux 8 host) was
# reported as "authenticated binary archive reports version ''",
# indistinguishable from a genuine packaging/version bug. Since a real
# cross-glibc-incompatible binary isn't available in every test
# environment, "cannot execute" is emulated with fixture scripts —
# exactly as the task's regression-test guidance suggests — covering
# all 4 states end to end.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CHECK_SH="$REPO_ROOT/deploy/lib/binary-version-check.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
OUT="$TMPDIR_TEST/out"

# check_binary_version() expects the caller to already define die() (same
# convention as ownership.sh/preflight.sh). Real die() prints and exits
# immediately, so subsequent lines in check_binary_version never run —
# reproduce that exactly in a subshell (a stub that only sets a flag
# without exiting would let execution fall through to the NEXT check
# inside check_binary_version and overwrite the real classification,
# which is not how the production code behaves). Output goes to $OUT and
# the subshell's own exit status is returned via `|| rc=$?` at each call
# site, never through a `set -e`-tripping command substitution.
run_check() {
  local fixture="$1" expected="$2" label="$3" context="$4"
  (
    die() { echo "DIE:$*"; exit 1; }
    # shellcheck disable=SC1090
    . "$CHECK_SH"
    check_binary_version "$fixture" "$expected" "$label" "$context"
    echo "NO_DIE"
  ) >"$OUT" 2>&1
}

echo "--- STATE 4: correct version -> success, die() never called ---"
cat > "$TMPDIR_TEST/ok-binary" <<'EOF'
#!/bin/sh
echo "vpn-admin 1.0.0"
EOF
chmod +x "$TMPDIR_TEST/ok-binary"
rc=0; run_check "$TMPDIR_TEST/ok-binary" "1.0.0" "vpn-admin" "ctx" || rc=$?
if [ "$rc" -eq 0 ] && grep -q "NO_DIE" "$OUT"; then
  ok "correct version: die() not called, execution reaches past the check"
else
  fail "correct version incorrectly triggered die(): rc=$rc out='$(cat "$OUT")'"
fi

echo
echo "--- STATE 3: wrong version -> die() with 'reports version X, expected Y' (fail closed) ---"
cat > "$TMPDIR_TEST/wrong-version" <<'EOF'
#!/bin/sh
echo "vpn-admin 0.9.9"
EOF
chmod +x "$TMPDIR_TEST/wrong-version"
rc=0; run_check "$TMPDIR_TEST/wrong-version" "1.0.0" "vpn-admin" "ctx" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "reports version '0.9.9', expected '1.0.0'" "$OUT"; then
  ok "wrong version correctly fails closed with the exact version mismatch reported"
else
  fail "wrong version did not produce the expected die() message: rc=$rc out='$(cat "$OUT")'"
fi

echo
echo "--- STATE 2: executes successfully but empty output -> 'no parseable version' ---"
cat > "$TMPDIR_TEST/empty-output" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$TMPDIR_TEST/empty-output"
rc=0; run_check "$TMPDIR_TEST/empty-output" "1.0.0" "vpn-admin" "ctx" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "no parseable version" "$OUT"; then
  ok "empty output is classified as 'no parseable version', not a version mismatch"
else
  fail "empty output did not produce the expected classification: rc=$rc out='$(cat "$OUT")'"
fi

echo
echo "--- STATE 2 (malformed): whitespace-only output is still classified as no-version, not a version mismatch ---"
cat > "$TMPDIR_TEST/malformed-output" <<'EOF'
#!/bin/sh
printf '   \n'
exit 0
EOF
chmod +x "$TMPDIR_TEST/malformed-output"
rc=0; run_check "$TMPDIR_TEST/malformed-output" "1.0.0" "vpn-admin" "ctx" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "no parseable version" "$OUT"; then
  ok "whitespace-only output is classified as 'no parseable version'"
else
  fail "whitespace-only output did not produce the expected classification: rc=$rc out='$(cat "$OUT")'"
fi

echo
echo "--- STATE 1: exits non-zero with stderr -> 'cannot execute', with a bounded stderr excerpt (the exact rc.3 regression) ---"
cat > "$TMPDIR_TEST/cannot-execute" <<'EOF'
#!/bin/sh
echo "vpn-admin: /lib64/libc.so.6: version 'GLIBC_2.39' not found (required by vpn-admin)" >&2
exit 1
EOF
chmod +x "$TMPDIR_TEST/cannot-execute"
rc=0; run_check "$TMPDIR_TEST/cannot-execute" "1.0.0" "vpn-admin" "ctx" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "cannot execute on this host" "$OUT" && grep -q "GLIBC_2.39" "$OUT"; then
  ok "an exec failure is classified as 'cannot execute' (never as an empty-string version) and the real stderr diagnostic (GLIBC_2.39 not found) is surfaced, not swallowed"
else
  fail "exec failure was not correctly classified/surfaced: rc=$rc out='$(cat "$OUT")'"
fi
if ! grep -qE "reports version ''|reports version \"\"" "$OUT"; then
  ok "the old, misleading empty-string-version phrasing is gone for this failure mode"
else
  fail "regression: exec failure is still reported as an empty-string version, the exact rc.3 bug"
fi

echo
echo "--- STATE 1 (no stderr): exits non-zero with no stderr output at all -> still classified as 'cannot execute', not a version mismatch ---"
cat > "$TMPDIR_TEST/cannot-execute-silent" <<'EOF'
#!/bin/sh
exit 126
EOF
chmod +x "$TMPDIR_TEST/cannot-execute-silent"
rc=0; run_check "$TMPDIR_TEST/cannot-execute-silent" "1.0.0" "vpn-admin" "ctx" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "cannot execute on this host (exit 126, no stderr output)" "$OUT"; then
  ok "a silent non-zero exit is still classified as 'cannot execute', with exit code reported"
else
  fail "silent non-zero exit was not correctly classified: rc=$rc out='$(cat "$OUT")'"
fi

echo
echo "--- STATE 1: stderr excerpt is bounded (never unbounded output into logs) ---"
cat > "$TMPDIR_TEST/huge-stderr" <<'EOF'
#!/bin/sh
head -c 100000 /dev/zero | tr '\0' 'x' >&2
exit 1
EOF
chmod +x "$TMPDIR_TEST/huge-stderr"
rc=0; run_check "$TMPDIR_TEST/huge-stderr" "1.0.0" "vpn-admin" "ctx" || rc=$?
out_len="$(wc -c < "$OUT")"
if [ "$rc" -ne 0 ] && [ "$out_len" -lt 2000 ]; then
  ok "a huge stderr stream is bounded before reaching the die() message (length=$out_len)"
else
  fail "stderr excerpt was not bounded: rc=$rc length=$out_len"
fi

echo
echo "--- context suffix is preserved in every failure message ---"
rc=0; run_check "$TMPDIR_TEST/wrong-version" "1.0.0" "vpn-admin" "for release v1.0.0-rc.4. Nothing live has been changed." || rc=$?
if [ "$rc" -ne 0 ] && grep -q "for release v1.0.0-rc.4. Nothing live has been changed." "$OUT"; then
  ok "caller-supplied context suffix is preserved in the die() message"
else
  fail "context suffix was dropped: rc=$rc out='$(cat "$OUT")'"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all binary-version-check tests passed"
