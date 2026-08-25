#!/usr/bin/env bash
# Regression test for deploy/lib/check-no-legacy-identity.sh itself: proves
# (a) it catches the forbidden obsolete identifier in an arbitrary tracked
# fixture file, and (b) it catches the identifier if planted inside the
# guard script's OWN tracked content — i.e. there is no self-exclusion
# blind spot (see docs/IMPLEMENTATION_STATUS.md's clean-break checkpoint).
#
# Runs the guard against a disposable, temporary git sandbox rather than
# this real repository, and never commits the forbidden literal to any
# file tracked by this repository — the literal only ever exists inside a
# throwaway git repo under $TMPDIR that is deleted on exit.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
GUARD_SRC="$REPO_ROOT/deploy/lib/check-no-legacy-identity.sh"

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT

# Built from parts, exactly like the guard itself — never written out as
# one literal string anywhere in this test file.
P1="vpn"
P2="1"
FORBIDDEN="${P1}${P2}"

SANDBOX="$TEST_TMP/sandbox"
mkdir -p "$SANDBOX/deploy/lib"
git -C "$SANDBOX" init -q
git -C "$SANDBOX" config user.email "test@example.com"
git -C "$SANDBOX" config user.name "test"
cp "$GUARD_SRC" "$SANDBOX/deploy/lib/check-no-legacy-identity.sh"

echo "hello world" > "$SANDBOX/README.md"
git -C "$SANDBOX" add -A
git -C "$SANDBOX" commit -q -m init

# --- case 1: a clean sandbox tree passes -------------------------------
if ! (cd "$SANDBOX" && bash deploy/lib/check-no-legacy-identity.sh) \
    >"$TEST_TMP/clean.log" 2>&1; then
  cat "$TEST_TMP/clean.log" >&2
  echo "FAIL: guard failed on a clean sandbox tree" >&2
  exit 1
fi

# --- case 2: the literal in an arbitrary tracked fixture is caught -----
printf '%s was here\n' "$FORBIDDEN" > "$SANDBOX/fixture.txt"
git -C "$SANDBOX" add -A
git -C "$SANDBOX" commit -q -m "add fixture"
rc=0
(cd "$SANDBOX" && bash deploy/lib/check-no-legacy-identity.sh) \
  >"$TEST_TMP/fixture.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: guard did not catch the forbidden literal in fixture.txt" >&2; exit 1; }
grep -q "fixture.txt" "$TEST_TMP/fixture.log" \
  || { echo "FAIL: guard failure output did not name fixture.txt" >&2; cat "$TEST_TMP/fixture.log" >&2; exit 1; }
git -C "$SANDBOX" rm -q fixture.txt
git -C "$SANDBOX" commit -q -m "remove fixture"

# --- case 3: the literal planted INSIDE the guard script's own tracked
# content is also caught — this is the regression Issue-C guards against:
# a self-exclusion would let a future edit paste the literal into this
# very file and have CI silently ignore it.
cp "$GUARD_SRC" "$SANDBOX/deploy/lib/check-no-legacy-identity.sh"
printf '\n# %s test marker\n' "$FORBIDDEN" >> "$SANDBOX/deploy/lib/check-no-legacy-identity.sh"
git -C "$SANDBOX" add -A
git -C "$SANDBOX" commit -q -m "plant literal in guard script"
rc=0
(cd "$SANDBOX" && bash deploy/lib/check-no-legacy-identity.sh) \
  >"$TEST_TMP/self.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: guard did not catch the forbidden literal planted inside itself (self-exclusion regression)" >&2; exit 1; }
grep -q "check-no-legacy-identity.sh" "$TEST_TMP/self.log" \
  || { echo "FAIL: guard failure output did not name itself" >&2; cat "$TEST_TMP/self.log" >&2; exit 1; }

echo "no-legacy-identity guard self-scan: PASS"
