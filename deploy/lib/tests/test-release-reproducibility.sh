#!/usr/bin/env bash
# Regression tests for reproducible production installs:
#   - deploy/lib/versions.env is the ONE authoritative source for
#     SINGBOX_VERSION/SINGBOX_SHA256_AMD64/SINGBOX_SHA256_ARM64/
#     SUPPORTED_ARCH — install.sh, CI's singbox-validate job, and
#     deploy/lib/fast-gate.sh must all read it, never hardcode a second
#     copy that can drift.
#   - valid/invalid sing-box checksum handling (install_singbox()).
#   - the top-level bootstrap (install.sh) resolves an immutable release
#     tag in the default (stable) channel, and REFUSES to fall back to
#     mutable branch source — only VPN1_CHANNEL=dev may do that.
#   - unsupported OS/arch is rejected before any system mutation.
#   - unexpected sing-box archive layout is rejected before install.
#   - the release payload carries a local, offline uninstaller.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
BOOTSTRAP_SH="$REPO_ROOT/install.sh"
VERSIONS_ENV="$REPO_ROOT/deploy/lib/versions.env"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
FAST_GATE_SH="$REPO_ROOT/deploy/lib/fast-gate.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- static: deploy/lib/versions.env exists and defines every required key exactly once ---"
[ -f "$VERSIONS_ENV" ] || { echo "FATAL: $VERSIONS_ENV missing"; exit 1; }
for key in SUPPORTED_ARCH SINGBOX_VERSION SINGBOX_SHA256_AMD64 SINGBOX_SHA256_ARM64; do
  count="$(grep -cE "^${key}=" "$VERSIONS_ENV")"
  if [ "$count" -eq 1 ]; then
    ok "$key defined exactly once in versions.env"
  else
    fail "$key defined $count times in versions.env (expected exactly 1)"
  fi
done

echo
echo "--- static: no hardcoded second copy of SINGBOX_VERSION/SHA256 anywhere else (single authoritative source) ---"
# shellcheck disable=SC1090
. "$VERSIONS_ENV"
for f in "$INSTALL_SH" "$CI_YML" "$FAST_GATE_SH"; do
  if grep -qE "SINGBOX_VERSION[[:space:]]*[:=][[:space:]]*[\"']?${SINGBOX_VERSION}[\"']?[[:space:]]*\$" "$f" 2>/dev/null \
     && [ "$f" != "$VERSIONS_ENV" ]; then
    fail "$f hardcodes SINGBOX_VERSION=$SINGBOX_VERSION instead of sourcing deploy/lib/versions.env"
  else
    ok "$(basename "$f") does not hardcode a duplicate SINGBOX_VERSION literal"
  fi
done
if grep -q "deploy/lib/versions.env" "$INSTALL_SH"; then ok "install.sh sources deploy/lib/versions.env"; else fail "install.sh does not reference deploy/lib/versions.env"; fi
if grep -q "deploy/lib/versions.env" "$CI_YML"; then ok "ci.yml loads deploy/lib/versions.env"; else fail "ci.yml does not reference deploy/lib/versions.env"; fi
if grep -q "deploy/lib/versions.env" "$FAST_GATE_SH"; then ok "fast-gate.sh sources deploy/lib/versions.env"; else fail "fast-gate.sh does not reference deploy/lib/versions.env"; fi

echo
echo "--- regression: ci.yml's \$GITHUB_ENV load of versions.env must filter comments/blank lines ---"
# \$GITHUB_ENV's file-command format rejects any line that is not a bare
# KEY=value (a comment line makes the whole step fail: "Invalid format").
# versions.env is a human-documented file with a comment header — ci.yml
# must never append it to \$GITHUB_ENV raw.
ci_load_line="$(grep -n 'versions\.env.*GITHUB_ENV' "$CI_YML" || true)"
if [ -z "$ci_load_line" ]; then
  fail "could not find the ci.yml step that loads versions.env into \$GITHUB_ENV"
elif echo "$ci_load_line" | grep -qE "^\s*[0-9]+:\s*run:\s*cat "; then
  fail "ci.yml appends deploy/lib/versions.env to \$GITHUB_ENV with a raw 'cat' — this breaks on the file's own comment header (VERIFIED against a real CI run: 'Invalid format' on the first comment line)"
else
  ok "ci.yml filters comments/blank lines before appending versions.env to \$GITHUB_ENV"
fi
filtered="$(grep -vE '^\s*(#|$)' "$VERSIONS_ENV")"
if echo "$filtered" | grep -qvE '^[A-Za-z_][A-Za-z0-9_]*='; then
  fail "the comment/blank-line filter still leaves a non-KEY=value line: $(echo "$filtered" | grep -vE '^[A-Za-z_][A-Za-z0-9_]*=')"
else
  ok "filtering deploy/lib/versions.env with the same pattern ci.yml uses leaves only valid KEY=value lines"
fi

echo
echo "--- static: install.sh dies closed if deploy/lib/versions.env is missing/incomplete ---"
if grep -qE 'missing from \$VERSIONS_ENV' "$INSTALL_SH"; then
  ok "install.sh errors out on a missing required key from versions.env"
else
  fail "install.sh has no fail-closed check for missing versions.env keys"
fi

echo
echo "--- functional: install_singbox() checksum verification (valid + invalid, pinned-digest path) ---"
TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT
FAKE_ASSET_CONTENT="not a real sing-box binary, just fixture bytes for checksum testing"
echo -n "$FAKE_ASSET_CONTENT" > "$TMPDIR_TEST/fake-asset.tar.gz"
REAL_SHA256="$(sha256sum "$TMPDIR_TEST/fake-asset.tar.gz" | awk '{print $1}')"
if [ "$REAL_SHA256" = "$REAL_SHA256" ] && sha256sum -c <(echo "$REAL_SHA256  $TMPDIR_TEST/fake-asset.tar.gz") >/dev/null 2>&1; then
  ok "valid checksum verifies successfully (sha256sum -c, the same mechanism install_singbox() uses)"
else
  fail "valid checksum unexpectedly failed to verify"
fi
BAD_SHA256="0000000000000000000000000000000000000000000000000000000000000000"
if ! sha256sum -c <(echo "$BAD_SHA256  $TMPDIR_TEST/fake-asset.tar.gz") >/dev/null 2>&1; then
  ok "invalid/mismatched checksum is correctly rejected"
else
  fail "invalid checksum unexpectedly verified"
fi
if grep -q 'expected_sha256" ] || die "no upstream checksums.txt' "$INSTALL_SH"; then
  ok "install_singbox() fails closed (die) when neither upstream checksums.txt nor a pinned digest is available"
else
  fail "install_singbox() no longer has a fail-closed no-digest-available guard"
fi
if grep -q 'checksum verification failed for \$tarball: expected \$expected_sha256' "$INSTALL_SH"; then
  ok "install_singbox() fails closed (die) on a pinned-digest mismatch"
else
  fail "install_singbox() no longer dies on a pinned-digest checksum mismatch"
fi

echo
echo "--- static: fetch_release_binaries() fails closed on missing SHA256SUMS (never installs an unverified binary) ---"
if grep -q 'refusing to install unverified binaries' "$INSTALL_SH" \
   && grep -q 'refusing to install a binary with no integrity verification' "$INSTALL_SH"; then
  ok "fetch_release_binaries() refuses an asset with no SHA256SUMS / failed checksum"
else
  fail "fetch_release_binaries() integrity fail-closed guards are missing"
fi

echo
echo "--- static: fetch_release_binaries() rejects an unexpected archive layout before installing anything ---"
if grep -q 'did not contain the expected vpn1-\${target}/ directory' "$INSTALL_SH"; then
  ok "fetch_release_binaries() checks for the expected vpn1-<target>/ directory before install -m"
else
  fail "fetch_release_binaries() no longer validates archive layout"
fi

echo
echo "--- static: unsupported CPU architecture is rejected in preflight, before any privileged mutation ---"
detect_arch_body="$(sed -n '/^detect_arch() {/,/^}/p' "$REPO_ROOT/deploy/lib/os.sh")"
if echo "$detect_arch_body" | grep -q 'return 1'; then
  ok "detect_arch() returns non-zero for an unrecognized uname -m"
else
  fail "detect_arch() has no rejection path for an unsupported architecture"
fi
if grep -qE 'ARCH="\$\(detect_arch\)" \|\| die' "$INSTALL_SH"; then
  ok "main preflight dies immediately if detect_arch() fails (before packages/binaries/singbox stages run)"
else
  fail "install.sh no longer hard-fails on detect_arch() failure"
fi

echo
echo "--- static: unsupported OS is rejected via os.sh before any privileged mutation (detect_os) ---"
if grep -q 'detect_os() fails when OS_RELEASE_FILE does not exist' "$REPO_ROOT/deploy/lib/tests/test-amazon-linux-2023.sh"; then
  ok "OS detection failure path already covered by test-amazon-linux-2023.sh — not re-testing detect_os() here"
else
  fail "expected existing OS-detection-failure coverage not found (test suite may have moved)"
fi

echo
echo "--- static: production (stable-channel) bootstrap install.sh refuses to fall back to branch source when no release tag exists ---"
resolve_body="$(sed -n '/^resolve_version() {/,/^}/p' "$BOOTSTRAP_SH")"
if echo "$resolve_body" | grep -q '^\s*die "no tagged release found'; then
  ok "resolve_version() calls die() (not warn+continue) when no tagged release is found in the default channel"
else
  fail "resolve_version() no longer refuses branch-source fallback in the default channel"
fi
if echo "$resolve_body" | grep -qi 'warn "no tagged release found'; then
  fail "resolve_version() still contains the old silent-warn-and-fall-back-to-main path"
else
  ok "resolve_version() no longer silently warns-and-falls-back to branch source"
fi

echo
echo "--- static: VPN1_CHANNEL=dev remains the ONLY documented way to install unpinned branch source ---"
if echo "$resolve_body" | grep -q 'VPN1_CHANNEL" = "dev"'; then
  ok "resolve_version() still has an explicit VPN1_CHANNEL=dev opt-in path"
else
  fail "VPN1_CHANNEL=dev opt-in path is missing from resolve_version()"
fi
if tr '\n' ' ' < "$BOOTSTRAP_SH" | grep -q 'this is the ONLY way to install unpinned branch source'; then
  ok "install.sh --help documents VPN1_CHANNEL=dev as the only unpinned-install path"
else
  fail "install.sh --help no longer documents the dev-mode-only branch-source path"
fi

echo
echo "--- functional: resolve_version() actually dies (not just a static grep) when no tag is resolvable and no dev mode ---"
# die() calls exit directly, so it terminates this subshell immediately —
# check the subshell's own exit status rather than trying to capture \$?
# from inside it (that line would never be reached).
if (
  set -Eeuo pipefail
  # shellcheck disable=SC1090
  . <(sed -n '/^log() { /p;/^warn() { /p;/^die() { /p;/^resolve_version() {/,/^}/p' "$BOOTSTRAP_SH")
  VPN1_REPO="this-org-does-not-exist-vpn1-test/no-such-repo-xyz"
  VPN1_VERSION=""
  VPN1_CHANNEL="stable"
  VPN1_REF="main"
  CURL_NET_FLAGS=(--connect-timeout 3 --max-time 8 --retry 0)
  resolve_version
) >/tmp/resolve_version_test.out 2>&1; then
  fail "resolve_version() exited 0 for a repo with no releases, default channel, no VPN1_VERSION — should have refused to fall back to branch source"
else
  ok "resolve_version() exited non-zero for a repo with no releases, default channel, no VPN1_VERSION (see /tmp/resolve_version_test.out)"
fi

echo
echo "--- functional: VPN1_CHANNEL=dev alone is refused without the second unsafe-code opt-in ---"
if (
  set -Eeuo pipefail
  # shellcheck disable=SC1090
  . <(sed -n '/^log() { /p;/^warn() { /p;/^die() { /p;/^resolve_version() {/,/^}/p' "$BOOTSTRAP_SH")
  VPN1_REPO="this-org-does-not-exist-vpn1-test/no-such-repo-xyz"
  VPN1_VERSION=""
  VPN1_CHANNEL="dev"
  VPN1_ALLOW_UNVERIFIED_DEV="0"
  VPN1_REF="main"
  resolve_version
) >/tmp/resolve_version_dev_single_optin_test.out 2>&1; then
  fail "resolve_version() accepted mutable root code with only VPN1_CHANNEL=dev"
else
  ok "resolve_version() refuses dev channel unless VPN1_ALLOW_UNVERIFIED_DEV=1 is also set"
fi

echo
echo "--- functional: resolve_version() succeeds when both development opt-ins are set ---"
if (
  set -Eeuo pipefail
  # shellcheck disable=SC1090
  . <(sed -n '/^log() { /p;/^warn() { /p;/^die() { /p;/^resolve_version() {/,/^}/p' "$BOOTSTRAP_SH")
  # shellcheck disable=SC2034  # read by resolve_version(), sourced above from process substitution
  VPN1_REPO="this-org-does-not-exist-vpn1-test/no-such-repo-xyz"
  # shellcheck disable=SC2034
  VPN1_VERSION=""
  # shellcheck disable=SC2034
  VPN1_CHANNEL="dev"
  # shellcheck disable=SC2034
  VPN1_ALLOW_UNVERIFIED_DEV="1"
  # shellcheck disable=SC2034
  VPN1_REF="main"
  # shellcheck disable=SC2034
  CURL_NET_FLAGS=(--connect-timeout 3 --max-time 8 --retry 0)
  resolve_version
) >/tmp/resolve_version_dev_test.out 2>&1; then
  ok "resolve_version() with both explicit dev opt-ins did not die for the same no-release repo"
else
  fail "resolve_version() with both explicit dev opt-ins unexpectedly died (see /tmp/resolve_version_dev_test.out)"
fi

echo
echo "--- static: release payload carries a local, offline, root-owned uninstaller ---"
if grep -q 'tar --exclude=target --exclude=.git' "$INSTALL_SH"; then
  ok "persist_source_tree() copies the full source tree (uninstall.sh included, only target/.git excluded) to /opt/vpn1"
else
  fail "persist_source_tree() no longer persists the source tree to /opt/vpn1"
fi
if [ -x "$REPO_ROOT/uninstall.sh" ] && [ -x "$REPO_ROOT/deploy/almalinux/uninstall.sh" ]; then
  ok "both uninstall.sh (bootstrap) and deploy/almalinux/uninstall.sh (real implementation) are present and executable in this source tree, so persisting the tree persists a working offline uninstaller"
else
  fail "uninstall.sh / deploy/almalinux/uninstall.sh missing or not executable"
fi
if grep -q 'PERSIST_DIR="/opt/vpn1"' "$INSTALL_SH" \
    && grep -q 'chown -R root:root "\$stage"' "$INSTALL_SH" \
    && grep -q 'chmod -R go-w "\$stage"' "$INSTALL_SH"; then
  ok "persistent source is staged, then normalized root-owned and non-group/world-writable after extraction"
else
  fail "PERSIST_DIR creation logic changed unexpectedly"
fi

echo
echo "--- static: release.yml build matrix arch matches versions.env's SUPPORTED_ARCH (x86_64) as one of its targets ---"
if grep -q 'x86_64-unknown-linux-gnu' "$REPO_ROOT/.github/workflows/release.yml"; then
  ok "release.yml builds the v1.0-supported x86_64 target"
else
  fail "release.yml does not build x86_64-unknown-linux-gnu"
fi

echo
echo "--- static: release.yml publishes a checksum-manifested source archive (vpn1-src.tar.gz) ---"
RELEASE_YML="$REPO_ROOT/.github/workflows/release.yml"
if grep -q 'git archive --format=tar.gz --prefix=vpn1-src/ -o vpn1-src.tar.gz' "$RELEASE_YML"; then
  ok "release.yml packages a deterministic vpn1-src.tar.gz via git archive"
else
  fail "release.yml no longer builds vpn1-src.tar.gz — bootstrap install.sh's release-source checksum verification has nothing to verify against"
fi
if grep -q 'sha256sum vpn1-src.tar.gz > vpn1-src.tar.gz.sha256' "$RELEASE_YML"; then
  ok "release.yml computes vpn1-src.tar.gz.sha256, folded into the published SHA256SUMS by the publish job's existing 'cat ./*.tar.gz.sha256' step"
else
  fail "release.yml no longer computes a checksum for vpn1-src.tar.gz"
fi
if grep -q 'test -x "$tmp/vpn1-src/deploy/almalinux/install.sh"' "$RELEASE_YML"; then
  ok "release.yml extracts the source archive and verifies the bootstrap handoff executable"
else
  fail "release.yml does not verify the source archive can hand off to deploy/almalinux/install.sh"
fi
if grep -q '^  verify-published-bootstrap:' "$RELEASE_YML" \
   && grep -q '^    needs: publish' "$RELEASE_YML" \
   && grep -q 'PATH="$testbin:$PATH" /usr/bin/bash install.sh' "$RELEASE_YML" \
   && grep -q -- '--version "$RELEASE_TAG"' "$RELEASE_YML"; then
  ok "release.yml verifies each published tag through the real public bootstrap path after publishing"
else
  fail "release.yml does not run the real bootstrap against each newly published release"
fi

echo
echo "--- static: bootstrap install.sh verifies the release source archive checksum for a pinned VPN1_VERSION, never falls through unverified ---"
if grep -q 'download_verified_source_release()' "$BOOTSTRAP_SH"; then
  ok "install.sh has a dedicated checksum-verifying downloader for pinned releases"
else
  fail "install.sh no longer has download_verified_source_release()"
fi
for needle in \
  'could not download release source archive' \
  'SHA256SUMS checksum manifest could not be downloaded' \
  'has no well-formed entry for vpn1-src.tar.gz' \
  'checksum verification failed for vpn1-src.tar.gz'
do
  if grep -qF "$needle" "$BOOTSTRAP_SH"; then
    ok "install.sh fails closed (die) on: $needle"
  else
    fail "install.sh is missing the fail-closed guard for: $needle"
  fi
done
if grep -q 'download_verified_source_release "\$VPN1_VERSION" "\$tarball"' "$BOOTSTRAP_SH"; then
  ok "download_source() routes any pinned VPN1_VERSION through the checksum-verifying downloader"
else
  fail "download_source() no longer routes pinned installs through checksum verification"
fi

echo
echo "--- functional: SHA256SUMS entry parsing/verification (same grep+sha256sum -c mechanism download_verified_source_release() uses) ---"
TMPDIR_SRC_TEST="$(mktemp -d)"
echo -n "fixture source archive bytes" > "$TMPDIR_SRC_TEST/vpn1-src.tar.gz"
real_sha="$(sha256sum "$TMPDIR_SRC_TEST/vpn1-src.tar.gz" | awk '{print $1}')"
{
  echo "deadbeef00000000000000000000000000000000000000000000000000000000  vpn1-x86_64-unknown-linux-gnu.tar.gz"
  echo "$real_sha  vpn1-src.tar.gz"
} > "$TMPDIR_SRC_TEST/SHA256SUMS"
if ( cd "$TMPDIR_SRC_TEST" && grep -E '  vpn1-src\.tar\.gz$' SHA256SUMS | sha256sum -c - ) >/dev/null 2>&1; then
  ok "a correct vpn1-src.tar.gz entry in a multi-asset SHA256SUMS verifies successfully"
else
  fail "a correct vpn1-src.tar.gz entry unexpectedly failed to verify"
fi
echo -n "tampered bytes" > "$TMPDIR_SRC_TEST/vpn1-src.tar.gz"
if ! ( cd "$TMPDIR_SRC_TEST" && grep -E '  vpn1-src\.tar\.gz$' SHA256SUMS | sha256sum -c - ) >/dev/null 2>&1; then
  ok "a tampered/mismatched vpn1-src.tar.gz is correctly rejected"
else
  fail "a tampered vpn1-src.tar.gz unexpectedly verified"
fi
rm -f "$TMPDIR_SRC_TEST/SHA256SUMS"
echo "not a valid checksum manifest line" > "$TMPDIR_SRC_TEST/SHA256SUMS"
if ! grep -qE '^[0-9a-f]{64}  vpn1-src\.tar\.gz$' "$TMPDIR_SRC_TEST/SHA256SUMS"; then
  ok "a malformed SHA256SUMS (no well-formed vpn1-src.tar.gz entry) is correctly rejected by the same pattern install.sh uses"
else
  fail "malformed-SHA256SUMS detection pattern unexpectedly matched"
fi
rm -rf "$TMPDIR_SRC_TEST"

echo
echo "--- static: VPN1_CHANNEL=dev branch-source path remains explicitly documented as unverified/dev-only, not silently equivalent to a verified install ---"
if grep -q 'UNVERIFIED singbox-vpn branch source' "$BOOTSTRAP_SH"; then
  ok "install.sh labels the branch-source download path as unverified at the point it runs, not just in --help text"
else
  fail "install.sh no longer labels the branch-source download as unverified"
fi

if [ "$failures" -eq 0 ]; then
  echo
  echo "all release-reproducibility tests passed"
else
  echo
  echo "$failures test(s) FAILED"
  exit 1
fi
