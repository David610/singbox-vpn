#!/usr/bin/env bash
# Regression for the boundary that test-bootstrap-release-handoff.sh does
# NOT cover: CURRENT MAIN'S BOOTSTRAP resolving GitHub's actual
# /releases/latest (no --version pinned) versus what that resolved
# release's assets are actually named.
#
# This is deterministic/hermetic (a fake curl, never live GitHub) so CI
# never depends on what the real /releases/latest happens to be at any
# given moment:
#   - a compatible latest release (new "singbox-vpn-src.tar.gz" naming) is
#     accepted and reaches the deployment-installer handoff.
#   - an incompatible historical-style latest release (the obsolete
#     asset-naming contract) is rejected BEFORE extraction/handoff, with a
#     clear "predates this bootstrap's release-asset contract" message —
#     never a silent mutable-source fallback, never a bypassed checksum.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BOOTSTRAP="$REPO_ROOT/install.sh"

if [ ! -f /etc/os-release ]; then
  echo "release/latest bootstrap contract: SKIP (non-Linux host)"
  exit 0
fi

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT

# Historical asset name, built from parts (never written out as one
# literal string) exactly like deploy/lib/check-no-legacy-identity.sh —
# this repro must not reintroduce the obsolete identifier into the tree.
LEGACY_PREFIX="vpn"
LEGACY_SUFFIX="1"
LEGACY_NAME="${LEGACY_PREFIX}${LEGACY_SUFFIX}"
LEGACY_ASSET="${LEGACY_NAME}-src.tar.gz"

mkdir -p "$TEST_TMP/source/singbox-vpn-src/deploy/almalinux" "$TEST_TMP/source/singbox-vpn-src/apps/admin" "$TEST_TMP/fakebin"
printf 'version = "0.1.3"\n' > "$TEST_TMP/source/singbox-vpn-src/apps/admin/Cargo.toml"
cat > "$TEST_TMP/source/singbox-vpn-src/deploy/almalinux/install.sh" <<'INSTALLER'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" > "$TEST_MARKER"
INSTALLER
chmod 0755 "$TEST_TMP/source/singbox-vpn-src/deploy/almalinux/install.sh"
tar -czf "$TEST_TMP/singbox-vpn-src.tar.gz" -C "$TEST_TMP/source" singbox-vpn-src
(cd "$TEST_TMP" && sha256sum singbox-vpn-src.tar.gz > SHA256SUMS)
# A historical-style release asset — same bytes, obsolete filename — so a
# fake curl 404s the new-name request against it exactly like the real
# GitHub API would for an actual pre-rename release.
cp "$TEST_TMP/singbox-vpn-src.tar.gz" "$TEST_TMP/$LEGACY_ASSET"

cat > "$TEST_TMP/fakebin/id" <<'FAKE_ID'
#!/usr/bin/env bash
if [ "${1:-}" = "-u" ]; then echo 0; else exec /usr/bin/id "$@"; fi
FAKE_ID
chmod 0755 "$TEST_TMP/fakebin/id"

cat > "$TEST_TMP/fakebin/gh" <<'FAKE_GH'
#!/usr/bin/env bash
[ "$1" = attestation ] && [ "$2" = verify ] && exit 0
exit 0
FAKE_GH
chmod 0755 "$TEST_TMP/fakebin/gh"

# LATEST_TAG / LATEST_STYLE select which fixture /releases/latest and the
# download URLs resolve to, for each scenario below.
cat > "$TEST_TMP/fakebin/curl" <<'FAKE_CURL'
#!/usr/bin/env bash
set -Eeuo pipefail
out="" url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    --connect-timeout|--max-time|--speed-limit|--speed-time|--retry|--retry-delay) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
case "$url" in
  */releases/latest)
    printf '{"tag_name":"%s"}\n' "$LATEST_TAG"
    ;;
  */releases/download/"$LATEST_TAG"/singbox-vpn-src.tar.gz)
    if [ "$LATEST_STYLE" = compatible ]; then
      cp "$FIXTURE_TAR" "$out"
    else
      echo "404 not found" >&2
      exit 22
    fi
    ;;
  */releases/download/"$LATEST_TAG"/SHA256SUMS)
    if [ "$LATEST_STYLE" = compatible ]; then
      cp "$FIXTURE_SUMS" "$out"
    else
      echo "404 not found" >&2
      exit 22
    fi
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
export TEST_MARKER="$TEST_TMP/handoff-args.txt"

# --- scenario 1: latest resolves to a COMPATIBLE release (new naming) --
rm -f "$TEST_MARKER"
if ! LATEST_TAG=v0.1.3 LATEST_STYLE=compatible PATH="$TEST_TMP/fakebin:$PATH" \
    bash "$BOOTSTRAP" --non-interactive --allow-ip-hostname \
    --reality-handshake-server www.cloudflare.com \
    >"$TEST_TMP/compatible.log" 2>&1; then
  cat "$TEST_TMP/compatible.log" >&2
  echo "FAIL: a compatible /releases/latest was rejected" >&2
  exit 1
fi
[ -e "$TEST_MARKER" ] || { echo "FAIL: compatible latest release did not reach installer handoff" >&2; exit 1; }

# --- scenario 2: latest resolves to an INCOMPATIBLE historical release -
# (old asset-naming contract) — must fail BEFORE extraction/handoff, with
# a clear message, and must NOT fall back to mutable/unverified source.
rm -f "$TEST_MARKER"
rc=0
LATEST_TAG=v0.1.2 LATEST_STYLE=historical PATH="$TEST_TMP/fakebin:$PATH" \
  bash "$BOOTSTRAP" --non-interactive --allow-ip-hostname \
  --reality-handshake-server www.cloudflare.com \
  >"$TEST_TMP/historical.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: an incompatible historical /releases/latest was accepted" >&2; exit 1; }
[ ! -e "$TEST_MARKER" ] || { echo "FAIL: incompatible historical release reached installer handoff" >&2; exit 1; }
grep -q "predates this bootstrap's release-asset" "$TEST_TMP/historical.log" \
  || { echo "FAIL: rejection message did not explain the release-asset contract mismatch" >&2; cat "$TEST_TMP/historical.log" >&2; exit 1; }
grep -qi "no server changes were made" "$TEST_TMP/historical.log" \
  || { echo "FAIL: rejection message did not confirm zero host mutation" >&2; cat "$TEST_TMP/historical.log" >&2; exit 1; }
if grep -qi "SINGBOX_VPN_CHANNEL=dev" "$TEST_TMP/historical.log"; then
  echo "FAIL: the stable-channel rejection must not suggest silently downloading unverified branch source" >&2
  exit 1
fi

echo "release/latest bootstrap contract: PASS"
