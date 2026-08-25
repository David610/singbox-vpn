#!/usr/bin/env bash
# Detects a pre-clean-break installation left over from before the
# obsolete-identifier rename (docs/IMPLEMENTATION_STATUS.md,
# deploy/lib/check-no-legacy-identity.sh). Sourced by
# deploy/almalinux/install.sh; expects log()/warn()/die() to already be
# defined by the caller.
#
# Clean-break strategy: this repository does not carry a permanent
# compatibility/migration layer for the pre-rename layout. This file only
# DETECTS one, so install/uninstall can refuse and point the operator at
# that installation's own persisted local uninstaller instead of either
# (a) misreporting "nothing installed" or (b) running current logic
# against an unidentified historical layout.
#
# The banned identifier is built from parts, exactly like
# deploy/lib/check-no-legacy-identity.sh, so this file never contains the
# literal string it exists to detect.
LEGACY_PREFIX="vpn"
LEGACY_SUFFIX="1"
LEGACY_NAME="${LEGACY_PREFIX}${LEGACY_SUFFIX}"
LEGACY_OPT_DIR="/opt/${LEGACY_NAME}"
LEGACY_STATE_DIR="/var/lib/${LEGACY_NAME}"
LEGACY_UNINSTALLER="${LEGACY_OPT_DIR}/bin/${LEGACY_NAME}-uninstall"
LEGACY_LEGACY_UNINSTALLER="${LEGACY_OPT_DIR}/deploy/almalinux/uninstall.sh"
LEGACY_MANIFEST="${LEGACY_STATE_DIR}/install-state.json"

# Conservative on purpose: only a specific, product-owned marker counts —
# never a bare directory. /opt and /var/lib are shared system paths, and an
# unrelated directory that happens to collide with the historical name
# must never be treated as evidence of a real historical singbox-vpn
# install.
legacy_install_present() {
  [ -x "$LEGACY_UNINSTALLER" ] && return 0
  [ -f "$LEGACY_MANIFEST" ] && return 0
  [ -x "$LEGACY_LEGACY_UNINSTALLER" ] && [ -d "$LEGACY_STATE_DIR" ] && return 0
  return 1
}

# Prints the precise operator instruction for the given action
# ("install" or "uninstall") and returns 1 for the caller to propagate as
# a refusal. Never migrates, deletes, or executes anything itself.
legacy_install_refuse() {
  local action="$1" runnable=""
  [ -x "$LEGACY_UNINSTALLER" ] && runnable="$LEGACY_UNINSTALLER --yes"
  [ -z "$runnable" ] && [ -x "$LEGACY_LEGACY_UNINSTALLER" ] && runnable="$LEGACY_LEGACY_UNINSTALLER --yes"
  {
    echo "[$action] ERROR: a pre-clean-break singbox-vpn installation was detected at $LEGACY_OPT_DIR."
    echo "[$action] This release intentionally does not migrate or remove a historical installation automatically (clean-break rename policy)."
    echo "[$action] First remove it using the local uninstaller that belongs to THAT installation:"
    if [ -n "$runnable" ]; then
      echo "[$action]     sudo $runnable"
    else
      echo "[$action]     (look for an uninstaller under $LEGACY_OPT_DIR — e.g. $LEGACY_OPT_DIR/bin or $LEGACY_OPT_DIR/deploy)"
    fi
    echo "[$action] After it completes successfully, re-run this $action."
    echo "[$action] State/profiles/credentials from that installation may need to be recreated unless you back them up first — this release does not promise automatic old-state migration."
    echo "[$action] No changes were made to this host by this run."
  } >&2
  return 1
}
