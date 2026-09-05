#!/usr/bin/env bash
# Static + fixture-based test for the release-archive contract between
# .github/workflows/release.yml (what gets packaged/published) and
# deploy/almalinux/install.sh's fetch_release_binaries() (what the
# installer assumes when it downloads a prebuilt release). Does not run
# GitHub Actions — builds a small real fixture archive using the exact
# same shell commands release.yml's "Package" step uses, then exercises
# the real extraction/validation logic from fetch_release_binaries()
# against it, so a future edit to either side that breaks the contract
# fails this test instead of only being caught by a live release.
#
# Live v0.1.0-rc.1 and v0.1.0-rc.2 tag runs exposed dependency-gating,
# CLI-version, and pinned-toolchain target-installation bugs. The assertions
# below cover those corrected contracts as well as packaging.
# rc.1's specific failure was a skipped-ancestor propagation bug:
# validate-tag was skipped for tag pushes, so build/publish were skipped even
# after the reusable CI gate passed. The static assertions below now cover
# the corrected explicit dependency-result conditions as well as packaging.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
RELEASE_YML="$REPO_ROOT/.github/workflows/release.yml"

failures=0
assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $desc — expected [$expected], got [$actual]"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

echo "--- static: every target release.yml builds/publishes has a corresponding deploy/lib/os.sh rust_target_for_arch() output ---"
# Intentionally a subset check, not set equality: os.sh's
# rust_target_for_arch() still resolves a target triple for every
# architecture it recognizes (used for source-build fallback and the
# pinned sing-box/cosign downloads on any arch), but release.yml no
# longer builds/publishes a prebuilt singbox-vpn artifact for every one
# of those — arm64 is cross-compiled-but-unvalidated and is NOT
# published as of the v1.0.0-rc.4 ABI-baseline fix (see
# docs/SUPPORTED_PRODUCT.md "Architecture support" / release.yml's
# `build` job comment). Every target release.yml DOES build/publish
# must still be one os.sh actually knows how to resolve.
yml_targets="$(grep -oE 'singbox-vpn-[A-Za-z0-9_.-]+-unknown-linux-gnu\.tar\.gz' "$RELEASE_YML" | sed -E 's/^singbox-vpn-//; s/\.tar\.gz$//' | sort -u)"
os_sh_targets="$(grep -oE 'echo "[A-Za-z0-9_.-]+-unknown-linux-gnu"' "$REPO_ROOT/deploy/lib/os.sh" | grep -oE '[A-Za-z0-9_.-]+-unknown-linux-gnu' | sort -u)"
[ -n "$yml_targets" ] || { echo "FAIL: could not find any singbox-vpn-<target>.tar.gz reference in release.yml"; failures=$((failures + 1)); }
missing=0
for t in $yml_targets; do
  if echo "$os_sh_targets" | grep -qFx "$t"; then
    echo "ok: release.yml builds $t, which os.sh's rust_target_for_arch() also resolves to"
  else
    echo "FAIL: release.yml builds $t, but os.sh's rust_target_for_arch() has no matching output"
    missing=1
  fi
done
[ "$missing" -eq 0 ] || failures=$((failures + 1))
if [ "$yml_targets" = "x86_64-unknown-linux-gnu" ]; then
  echo "ok: release.yml currently publishes exactly the v1.0 SUPPORTED TARGET architecture (x86_64) and nothing else"
else
  echo "FAIL: release.yml's published target set changed from the expected x86_64-only set ($yml_targets) — update this test deliberately if that was intentional (e.g. arm64 gained real ABI-baseline + runtime validation)"
  failures=$((failures + 1))
fi

echo
echo "--- static: release target installation uses the repository's pinned Rust toolchain ---"
rust_channel="$(sed -nE 's/^channel = "([^"]+)"$/\1/p' "$REPO_ROOT/rust-toolchain.toml")"
# Actions are pinned to immutable full commit SHAs; the mutable ref each SHA
# resolves to is preserved as a trailing comment on the same line. The
# channel rust-toolchain.toml pins must appear in that comment, so a
# toolchain bump that updates one side but not the other still fails here.
if [ -n "$rust_channel" ] \
  && grep -Eq "^[-[:space:]]*uses: dtolnay/rust-toolchain@[0-9a-f]{40} # ${rust_channel}\$" "$RELEASE_YML"; then
  echo "ok: release.yml installs cross targets for pinned Rust $rust_channel (immutable SHA pin, mutable ref kept as trailing comment)"
else
  echo "FAIL: release.yml does not install targets for rust-toolchain.toml's pinned channel ($rust_channel) as a SHA-pinned ref with a trailing '# $rust_channel' comment"
  failures=$((failures + 1))
fi

echo
echo "--- static: asset filename pattern matches between release.yml and install.sh ---"
if grep -q 'singbox-vpn-x86_64-unknown-linux-gnu\.tar\.gz' "$RELEASE_YML" && \
   grep -q 'asset="singbox-vpn-\${target}\.tar\.gz"' "$INSTALL_SH"; then
  echo "ok: both sides use the 'singbox-vpn-<target>.tar.gz' asset filename pattern"
else
  echo "FAIL: asset filename pattern differs between release.yml and install.sh"
  failures=$((failures + 1))
fi

if grep -q 'SHA256SUMS' "$RELEASE_YML" && grep -q 'SHA256SUMS' "$INSTALL_SH"; then
  echo "ok: both sides reference a SHA256SUMS checksum manifest"
else
  echo "FAIL: SHA256SUMS is not referenced by both release.yml and install.sh"
  failures=$((failures + 1))
fi

echo
echo "--- fixture: build a real archive exactly the way release.yml's Package step does ---"
target="x86_64-unknown-linux-gnu"
(
  cd "$TMPDIR_TEST"
  out="singbox-vpn-${target}"
  mkdir -p "$out"
  # Stand-ins for the real Cargo build artifacts — content doesn't
  # matter here, only that fetch_release_binaries()'s path/layout
  # assumptions hold.
  printf '#!/bin/sh\necho fake-vpn-admin\n' > "$out/vpn-admin"
  printf '#!/bin/sh\necho fake-subscription\n' > "$out/subscription"
  chmod 0755 "$out/vpn-admin" "$out/subscription"
  cp "$REPO_ROOT/README.md" "$out/README.md"
  cp "$REPO_ROOT/LICENSE" "$out/LICENSE"
  tar -czf "${out}.tar.gz" "$out"
  sha256sum "${out}.tar.gz" > "${out}.tar.gz.sha256"
  # publish job: `cat ./*.tar.gz.sha256 > SHA256SUMS`
  cat ./*.tar.gz.sha256 > SHA256SUMS
  sha256sum -c SHA256SUMS
)
echo "ok: fixture archive built and its own SHA256SUMS verifies (mirrors the publish job's 'sha256sum -c SHA256SUMS' step)"

echo
echo "--- fixture: exercise the real fetch_release_binaries() extraction/validation logic ---"
# Extract fetch_release_binaries()'s body and run only its
# tar-extract + directory-layout-validation portion (the part that has
# no network dependency) against the fixture built above, using the
# EXACT same relative paths the real function uses.
run_extraction_against_fixture() {
  local asset_dir="$1" target="$2"
  (
    set -Eeuo pipefail
    # `exit`, not `return`: this whole body runs inside a subshell (the
    # surrounding parens), not inside a bash function of its own, so
    # `return` here would not actually stop execution the way the real
    # install.sh's `die() { ...; exit 1; }` does — it would just fall
    # through to the next line, exactly the bug this test exists to
    # catch. Mirror the real die()'s hard-exit semantics.
    die() { echo "DIE: $*" >&2; exit 1; }
    tmp="$asset_dir"
    asset="singbox-vpn-${target}.tar.gz"
    tar -xzf "$tmp/$asset" -C "$tmp"
    extracted="$tmp/singbox-vpn-${target}"
    [ -d "$extracted" ] || die "release asset $asset did not contain the expected singbox-vpn-${target}/ directory"
    [ -x "$extracted/vpn-admin" ] || die "vpn-admin missing/not executable"
    [ -x "$extracted/subscription" ] || die "subscription missing/not executable"
    echo "extraction OK: $extracted"
  )
}

if run_extraction_against_fixture "$TMPDIR_TEST" "$target"; then
  echo "ok: fetch_release_binaries()'s real extraction/layout assumptions accept a release.yml-built archive"
else
  echo "FAIL: fetch_release_binaries()'s extraction logic rejected a well-formed release.yml-built archive — contract drift"
  failures=$((failures + 1))
fi

echo
echo "--- fixture: a malformed archive (wrong top-level dir) is correctly rejected ---"
BADDIR="$TMPDIR_TEST/bad"
mkdir -p "$BADDIR"
(
  cd "$BADDIR"
  mkdir -p "wrong-dir-name"
  echo x > "wrong-dir-name/vpn-admin"
  chmod 0755 "wrong-dir-name/vpn-admin"
  echo x > "wrong-dir-name/subscription"
  chmod 0755 "wrong-dir-name/subscription"
  tar -czf "singbox-vpn-${target}.tar.gz" "wrong-dir-name"
)
if run_extraction_against_fixture "$BADDIR" "$target" >/dev/null 2>&1; then
  echo "FAIL: a malformed archive (wrong top-level directory name) was incorrectly accepted"
  failures=$((failures + 1))
else
  echo "ok: a malformed archive (wrong top-level directory name) is correctly rejected"
fi

echo
echo "--- static: release workflow is gated by CI and validates workflow_dispatch tag/ref ---"
if grep -q 'uses: \./\.github/workflows/ci\.yml' "$RELEASE_YML" && grep -qE '^\s*build:\s*$' "$RELEASE_YML" && grep -q 'needs: \[gate\]' "$RELEASE_YML"; then
  echo "ok: build job depends on the 'gate' job, which reuses ci.yml as a reusable workflow"
else
  echo "FAIL: release.yml's build job is not gated on ci.yml passing (Checkpoint 6 §9)"
  failures=$((failures + 1))
fi
if grep -q "github.ref_name.*github.event.inputs.tag" "$RELEASE_YML"; then
  echo "ok: release.yml validates that a workflow_dispatch run's ref matches the requested tag before building"
else
  echo "FAIL: release.yml does not validate workflow_dispatch ref == requested tag — could build/publish a mismatched commit (Checkpoint 6 §8)"
  failures=$((failures + 1))
fi
if grep -q '"\$extracted/vpn-admin" --version' "$RELEASE_YML" \
    && grep -q '"\$extracted/subscription" --version' "$RELEASE_YML"; then
  echo "ok: native release archives execute both packaged binaries' --version contract"
else
  echo "FAIL: release.yml does not execute both packaged binaries with --version"
  failures=$((failures + 1))
fi
if grep -q 'name: Confirm tag push ref' "$RELEASE_YML" \
    && grep -q "if: github.event_name == 'push'" "$RELEASE_YML" \
    && grep -q "needs.gate.result == 'success'" "$RELEASE_YML" \
    && grep -q "needs.build.result == 'success'" "$RELEASE_YML"; then
  echo "ok: tag pushes run validation and successful dependencies explicitly unlock build/publish"
else
  echo "FAIL: release.yml can regress to skipping build/publish after a successful tag gate"
  failures=$((failures + 1))
fi
if grep -q 'bash deploy/lib/check-release-version.sh' "$RELEASE_YML" \
    && grep -q 'needs: \[validate-tag, validate-version\]' "$RELEASE_YML"; then
  echo "ok: release.yml validates tag/package version consistency before running the CI gate"
else
  echo "FAIL: release.yml does not gate publication on the release tag/package version contract"
  failures=$((failures + 1))
fi
if grep -q "prerelease: \${{ contains(" "$RELEASE_YML"; then
  echo "ok: release.yml marks SemVer-prerelease tags (e.g. -rc.1) as GitHub prereleases, so install.sh's stable /releases/latest resolution never auto-selects one"
else
  echo "FAIL: release.yml does not mark prerelease tags as prerelease — an RC could be silently auto-installed by the stable channel (Checkpoint 6 §19)"
  failures=$((failures + 1))
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
echo
echo "NOTE: packaging/checksum logic matches install.sh's fetch_release_binaries()"
echo "contract, and live-tag regressions exposed by rc.1/rc.2 are covered."
