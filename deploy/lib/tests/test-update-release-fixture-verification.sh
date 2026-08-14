#!/usr/bin/env bash
# Checkpoint 5 fixture test for update.sh's release-source checksum
# verification. update.sh's download_verified_source_release() hardcodes
# its download URL to https://github.com/$VPN1_REPO/releases/... (no
# base-URL override exists — adding one is out of scope for this
# checkpoint, a "harness repair", not an update.sh network redesign), so
# it cannot be called network-free against a local fixture directly.
#
# Following the same precedent as test-release-archive-contract.sh, this
# extracts the REAL verification logic (the grep/sha256sum -c block) out
# of update.sh via sed and exercises it — unmodified — against two
# distinct local fixture "releases" with immutable version identifiers,
# separate content, and real sha256sum-computed checksums:
#   - VERIFIED-TEST: the extracted checksum-verification contract itself
#     (this file).
#   - Still UNVERIFIED: a real GitHub release A->B transition driven over
#     SSH by lifecycle-acceptance.sh, until a first tagged release exists
#     (see docs/IMPLEMENTATION_STATUS.md).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

[ -f "$UPDATE_SH" ] || fail "update.sh not found at $UPDATE_SH"

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# Extract ONLY the checksum-verification lines of
# download_verified_source_release() (from the well-formed-entry grep
# through the sha256sum -c call, stopping before its closing "log"
# success line and the function's closing brace) — the two curl downloads
# above them are deliberately excluded since there is no real GitHub
# release to fetch here; $sums/$tarball are populated directly from the
# local fixture below instead. This still exercises the REAL
# verification logic, unmodified, not a reimplementation of it.
BODY_FILE="$TMPDIR_TEST/verify-body.sh"
sed -n '/^  grep -qE .\^\[0-9a-f\]{64}/,/checksum verification failed for vpn1-src/p' "$UPDATE_SH" > "$BODY_FILE"
[ -s "$BODY_FILE" ] || fail "could not extract the checksum-verification lines from download_verified_source_release() in update.sh (function changed shape?)"
die() { echo "DIE: $*" >&2; return 1; }
export -f die

if grep -q 'grep -qE' "$BODY_FILE" && grep -q 'sha256sum -c' "$BODY_FILE"; then
  ok "extracted the real well-formed-check + sha256sum -c verification lines"
else
  fail "extracted function body is missing the expected checksum-verification lines"
fi

# Build two distinct fixture "releases" with immutable version identifiers
# and separate content — mirroring the real release layout (vpn1-src.tar.gz
# + SHA256SUMS) closely enough to exercise the same verification contract.
build_fixture() {
  local dir="$1" content="$2"
  mkdir -p "$dir"
  mkdir -p "$dir/vpn1-src-payload"
  echo "$content" > "$dir/vpn1-src-payload/marker.txt"
  ( cd "$dir" && tar -czf vpn1-src.tar.gz vpn1-src-payload )
  ( cd "$dir" && sha256sum vpn1-src.tar.gz > SHA256SUMS )
}

FIXTURE_A="$TMPDIR_TEST/release-vfixtureA"
FIXTURE_B="$TMPDIR_TEST/release-vfixtureB"
build_fixture "$FIXTURE_A" "vpn1 fixture release A payload"
build_fixture "$FIXTURE_B" "vpn1 fixture release B payload — different content"

if ! cmp -s "$FIXTURE_A/vpn1-src.tar.gz" "$FIXTURE_B/vpn1-src.tar.gz"; then
  ok "fixture A and fixture B have separate content (not the same archive twice)"
else
  fail "fixture A and B archives are identical — this would not be a real A->B test"
fi
if [ "$(cat "$FIXTURE_A/SHA256SUMS")" != "$(cat "$FIXTURE_B/SHA256SUMS")" ]; then
  ok "fixture A and B have distinct real sha256 checksums"
else
  fail "fixture A and B checksums are identical"
fi

run_verification_against_fixture() {
  local dir="$1" version="${2:-vfixture-test}"
  (
    set -Eeuo pipefail
    cd "$dir"
    # These are consumed by the sourced $BODY_FILE fragment (extracted
    # straight from update.sh), not by this test script directly.
    # shellcheck disable=SC2034
    STAGING_ROOT="$dir"
    # shellcheck disable=SC2034
    tarball="$dir/vpn1-src.tar.gz"
    # shellcheck disable=SC2034
    sums="$dir/SHA256SUMS"
    # shellcheck disable=SC2034
    version="$version"
    # shellcheck disable=SC1090
    source "$BODY_FILE"
  ) 2>&1
}

echo "--- VERIFIED-TEST: real checksum-verification logic accepts a valid fixture release ---"
if out="$(run_verification_against_fixture "$FIXTURE_A" 2>&1)"; then
  ok "valid fixture A passes the real verification block"
else
  fail "valid fixture A was rejected by the real verification block: $out"
fi
if out="$(run_verification_against_fixture "$FIXTURE_B" 2>&1)"; then
  ok "valid fixture B (distinct version/content) passes the real verification block"
else
  fail "valid fixture B was rejected by the real verification block: $out"
fi

echo
echo "--- VERIFIED-TEST: real checksum-verification logic rejects a tampered archive ---"
TAMPERED="$TMPDIR_TEST/release-tampered"
mkdir -p "$TAMPERED"
cp "$FIXTURE_A/vpn1-src.tar.gz" "$TAMPERED/"
cp "$FIXTURE_A/SHA256SUMS" "$TAMPERED/"
echo "tampered-after-checksum" >> "$TAMPERED/vpn1-src.tar.gz"
rc=0
out="$(run_verification_against_fixture "$TAMPERED" 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && echo "$out" | grep -qi 'checksum verification failed'; then
  ok "tampered archive is rejected by the real verification block (fails closed)"
else
  fail "tampered archive was NOT rejected (rc=$rc out=$out) — checksum verification is not fail-closed"
fi

echo
echo "--- VERIFIED-TEST: real checksum-verification logic rejects a malformed SHA256SUMS ---"
MALFORMED="$TMPDIR_TEST/release-malformed"
mkdir -p "$MALFORMED"
cp "$FIXTURE_A/vpn1-src.tar.gz" "$MALFORMED/"
echo "not-a-valid-checksum-line" > "$MALFORMED/SHA256SUMS"
rc=0
out="$(run_verification_against_fixture "$MALFORMED" 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && echo "$out" | grep -qi 'no well-formed entry'; then
  ok "malformed SHA256SUMS is rejected by the real verification block (fails closed)"
else
  fail "malformed SHA256SUMS was NOT rejected (rc=$rc out=$out)"
fi

echo
echo "--- static: update.sh has no in-process test/mock shim in its production path ---"
if grep -qiE 'if.*\bTEST\b.*then|VPN1_TEST_MODE|VPN1_MOCK' "$UPDATE_SH"; then
  fail "update.sh appears to contain test/mock branching in its production code path"
else
  ok "update.sh's production download/verify path has no test-mode branching (this fixture test lives entirely outside it)"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "PASS: test-update-release-fixture-verification.sh"
  exit 0
else
  echo "FAIL: test-update-release-fixture-verification.sh ($failures failure(s))"
  exit 1
fi
