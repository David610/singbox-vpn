#!/usr/bin/env bash
# recover-interrupted-update.sh may delete ONLY directories that the update
# marker names AND that pass strict validation (direct child of /opt, exact name
# pattern, real directory, not a symlink, root-owned, already normalized). Any
# doubt aborts with nothing deleted. It also must default to a dry run, refuse on
# an unhealthy host, archive the marker, and be idempotent. Stubs only.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
R="$REPO_ROOT/deploy/lib/recover-interrupted-update.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/opt" "$T/state" "$T/victim/keep"
printf '#!/bin/sh\nexit 0\n' >"$T/bin/systemctl"; chmod +x "$T/bin/systemctl"
echo canary >"$T/victim/keep/canary"; echo canary >"$T/canary-cwd"
export PATH="$T/bin:$PATH" RECOVER_MARKER="$T/state/update-transaction.json" RECOVER_OPT_DIR="$T/opt" RECOVER_ARCHIVE_DIR="$T/state" RECOVER_HEALTH_CMD="echo [OK] fine"
P="$T/opt/.singbox-vpn-prev-1"; S="$T/opt/.singbox-vpn-update-staging.abc"
reset() { rm -rf "$T/opt" "$T/state"; mkdir -p "$T/opt/singbox-vpn" "$T/state"; }
marker() { printf '{"prev_opt_dir":%s,"staged_src_dir":%s}' "$1" "$2" >"$RECOVER_MARKER"; }
intact() { [ -f "$T/victim/keep/canary" ] && [ -f "$T/canary-cwd" ] && [ -d "$T/opt/singbox-vpn" ]; }
refuses() { # $1=label ; expects nonzero exit, marker kept, canaries intact
  (cd "$T" && bash "$R" --yes >"$T/out" 2>&1); rc=$?
  if [ "$rc" -ne 0 ] && [ -e "$RECOVER_MARKER" ] && intact; then ok "$1: refused, nothing deleted"; else bad "$1: rc=$rc marker=$([ -e "$RECOVER_MARKER" ] && echo kept || echo GONE) $(intact || echo CANARY-DAMAGED): $(head -2 "$T/out")"; fi
}

# ---- happy path -------------------------------------------------------------
reset; marker "\"$P\"" "\"$S/singbox-vpn-src\""; mkdir -p "$P" "$S/singbox-vpn-src"
bash "$R" >/dev/null 2>&1; { [ -e "$RECOVER_MARKER" ] && [ -d "$P" ]; } && ok "default is a dry run" || bad "dry run removed something"
bash "$R" --yes >/dev/null 2>&1
{ [ ! -e "$RECOVER_MARKER" ] && [ ! -d "$P" ] && [ ! -d "$S" ] && [ -d "$T/opt/singbox-vpn" ]; } && ok "--yes removes the marker and exactly the two stale dirs" || bad "--yes result wrong"
ls "$T/state"/interrupted-update-*.json >/dev/null 2>&1 && ok "marker archived" || bad "marker not archived"
bash "$R" --yes >/dev/null 2>&1; [ $? -eq 0 ] && ok "idempotent: second run is a clean no-op" || bad "second run failed"
reset; marker "\"$P\"" "\"$S/singbox-vpn-src\""   # dirs already gone
bash "$R" --yes >/dev/null 2>&1; { [ $? -eq 0 ] && [ ! -e "$RECOVER_MARKER" ]; } && ok "named dirs already absent: marker cleared, exit 0" || bad "absent-dirs case failed"

# ---- health gates -------------------------------------------------------------
reset; marker "\"$P\"" "\"\""; mkdir -p "$P"
printf '#!/bin/sh\necho "[FAIL] x"\n' >"$T/bin/badhealth"; chmod +x "$T/bin/badhealth"
RECOVER_HEALTH_CMD="$T/bin/badhealth" bash "$R" --yes >/dev/null 2>&1; { [ $? -ne 0 ] && [ -e "$RECOVER_MARKER" ] && [ -d "$P" ]; } && ok "doctor FAIL: refused" || bad "ran despite doctor FAIL"
printf '#!/bin/sh\n[ "$3" = sing-box ] && exit 3 || exit 0\n' >"$T/bin/systemctl"
bash "$R" --yes >/dev/null 2>&1; { [ $? -ne 0 ] && [ -e "$RECOVER_MARKER" ] && [ -d "$P" ]; } && ok "inactive service: refused" || bad "ran despite inactive service"
printf '#!/bin/sh\nexit 0\n' >"$T/bin/systemctl"

# ---- adversarial markers: every one must delete NOTHING ----------------------------
reset; ln -s "$T/victim" "$P"; marker "\"$P\"" "\"\""
refuses "prev_opt_dir is a correctly named SYMLINK to an outside dir"
reset; ln -s "$T/opt/singbox-vpn" "$P"; marker "\"$P\"" "\"\""
refuses "prev_opt_dir is a correctly named symlink to the live source tree"
reset; mkdir -p "$S"; rmdir "$S"; ln -s "$T/victim" "$S"; marker "\"\"" "\"$S/singbox-vpn-src\""
refuses "staging dir is a symlink (staged_src_dir would resolve into it)"
reset; mkdir -p "$P"; marker "\"$P/../../victim\"" "\"\""
refuses "path traversal with .. in prev_opt_dir"
reset; marker "\"$T/opt/.singbox-vpn-prev-1/../singbox-vpn\"" "\"\""
refuses "traversal that lands on the live /opt/singbox-vpn"
reset; mkdir -p "$T/victim/.singbox-vpn-prev-1"; marker "\"$T/victim/.singbox-vpn-prev-1\"" "\"\""
refuses "valid name but wrong parent directory"
reset; mkdir -p "$T/opt/sub/.singbox-vpn-prev-1"; marker "\"$T/opt/sub/.singbox-vpn-prev-1\"" "\"\""
refuses "valid name two levels below /opt"
reset; mkdir -p "$T/opt/not-a-stale-dir"; marker "\"$T/opt/not-a-stale-dir\"" "\"\""
refuses "unexpected directory name"
reset; mkdir -p "$T/opt/.singbox-vpn-prev-1x/../victim" 2>/dev/null; marker "\"$T/opt/.singbox-vpn-prev-1x/../victim\"" "\"\""
refuses "name tricks with dot-dot after a valid prefix"
reset; : >"$P"; marker "\"$P\"" "\"\""
refuses "prev_opt_dir is a regular file"
reset; mkdir -p "$T/opt/$(printf '.singbox-vpn-prev-1\nrel')"; marker "$(python3 -c "import json,sys;print(json.dumps(sys.argv[1]))" "$T/opt/$(printf '.singbox-vpn-prev-1\nrel')")" "\"\""
refuses "newline in the directory name (could smuggle a relative path into rm)"
reset; marker "\"relative/.singbox-vpn-prev-1\"" "\"\""
refuses "relative path"
reset; mkdir -p "$P"; marker "[\"$P\"]" "\"\""
refuses "non-string field (list)"
reset; mkdir -p "$P"; marker "123" "\"\""
refuses "non-string field (number)"
reset; mkdir -p "$P"; printf 'this is not json' >"$RECOVER_MARKER"
refuses "malformed JSON marker"
reset; mkdir -p "$P"; printf '[1,2,3]' >"$RECOVER_MARKER"
refuses "marker is not a JSON object"
reset; mkdir -p "$P"; marker "\"$P\"" "\"\""; RECOVER_EXPECT_UID=424242 refuses "directory not owned by the expected uid"
reset; mkdir -p "$P"; marker "\"$P\"" "\"\""; mv "$RECOVER_MARKER" "$T/real-marker"; ln -s "$T/real-marker" "$RECOVER_MARKER"
(cd "$T" && bash "$R" --yes >/dev/null 2>&1); { [ $? -ne 0 ] && [ -d "$P" ]; } && ok "marker that is itself a symlink: refused" || bad "followed a symlinked marker"
# all-or-nothing: one valid dir + one invalid must delete neither
reset; mkdir -p "$P" "$T/victim2"; ln -s "$T/victim2" "$S"; marker "\"$P\"" "\"$S/singbox-vpn-src\""
(cd "$T" && bash "$R" --yes >/dev/null 2>&1); { [ $? -ne 0 ] && [ -d "$P" ] && [ -d "$T/victim2" ] && [ -e "$RECOVER_MARKER" ]; } && ok "all-or-nothing: a valid dir is NOT deleted when another entry is invalid" || bad "partial deletion happened"
# a symlink INSIDE a valid stale dir must not be followed by rm
reset; mkdir -p "$P"; ln -s "$T/victim" "$P/escape"; marker "\"$P\"" "\"\""
(cd "$T" && bash "$R" --yes >/dev/null 2>&1); { [ ! -d "$P" ] && intact; } && ok "symlink inside a stale dir is removed, its target untouched" || bad "rm followed a symlink inside the stale dir"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
