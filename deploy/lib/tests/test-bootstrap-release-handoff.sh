#!/usr/bin/env bash
# End-to-end regression for the top-level stable bootstrap's verified source
# download, extraction, and handoff. This executes the real install.sh with a
# fake GitHub download surface and a tiny stand-in deployment installer.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BOOTSTRAP="$REPO_ROOT/install.sh"
PACKAGE_VERSION="$(sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' "$REPO_ROOT/apps/admin/Cargo.toml" | head -n1)"
[ -n "$PACKAGE_VERSION" ] || {
  echo "FAIL: could not resolve admin package version" >&2
  exit 1
}
FIXTURE_VERSION="v${PACKAGE_VERSION}-test.1"

if [ ! -f /etc/os-release ]; then
  echo "bootstrap verified-release source handoff: SKIP (non-Linux host)"
  exit 0
fi

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT

mkdir -p "$TEST_TMP/source/vpn1-src/deploy/almalinux" "$TEST_TMP/source/vpn1-src/apps/admin" "$TEST_TMP/fakebin"
printf 'version = "%s"\n' "$PACKAGE_VERSION" > "$TEST_TMP/source/vpn1-src/apps/admin/Cargo.toml"

cat > "$TEST_TMP/source/vpn1-src/deploy/almalinux/install.sh" <<'INSTALLER'
#!/usr/bin/env bash
set -Eeuo pipefail
[ "${VPN1_RELEASE_REPO:-}" = "David610/singbox-vpn" ]
[ "${VPN1_VERSION:-}" = "$FIXTURE_VERSION" ]
printf '%s\n' "$*" > "$TEST_MARKER"
INSTALLER
chmod 0755 "$TEST_TMP/source/vpn1-src/deploy/almalinux/install.sh"

tar -czf "$TEST_TMP/vpn1-src.tar.gz" -C "$TEST_TMP/source" vpn1-src
(
  cd "$TEST_TMP"
  sha256sum vpn1-src.tar.gz > SHA256SUMS
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

cat > "$TEST_TMP/fakebin/gh" <<'FAKE_GH'
#!/usr/bin/env bash
set -Eeuo pipefail
[ "$1" = attestation ] && [ "$2" = verify ]
[ "$4" = --repo ] && [ "$5" = David610/singbox-vpn ]
case "${GH_ATTESTATION_RESULT:-valid}" in
  valid) exit 0 ;;
  missing) echo "no attestations found" >&2; exit 1 ;;
  wrong) echo "attestation identity did not match" >&2; exit 1 ;;
esac
FAKE_GH
chmod 0755 "$TEST_TMP/fakebin/gh"

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
  */releases/download/"$FIXTURE_VERSION"/vpn1-src.tar.gz)
    cp "$FIXTURE_TAR" "$out"
    ;;
  */releases/download/"$FIXTURE_VERSION"/SHA256SUMS)
    cp "$FIXTURE_SUMS" "$out"
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

export FIXTURE_TAR="$TEST_TMP/vpn1-src.tar.gz"
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

for bad_result in missing wrong; do
  rm -f "$TEST_MARKER"
  rc=0
  GH_ATTESTATION_RESULT="$bad_result" PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" \
    --version "$FIXTURE_VERSION" --non-interactive --allow-ip-hostname \
    --reality-handshake-server www.cloudflare.com \
    >"$TEST_TMP/bootstrap-$bad_result.log" 2>&1 || rc=$?
  [ "$rc" -ne 0 ] || { echo "FAIL: $bad_result attestation was accepted" >&2; exit 1; }
  [ ! -e "$TEST_MARKER" ] || { echo "FAIL: installer handoff ran after $bad_result attestation" >&2; exit 1; }
  grep -q 'attestation verification failed or is missing' "$TEST_TMP/bootstrap-$bad_result.log"
done

# Even a valid repository attestation is not enough if a release page is wired
# to an archive built for a different package version.
printf 'version = "9.9.9"\n' > "$TEST_TMP/source/vpn1-src/apps/admin/Cargo.toml"
tar -czf "$TEST_TMP/wrong-version.tar.gz" -C "$TEST_TMP/source" vpn1-src
printf '%s  vpn1-src.tar.gz\n' "$(sha256sum "$TEST_TMP/wrong-version.tar.gz" | awk '{print $1}')" > "$TEST_TMP/wrong-version-SHA256SUMS"
rm -f "$TEST_MARKER"
rc=0
FIXTURE_TAR="$TEST_TMP/wrong-version.tar.gz" FIXTURE_SUMS="$TEST_TMP/wrong-version-SHA256SUMS" \
  PATH="$TEST_TMP/fakebin:$PATH" bash "$BOOTSTRAP" --version "$FIXTURE_VERSION" \
  --non-interactive --allow-ip-hostname --reality-handshake-server www.cloudflare.com \
  >"$TEST_TMP/bootstrap-wrong-version.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: wrong-version authenticated source was accepted" >&2; exit 1; }
[ ! -e "$TEST_MARKER" ] || { echo "FAIL: wrong-version source reached installer handoff" >&2; exit 1; }
grep -q 'does not match requested release' "$TEST_TMP/bootstrap-wrong-version.log"

echo "bootstrap verified-release source handoff: PASS"
