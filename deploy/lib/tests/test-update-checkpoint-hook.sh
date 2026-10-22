#!/usr/bin/env bash
# The updater's TEST-ONLY kill-point hook must be inert unless explicitly and safely enabled (it blocks the updater!).
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; UPD="$HERE/../../almalinux/update.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
F=0; ok() { echo "ok   $1"; }; bad() { echo "FAIL $1"; F=$((F + 1)); }
fn="$(awk '/^checkpoint\(\) \{/{p=1} p{print} p&&/^}/{exit}' "$UPD")"
[ -n "$fn" ] || { echo "FAIL: checkpoint() not found in update.sh"; exit 1; }
run() { ( eval "$fn"; sleep() { :; }; "$@" ) ; }
# 1. no env: returns immediately, writes nothing
mkdir "$T/d" && chmod 0700 "$T/d"
( unset SINGBOX_VPN_TEST_CHECKPOINT SINGBOX_VPN_TEST_CHECKPOINT_DIR; eval "$fn"; checkpoint "SWITCHING to x" ); [ ! -e "$T/d/reached" ] && ok "inert without env" || bad "inert without env"
# 2. env set but the directory is unsafe (mode 0755 / symlink): inert
chmod 0755 "$T/d"; ( export SINGBOX_VPN_TEST_CHECKPOINT="SWITCH" SINGBOX_VPN_TEST_CHECKPOINT_DIR="$T/d"; eval "$fn"; sleep() { :; }; checkpoint "SWITCHING to x" ); [ ! -e "$T/d/reached" ] && ok "inert for a non-0700 directory" || bad "inert for a non-0700 directory"
chmod 0700 "$T/d"; ln -s "$T/d" "$T/l"; ( export SINGBOX_VPN_TEST_CHECKPOINT="SWITCH" SINGBOX_VPN_TEST_CHECKPOINT_DIR="$T/l"; eval "$fn"; sleep() { :; }; checkpoint "SWITCHING to x" ); [ ! -e "$T/d/reached" ] && ok "inert for a symlinked directory" || bad "inert for a symlink"
# 3. different checkpoint: inert
( export SINGBOX_VPN_TEST_CHECKPOINT="@manifest-renamed" SINGBOX_VPN_TEST_CHECKPOINT_DIR="$T/d"; eval "$fn"; sleep() { :; }; checkpoint "SWITCHING to x" ); [ ! -e "$T/d/reached" ] && ok "inert for another checkpoint" || bad "other checkpoint"
# 4. matching prefix + safe dir: records the checkpoint (sleep stubbed so the test does not block)
( export SINGBOX_VPN_TEST_CHECKPOINT="SWITCH" SINGBOX_VPN_TEST_CHECKPOINT_DIR="$T/d"; eval "$fn"; sleep() { :; }; checkpoint "SWITCHING to x" ); [ "$(cat "$T/d/reached" 2>/dev/null)" = "SWITCHING to x" ] && ok "records a matching checkpoint" || bad "records a matching checkpoint"
# 5. every checkpoint the harness requests exists in the updater, in the order it runs
while IFS= read -r m; do
  case "$m" in @*) grep -q "checkpoint \"$m\"" "$UPD" && ok "explicit checkpoint $m is present" || bad "explicit checkpoint $m missing";;
  *) grep -qF "log \"$m" "$UPD" && ok "log checkpoint '$m' is present" || bad "log checkpoint '$m' missing";; esac
done < <(sed -n 's/^MARKERS=(//p' "$HERE/../interrupt_matrix.sh" | grep -o '"[^"]*"' | tr -d '"')
exit $F
