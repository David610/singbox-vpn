#!/usr/bin/env bash
# Regression coverage for deploy/lib/check-glibc-baseline.sh — the release
# pipeline gate that fails an x86_64 release artifact whose GLIBC_*
# symbol requirement exceeds the declared baseline (v1.0.0-rc.3 incident:
# ubuntu-latest's own glibc 2.39 leaked into the shipped binaries).
#
# The version-comparison logic is exercised directly (sourcing the pure
# functions), never via objdump against real binaries built for
# different glibc versions — that would require multiple real glibc
# environments this test cannot assume. The functional end-to-end path
# (does it actually detect a real binary's real GLIBC_* requirement) is
# instead exercised against whatever binaries this very build produces
# (this host's own toolchain) using a deliberately permissive baseline,
# which proves the ELF-parsing plumbing (objdump/readelf invocation,
# output parsing) works without needing a controlled glibc mismatch.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CHECK_SH="$REPO_ROOT/deploy/lib/check-glibc-baseline.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# shellcheck disable=SC1090
. "$CHECK_SH"

echo "--- functional: glibc_version_le() compares numerically, never lexicographically ---"
# The exact case the spec calls out: lexicographically "2.9" > "2.28"
# (the character '9' sorts after '2'), but numerically 2.9 < 2.28. A
# broken string/glob comparison would get this backwards.
if glibc_version_le "2.9" "2.28"; then
  ok "2.9 <= 2.28 (numeric compare, not lexicographic)"
else
  fail "2.9 was incorrectly judged > 2.28 — comparison is lexicographic, not numeric"
fi
if glibc_version_le "2.28" "2.28"; then
  ok "2.28 <= 2.28 (boundary is inclusive)"
else
  fail "2.28 <= 2.28 incorrectly failed — baseline boundary must be inclusive"
fi
if ! glibc_version_le "2.29" "2.28"; then
  ok "2.29 > 2.28 correctly rejected"
else
  fail "2.29 was incorrectly judged <= 2.28"
fi
if ! glibc_version_le "3.2" "2.28"; then
  ok "3.2 > 2.28 correctly rejected (major-version comparison)"
else
  fail "3.2 was incorrectly judged <= 2.28"
fi
if glibc_version_le "1.9" "2.0"; then
  ok "1.9 <= 2.0 correctly accepted (lower major always wins regardless of minor)"
else
  fail "1.9 was incorrectly judged > 2.0"
fi

echo
echo "--- functional: glibc_max_version() finds the numeric maximum, not the lexicographic/last one ---"
max="$(glibc_max_version "2.2.5" "2.17" "2.9" "2.28" "2.3")"
[ "$max" = "2.28" ] && ok "max of (2.2.5 2.17 2.9 2.28 2.3) == 2.28" || fail "expected max 2.28, got '$max'"
max2="$(glibc_max_version "2.39" "2.9" "2.5")"
[ "$max2" = "2.39" ] && ok "max of (2.39 2.9 2.5) == 2.39" || fail "expected max 2.39, got '$max2'"

echo
echo "--- functional: is_valid_glibc_version() accepts well-formed versions and rejects garbage ---"
for good in "2.28" "2.9" "10.5" "2.3.4"; do
  is_valid_glibc_version "$good" && ok "accepts '$good'" || fail "wrongly rejected '$good'"
done
for bad in "" "abc" "2" "2." ".28" "2.28.x"; do
  if ! is_valid_glibc_version "$bad"; then
    ok "rejects '$bad'"
  else
    fail "wrongly accepted '$bad'"
  fi
done

echo
echo "--- functional: check_one_binary() against this host's own toolchain output (permissive baseline passes) ---"
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
cat > "$TMPDIR_TEST/hello.c" <<'EOF'
int main(void) { return 0; }
EOF
if command -v cc >/dev/null 2>&1 && cc "$TMPDIR_TEST/hello.c" -o "$TMPDIR_TEST/hello" 2>/dev/null; then
  if check_one_binary "$TMPDIR_TEST/hello" "99.0" >/tmp/glibc-check-pass.out 2>&1; then
    ok "a real compiled binary passes against a deliberately permissive baseline (99.0)"
  else
    fail "a real compiled binary unexpectedly failed against baseline 99.0 (see /tmp/glibc-check-pass.out)"
  fi
  if ! check_one_binary "$TMPDIR_TEST/hello" "0.1" >/tmp/glibc-check-fail.out 2>&1; then
    ok "the same binary correctly fails against an impossibly low baseline (0.1)"
    grep -q '::error::' /tmp/glibc-check-fail.out && ok "failure output includes an ::error:: diagnostic line" || fail "failure output missing ::error:: line"
  else
    fail "the same binary unexpectedly PASSED against baseline 0.1 — comparison logic is broken"
  fi
else
  echo "SKIP: no C compiler available in this environment — functional ELF-parsing check against a real binary skipped (comparison-logic unit tests above still ran and are authoritative for the version-aware-comparison requirement)"
fi

echo
echo "--- functional: check_one_binary() fails closed on a missing file ---"
if ! check_one_binary "$TMPDIR_TEST/does-not-exist" "2.28" >/tmp/glibc-check-missing.out 2>&1; then
  ok "a missing binary path fails closed"
else
  fail "a missing binary path unexpectedly passed"
fi

echo
echo "--- static: the CLI entry point (main()) requires at least a baseline and one binary argument ---"
if ! bash "$CHECK_SH" 2>/tmp/glibc-check-usage.out; then
  ok "running with no arguments fails closed"
  grep -qi usage /tmp/glibc-check-usage.out && ok "prints a usage message" || fail "missing usage message"
else
  fail "running with no arguments unexpectedly succeeded"
fi
if ! bash "$CHECK_SH" "not-a-version" "$TMPDIR_TEST/hello" 2>/tmp/glibc-check-badversion.out; then
  ok "an invalid baseline version string is rejected"
else
  fail "an invalid baseline version string was unexpectedly accepted"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all GLIBC baseline check tests passed"
