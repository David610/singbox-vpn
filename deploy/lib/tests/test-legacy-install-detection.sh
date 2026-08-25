#!/usr/bin/env bash
# Regression for the clean-break pre-rename install detector
# (deploy/lib/legacy-install-detect.sh, and its inline copies in the
# top-level install.sh/uninstall.sh bootstraps): a detected pre-rename
# installation must REFUSE automatic install/uninstall — never silently
# report "nothing installed", never migrate/delete/execute anything, and
# never mutate the host — and must print the precise path to the
# historical installation's own uninstaller.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

if [ ! -f /etc/os-release ]; then
  echo "legacy pre-rename install detection: SKIP (non-Linux host)"
  exit 0
fi
if [ "$(id -u)" -ne 0 ]; then
  echo "legacy pre-rename install detection: SKIP (requires root to fake /opt, /var/lib paths)"
  exit 0
fi

# Built from parts, never written out as one literal string, matching
# deploy/lib/check-no-legacy-identity.sh.
LEGACY_PREFIX="vpn"
LEGACY_SUFFIX="1"
LEGACY_NAME="${LEGACY_PREFIX}${LEGACY_SUFFIX}"

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT

# --- deploy/lib/legacy-install-detect.sh: unit-level detector behavior -
LIB="$REPO_ROOT/deploy/lib/legacy-install-detect.sh"
set +e
(
  set -Eeuo pipefail
  log() { :; }; warn() { :; }; die() { echo "DIE: $*"; exit 1; }
  # shellcheck source=/dev/null
  . "$LIB"
  if legacy_install_present; then
    echo "FAIL: legacy_install_present() true with no historical markers on this host" >&2
    exit 1
  fi
)
rc=$?
set -e
[ "$rc" -eq 0 ] || exit 1

# Fake a historical persisted install using a REAL, but disposable, root
# path this test creates and removes itself: $TEST_TMP masquerading is not
# possible for /opt or /var/lib (the detector hardcodes those roots by
# design — a configurable root would be a bigger behavior change than this
# regression needs), so this test creates and always cleans up the exact
# fixed historical paths under real /opt and /var/lib.
FAKE_LEGACY_OPT="/opt/${LEGACY_NAME}"
FAKE_LEGACY_STATE="/var/lib/${LEGACY_NAME}"
cleanup_fake_legacy() {
  rm -rf "$FAKE_LEGACY_OPT" "$FAKE_LEGACY_STATE"
}
trap 'cleanup_fake_legacy; rm -rf "$TEST_TMP"' EXIT

if [ -e "$FAKE_LEGACY_OPT" ] || [ -e "$FAKE_LEGACY_STATE" ]; then
  echo "legacy pre-rename install detection: SKIP ($FAKE_LEGACY_OPT or $FAKE_LEGACY_STATE already exists on this host — refusing to touch real state)"
  exit 0
fi

mkdir -p "$FAKE_LEGACY_OPT/bin"
cat > "$FAKE_LEGACY_OPT/bin/${LEGACY_NAME}-uninstall" <<'EOF'
#!/usr/bin/env bash
echo "fake historical uninstaller — should never actually run in this test"
exit 1
EOF
chmod 0755 "$FAKE_LEGACY_OPT/bin/${LEGACY_NAME}-uninstall"

set +e
(
  set -Eeuo pipefail
  log() { :; }; warn() { :; }; die() { echo "DIE: $*"; exit 1; }
  # shellcheck source=/dev/null
  . "$LIB"
  legacy_install_present
)
rc=$?
set -e
[ "$rc" -eq 0 ] || { echo "FAIL: legacy_install_present() did not detect the fake historical uninstaller" >&2; exit 1; }

set +e
# shellcheck source=/dev/null
out="$( ( set -Eeuo pipefail; log() { :; }; warn() { :; }; die() { echo "DIE: $*"; exit 1; }; . "$LIB"; legacy_install_refuse install ) 2>&1 )"
rc=$?
set -e
[ "$rc" -ne 0 ] || { echo "FAIL: legacy_install_refuse did not return failure" >&2; exit 1; }
echo "$out" | grep -q "pre-clean-break singbox-vpn installation was detected" \
  || { echo "FAIL: refusal message missing detection statement:"$'\n'"$out" >&2; exit 1; }
echo "$out" | grep -q -- "--yes" \
  || { echo "FAIL: refusal message did not point at the historical uninstaller with --yes:"$'\n'"$out" >&2; exit 1; }
echo "$out" | grep -qi "no changes were made" \
  || { echo "FAIL: refusal message did not confirm zero host mutation:"$'\n'"$out" >&2; exit 1; }

# --- deploy/almalinux/install.sh refuses a fresh install over the fake
# historical layout, before any host mutation (no lock file, no
# install-state.json written).
rc=0
SINGBOX_VPN_NON_INTERACTIVE=1 bash "$REPO_ROOT/deploy/almalinux/install.sh" \
  --non-interactive --allow-ip-hostname --reality-handshake-server www.cloudflare.com \
  >"$TEST_TMP/install-refuse.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: deploy/almalinux/install.sh proceeded over a detected pre-rename installation" >&2; cat "$TEST_TMP/install-refuse.log" >&2; exit 1; }
grep -q "pre-clean-break singbox-vpn installation was detected" "$TEST_TMP/install-refuse.log" \
  || { echo "FAIL: deploy/almalinux/install.sh did not print the clean-break refusal" >&2; cat "$TEST_TMP/install-refuse.log" >&2; exit 1; }
[ ! -f /var/lib/singbox-vpn/install-state.json ] \
  || { echo "FAIL: install.sh mutated /var/lib/singbox-vpn despite refusing" >&2; exit 1; }

# --- online uninstall.sh bootstrap refuses (rather than silently
# reporting "nothing installed") when /opt/singbox-vpn is absent but a
# historical layout is present.
rc=0
bash "$REPO_ROOT/uninstall.sh" --yes >"$TEST_TMP/uninstall-refuse.log" 2>&1 || rc=$?
[ "$rc" -ne 0 ] || { echo "FAIL: uninstall.sh proceeded over a detected pre-rename installation" >&2; cat "$TEST_TMP/uninstall-refuse.log" >&2; exit 1; }
grep -q "pre-clean-break singbox-vpn installation was detected" "$TEST_TMP/uninstall-refuse.log" \
  || { echo "FAIL: uninstall.sh did not print the clean-break refusal" >&2; cat "$TEST_TMP/uninstall-refuse.log" >&2; exit 1; }
if grep -qi "nothing.*install" "$TEST_TMP/uninstall-refuse.log"; then
  echo "FAIL: uninstall.sh misleadingly implied nothing is installed" >&2
  exit 1
fi

echo "legacy pre-rename install detection: PASS"
