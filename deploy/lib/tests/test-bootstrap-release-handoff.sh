#!/usr/bin/env bash
# End-to-end regression for the top-level stable bootstrap's verified source
# download, extraction, and handoff. This executes the real install.sh with a
# fake GitHub download surface and a tiny stand-in deployment installer.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BOOTSTRAP="$REPO_ROOT/install.sh"
PACKAGE_VERSION="0.1.3"
FIXTURE_VERSION="v${PACKAGE_VERSION}-rc.1"

if [ ! -f /etc/os-release ]; then
  echo "bootstrap verified-release source handoff: SKIP (non-Linux host)"
  exit 0
fi

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT

mkdir -p "$TEST_TMP/source/singbox-vpn-src/deploy/almalinux" "$TEST_TMP/source/singbox-vpn-src/apps/admin" "$TEST_TMP/fakebin"
# Matches the real repo's layout: apps/admin/Cargo.toml only declares
# version.workspace = true (deploy/lib/check-workspace-version-
# consistency.sh enforces this) — the authoritative version lives in the
# root Cargo.toml's [workspace.package] section, which is what
# install.sh's download_source() actually parses.
printf '[workspace.package]\nversion = "%s"\n' "$PACKAGE_VERSION" > "$TEST_TMP/source/singbox-vpn-src/Cargo.toml"

cat > "$TEST_TMP/source/singbox-vpn-src/deploy/almalinux/install.sh" <<'INSTALLER'
#!/usr/bin/env bash
set -Eeuo pipefail
[ "${SINGBOX_VPN_RELEASE_REPO:-}" = "David610/singbox-vpn" ]
[ "${SINGBOX_VPN_VERSION:-}" = "$FIXTURE_VERSION" ]
printf '%s\n' "$*" > "$TEST_MARKER"
INSTALLER
chmod 0755 "$TEST_TMP/source/singbox-vpn-src/deploy/almalinux/install.sh"

tar -czf "$TEST_TMP/singbox-vpn-src.tar.gz" -C "$TEST_TMP/source" singbox-vpn-src
(
  cd "$TEST_TMP"
  sha256sum singbox-vpn-src.tar.gz > SHA256SUMS
)

cat > "$TEST_TMP/fakebin/id" <<'FAKE_ID'
#!/usr/bin/env bash
if [ "${1:-}" = "-u" ]; then
  echo 0
else
  exec /usr/bin/id "$@"
fi
FAKE_ID
chmod 0755 "$TEST_TMP/fakebin/id"

cat > "$TEST_TMP/fakebin/cosign" <<'FAKE_COSIGN'
#!/usr/bin/env bash
set -Eeuo pipefail
[ "$1" = verify-blob-attestation ]
identity=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --certificate-identity) identity="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$identity" in
  *David610/singbox-vpn*) ;;
  *) echo "unexpected certificate identity: $identity" >&2; exit 1 ;;
esac
case "${ATTESTATION_RESULT:-valid}" in
  valid) exit 0 ;;
  missing) echo "no matching attestations found in the transparency log" >&2; exit 1 ;;
  wrong-repository|wrong-workflow) echo "certificate identity did not match" >&2; exit 1 ;;
  modified) echo "attestation subject digest did not match" >&2; exit 1 ;;
esac
FAKE_COSIGN
chmod 0755 "$TEST_TMP/fakebin/cosign"

cat > "$TEST_TMP/fakebin/curl" <<'FAKE_CURL'
#!/usr/bin/env bash
set -Eeuo pipefail
out=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      out="$2"
      shift 2
      ;;
    --connect-timeout|--max-time|--speed-limit|--speed-time|--retry|--retry-delay)
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

case "$url" in
  */releases/download/"$FIXTURE_VERSION"/singbox-vpn-src.tar.gz)
    cp "$FIXTURE_TAR" "$out"
    ;;
  */releases/download/"$FIXTURE_VERSION"/SHA256SUMS)
    cp "$FIXTURE_SUMS" "$out"
    ;;
  */releases/download/*/singbox-vpn-src.tar.gz.sigstore.json)
    printf '{}' > "$out"
    ;;
  */releases/latest)
    printf '{"tag_name":"%s"}\n' "$FIXTURE_VERSION"
    ;;
  *)
    echo "unexpected curl URL: $url" >&2
    exit 1
    ;;
esac
FAKE_CURL
chmod 0755 "$TEST_TMP/fakebin/curl"

export FIXTURE_TAR="$TEST_TMP/singbox-vpn-src.tar.gz"
export FIXTURE_SUMS="$TEST_TMP/SHA256SUMS"
export FIXTURE_VERSION
export TEST_MARKER="$TEST_TMP/handoff-args.txt"

if ! PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" \
    --version "$FIXTURE_VERSION" \
    --non-interactive \
    --allow-ip-hostname \
    --reality-handshake-server www.cloudflare.com \
    >"$TEST_TMP/bootstrap.log" 2>&1; then
  cat "$TEST_TMP/bootstrap.log" >&2
  echo "FAIL: stable bootstrap did not reach the extracted deployment installer" >&2
  exit 1
fi

expected_args="--non-interactive --allow-ip-hostname --reality-handshake-server www.cloudflare.com"
actual_args="$(cat "$TEST_MARKER")"
[ "$actual_args" = "$expected_args" ] || {
  echo "FAIL: bootstrap argument handoff changed: [$actual_args]" >&2
  exit 1
}

grep -q 'source archive checksum verified' "$TEST_TMP/bootstrap.log"
grep -q 'artifact attestation verified' "$TEST_TMP/bootstrap.log"
grep -q 'handing off to deploy/almalinux/install.sh' "$TEST_TMP/bootstrap.log"
if grep -q 'downloaded source is missing' "$TEST_TMP/bootstrap.log"; then
  echo "FAIL: verified source directory was corrupted by command output" >&2
  exit 1
fi

for bad_result in missing wrong-repository wrong-workflow modified; do
  rm -f "$TEST_MARKER"
  rc=0
  ATTESTATION_RESULT="$bad_result" PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" \
    --version "$FIXTURE_VERSION" --non-interactive --allow-ip-hostname \
    --reality-handshake-server www.cloudflare.com \
    >"$TEST_TMP/bootstrap-$bad_result.log" 2>&1 || rc=$?
  [ "$rc" -ne 0 ] || { echo "FAIL: $bad_result attestation was accepted" >&2; exit 1; }
  [ ! -e "$TEST_MARKER" ] || { echo "FAIL: installer handoff ran after $bad_result attestation" >&2; exit 1; }
  grep -q 'attestation verification failed or is missing' "$TEST_TMP/bootstrap-$bad_result.log"
done

# There is no version-gated fallback: an old-numbered tag with no valid
# attestation must fail closed exactly like a new one would (see
# verify_release_attestation() in install.sh). A version-gated exemption
# would be gated on the release's own attacker-suppliable version string,
# letting anyone with release-publish access republish an old-numbered tag
# with malicious content and skip attestation.
LEGACY_VERSION=v0.1.2
printf '[workspace.package]\nversion = "0.1.2"\n' > "$TEST_TMP/source/singbox-vpn-src/Cargo.toml"
tar -czf "$TEST_TMP/legacy.tar.gz" -C "$TEST_TMP/source" singbox-vpn-src
printf '%s  singbox-vpn-src.tar.gz\n' "$(sha256sum "$TEST_TMP/legacy.tar.gz" | awk '{print $1}')" > "$TEST_TMP/legacy-SHA256SUMS"
rm -f "$TEST_MARKER"
rc=0
FIXTURE_VERSION="$LEGACY_VERSION" FIXTURE_TAR="$TEST_TMP/legacy.tar.gz" \
  FIXTURE_SUMS="$TEST_TMP/legacy-SHA256SUMS" ATTESTATION_RESULT=missing \
  PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" --version "$LEGACY_VERSION" \
  --non-interactive --allow-ip-hostname --reality-handshake-server www.cloudflare.com \
  >"$TEST_TMP/bootstrap-legacy.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: old-numbered release with no attestation was accepted" >&2; exit 1; }
[ ! -e "$TEST_MARKER" ] || { echo "FAIL: old-numbered release with no attestation reached installer handoff" >&2; exit 1; }
grep -q 'attestation verification failed or is missing' "$TEST_TMP/bootstrap-legacy.log"

# Restore the new-release fixture consumed by the legacy-version case above,
# for the remaining wrong-version check below.
FIXTURE_VERSION="v${PACKAGE_VERSION}-rc.1"
printf '[workspace.package]\nversion = "%s"\n' "$PACKAGE_VERSION" > "$TEST_TMP/source/singbox-vpn-src/Cargo.toml"

# Even a valid repository attestation is not enough if a release page is wired
# to an archive built for a different package version.
printf '[workspace.package]\nversion = "9.9.9"\n' > "$TEST_TMP/source/singbox-vpn-src/Cargo.toml"
tar -czf "$TEST_TMP/wrong-version.tar.gz" -C "$TEST_TMP/source" singbox-vpn-src
printf '%s  singbox-vpn-src.tar.gz\n' "$(sha256sum "$TEST_TMP/wrong-version.tar.gz" | awk '{print $1}')" > "$TEST_TMP/wrong-version-SHA256SUMS"
rm -f "$TEST_MARKER"
rc=0
FIXTURE_TAR="$TEST_TMP/wrong-version.tar.gz" FIXTURE_SUMS="$TEST_TMP/wrong-version-SHA256SUMS" \
  PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" --version "$FIXTURE_VERSION" \
  --non-interactive --allow-ip-hostname --reality-handshake-server www.cloudflare.com \
  >"$TEST_TMP/bootstrap-wrong-version.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: wrong-version authenticated source was accepted" >&2; exit 1; }
[ ! -e "$TEST_MARKER" ] || { echo "FAIL: wrong-version source reached installer handoff" >&2; exit 1; }
grep -q 'does not match requested release' "$TEST_TMP/bootstrap-wrong-version.log"

# A modified archive must be rejected by SHA256SUMS before provenance or
# installer handoff can make it relevant.
cp "$TEST_TMP/singbox-vpn-src.tar.gz" "$TEST_TMP/modified.tar.gz"
printf 'tamper' >> "$TEST_TMP/modified.tar.gz"
rm -f "$TEST_MARKER"
rc=0
FIXTURE_TAR="$TEST_TMP/modified.tar.gz" FIXTURE_SUMS="$TEST_TMP/SHA256SUMS" \
  PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" --version "$FIXTURE_VERSION" \
  --non-interactive --allow-ip-hostname --reality-handshake-server www.cloudflare.com \
  >"$TEST_TMP/bootstrap-checksum-mismatch.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: checksum-mismatched source was accepted" >&2; exit 1; }
[ ! -e "$TEST_MARKER" ] || { echo "FAIL: checksum-mismatched source reached installer handoff" >&2; exit 1; }
grep -q 'checksum verification failed' "$TEST_TMP/bootstrap-checksum-mismatch.log"

echo "bootstrap verified-release source handoff: PASS"
