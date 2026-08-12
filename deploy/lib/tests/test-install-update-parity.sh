#!/usr/bin/env bash
# Static drift detector: asserts install.sh and update.sh install/sync
# the same set of systemd units and $BIN_DIR helper scripts. Does not
# run either script (both need root/systemd/a real build) — it parses
# the two scripts' own source for the specific patterns each already
# uses to install these assets, and fails loudly the moment they
# diverge, rather than relying on a human to notice a future asset
# added to one script but not the other (exactly the class of bug this
# check exists to catch — see docs/PERFORMANCE_OPTIMIZATION_PLAN.md's
# "update path" section).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"

failures=0
assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $desc"
    echo "  install.sh: $expected"
    echo "  update.sh:  $actual"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

# --- systemd units ---
# install.sh: install_systemd_units() installs each unit with a literal
# `install -m 0644 ".../systemd/<name>" /etc/systemd/system/<name>` line.
install_units="$(grep -oE 'systemd/[A-Za-z0-9_.-]+\.(service|timer)"' "$INSTALL_SH" \
  | sed -E 's#systemd/##; s/"$//' | sort -u)"
# update.sh: SYSTEMD_UNITS=(...) array.
update_units="$(sed -n '/^SYSTEMD_UNITS=(/,/)/p' "$UPDATE_SH" \
  | sed -E 's/^SYSTEMD_UNITS=\(//' | tr -d '()' | tr ' ' '\n' \
  | grep -E '\.(service|timer)$' | sort -u)"
assert_eq "systemd units installed by install.sh match units synced by update.sh" \
  "$install_units" "$update_units"

# update.sh must actually reload systemd after touching unit files —
# a unit file change with no daemon-reload never takes effect.
if grep -q '^systemctl daemon-reload$' "$UPDATE_SH"; then
  echo "ok: update.sh runs 'systemctl daemon-reload' after installing units"
else
  echo "FAIL: update.sh does not run 'systemctl daemon-reload' — unit-file changes would silently not take effect"
  failures=$((failures + 1))
fi

# --- $BIN_DIR helper scripts ---
# install.sh: literal `install -m 0<mode> "$REPO_ROOT/<src>" "$BIN_DIR/<name>"` lines
# for the non-core-binary helper scripts (vpn-health-check, vpn-benchmark,
# vpn-benchmark-lib.sh). Core binaries (vpn-admin/vpn/vpn-subscription-svc)
# are intentionally excluded from this comparison — they're Cargo build
# artifacts staged via `target/release/...`, a different (and already
# parity-checked-by-construction, since both scripts literally build and
# install the same three) mechanism.
# sing-box itself is a pinned vendor binary managed by its own
# installer function/version pin (SINGBOX_VERSION), not part of this
# project's own update.sh flow at all (updating it means bumping
# SINGBOX_VERSION and re-running install.sh) — excluded from this
# comparison on purpose, not an oversight.
install_helpers="$(grep -oE '\$BIN_DIR/[A-Za-z0-9_.-]+"' "$INSTALL_SH" \
  | sed -E 's#\$BIN_DIR/##; s/"$//' \
  | grep -vE '^(vpn-admin|vpn|vpn-subscription-svc|sing-box|sing-box\.LICENSE)$' | sort -u)"
update_helpers="$(grep -oE '"\$BIN_DIR/[A-Za-z0-9_.-]+\.update-new"' "$UPDATE_SH" \
  | sed -E 's#"\$BIN_DIR/##; s/\.update-new"$//' \
  | grep -vE '^(vpn-admin|vpn|vpn-subscription-svc)$' | sort -u)"
assert_eq "helper scripts installed by install.sh match helper scripts synced by update.sh" \
  "$install_helpers" "$update_helpers"

echo
if [ "$failures" -eq 0 ]; then
  echo "install.sh / update.sh asset parity: OK"
  exit 0
else
  echo "$failures parity check(s) FAILED — fresh install and update would produce different runtime state"
  exit 1
fi
