#!/usr/bin/env bash
# recover-interrupted-update.sh must clear an interrupted-update marker only on
# a healthy host, only delete the stale paths the marker names AND that match
# the expected pattern, archive the marker, and default to a dry run.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
R="$REPO_ROOT/deploy/lib/recover-interrupted-update.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/opt" "$T/state"
printf '#!/bin/sh\nexit 0\n' >"$T/bin/systemctl"; chmod +x "$T/bin/systemctl"
export PATH="$T/bin:$PATH" RECOVER_MARKER="$T/state/update-transaction.json" RECOVER_OPT_DIR="$T/opt" RECOVER_ARCHIVE_DIR="$T/state" RECOVER_HEALTH_CMD="echo [OK] fine"
mkmarker() {
  mkdir -p "$T/opt/.singbox-vpn-prev-1" "$T/opt/.singbox-vpn-update-staging.abc/singbox-vpn-src" "$T/opt/singbox-vpn"
  printf '{"prev_opt_dir":"%s","staged_src_dir":"%s","backup_dir":"/etc/vpn/backups/x"}' "$T/opt/.singbox-vpn-prev-1" "$T/opt/.singbox-vpn-update-staging.abc/singbox-vpn-src" >"$RECOVER_MARKER"
}

bash "$R" >/dev/null 2>&1; [ $? -eq 0 ] && ok "no marker: nothing to do, exit 0" || bad "no-marker case failed"

mkmarker
bash "$R" >/dev/null 2>&1
{ [ -e "$RECOVER_MARKER" ] && [ -d "$T/opt/.singbox-vpn-prev-1" ]; } && ok "default is a dry run: nothing removed" || bad "dry run removed something"

bash "$R" --yes >/dev/null 2>&1
if [ ! -e "$RECOVER_MARKER" ] && [ ! -d "$T/opt/.singbox-vpn-prev-1" ] && [ ! -d "$T/opt/.singbox-vpn-update-staging.abc" ] && [ -d "$T/opt/singbox-vpn" ]; then
  ok "--yes removes the marker and exactly the stale dirs, keeps /opt/singbox-vpn"
else bad "--yes result wrong"; fi
ls "$T/state"/interrupted-update-*.json >/dev/null 2>&1 && ok "marker archived" || bad "marker not archived"

printf '#!/bin/sh\necho "[FAIL] x"\n' >"$T/bin/badhealth"; chmod +x "$T/bin/badhealth"
mkmarker; RECOVER_HEALTH_CMD="$T/bin/badhealth" bash "$R" --yes >/dev/null 2>&1
{ [ $? -ne 0 ] && [ -e "$RECOVER_MARKER" ]; } && ok "doctor FAIL: refuses and changes nothing" || bad "ran despite a failing doctor"

mkmarker; printf '#!/bin/sh\n[ "$3" = sing-box ] && exit 3 || exit 0\n' >"$T/bin/systemctl"
bash "$R" --yes >/dev/null 2>&1
{ [ $? -ne 0 ] && [ -e "$RECOVER_MARKER" ] && [ -d "$T/opt/.singbox-vpn-prev-1" ]; } && ok "inactive service: refuses and changes nothing" || bad "ran despite an inactive service"
printf '#!/bin/sh\nexit 0\n' >"$T/bin/systemctl"

printf '{"prev_opt_dir":"%s/etc","staged_src_dir":""}' "$T" >"$RECOVER_MARKER"; mkdir -p "$T/etc"
bash "$R" --yes >/dev/null 2>&1
{ [ $? -ne 0 ] && [ -d "$T/etc" ] && [ -e "$RECOVER_MARKER" ]; } && ok "tampered marker naming another path: refuses, deletes nothing" || bad "deleted a path outside the pattern"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
