#!/usr/bin/env bash
# shellcheck disable=SC2034  # variables are consumed by eval-ed installer functions
# I-33: the installer (bootstrap + real installer) must not leak scratch data (the pinned cosign alone is ~135 MB) and
# must not download cosign twice. Hermetic: temp dirs only, stubs for curl/sha256, no host state.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; ROOT="$HERE/../../.."
BOOT="$ROOT/install.sh"; INNER="$ROOT/deploy/almalinux/install.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
F=0; ok() { echo "ok   $1"; }; bad() { echo "FAIL $1"; F=$((F + 1)); }
fn_of() { awk -v n="$2" '$0 ~ "^"n"\\(\\) \\{" {p=1} p{print} p&&/^}/{exit}' "$1"; }

for S in "$BOOT" "$INNER"; do
  name="$(basename "$(dirname "$S")")/$(basename "$S")"
  sw="$(fn_of "$S" sweep_stale_scratch)"; [ -n "$sw" ] || { bad "$name: sweep_stale_scratch missing"; continue; }
  P="$T/p-$RANDOM"; mkdir -p "$P"
  mk() { mkdir "$P/$1"; [ -z "${2:-}" ] || echo "$2" >"$P/$1/.owner-pid"; touch -d '2 hours ago' "$P/$1"; }
  sleep 300 & live=$!; dead=$(sh -c 'echo $$'); 
  mk singbox-vpn-x.dead "$dead"; mk singbox-vpn-x.live "$live"; mk singbox-vpn-x.noowner ""; mk singbox-vpn-x.badpid "abc"; mk other.dead "$dead"
  mkdir "$P/singbox-vpn-x.young"; echo "$dead" >"$P/singbox-vpn-x.young/.owner-pid"
  mkdir -p "$T/target"; ln -s "$T/target" "$P/singbox-vpn-x.link"
  ( eval "$sw"; sweep_stale_scratch "$P" 'singbox-vpn-x.*' )
  [ ! -e "$P/singbox-vpn-x.dead" ] && ok "$name: stale workspace of a dead owner removed" || bad "$name: stale workspace not removed"
  [ -d "$P/singbox-vpn-x.live" ] && ok "$name: workspace of a live owner kept" || bad "$name: removed a live owner's workspace"
  [ -d "$P/singbox-vpn-x.noowner" ] && [ -d "$P/singbox-vpn-x.badpid" ] && ok "$name: workspaces without a valid owner pid kept" || bad "$name: removed an unowned workspace"
  [ -d "$P/singbox-vpn-x.young" ] && ok "$name: young workspace kept (pid-reuse/race guard)" || bad "$name: removed a young workspace"
  [ -d "$P/other.dead" ] && ok "$name: non-matching name untouched" || bad "$name: removed a non-matching directory"
  [ -L "$P/singbox-vpn-x.link" ] && [ -d "$T/target" ] && ok "$name: symlink not followed" || bad "$name: followed or removed a symlink"
  kill "$live" 2>/dev/null; wait "$live" 2>/dev/null
done

# the real installer routes EVERY bare mktemp into one workspace that an EXIT trap removes
l_tmpdir="$(grep -n '^export TMPDIR="\$INSTALL_WORK"' "$INNER" | head -1 | cut -d: -f1)"
l_first="$(grep -n 'mktemp' "$INNER" | grep -v 'INSTALL_WORK=\|^.*#' | head -1 | cut -d: -f1)"
[ -n "$l_tmpdir" ] && [ -n "$l_first" ] && [ "$l_tmpdir" -lt "$l_first" ] && ok "TMPDIR=workspace is set before the first mktemp" || bad "TMPDIR workspace not set before the first mktemp ($l_tmpdir vs $l_first)"
# (I-74: installed via install_scratch_exit_trap, which keeps a sourcing caller's own EXIT trap;
# behaviour is covered by test-installer-sourcing-keeps-exit-trap.sh)
grep -q '^install_scratch_exit_trap$' "$INNER" && grep -q '    trap cleanup_scratch EXIT' "$INNER" \
  && grep -q 'trap "cleanup_scratch; \$prev" EXIT' "$INNER" \
  && ok "EXIT trap removes the workspace" || bad "no EXIT trap for the workspace"
grep -n 'mktemp' "$INNER" | grep -E 'mktemp[^)]*/tmp/' | grep -v 'singbox-vpn-inner' && bad "a mktemp bypasses the workspace" || ok "no mktemp bypasses the workspace"

# behaviour: the EXIT trap really removes the workspace (success and failure) -- run the same two lines in a subshell
for rc in 0 3; do
  ( W="$(mktemp -d "$T/w.XXXX")"; echo data >"$W/cosign"; INSTALL_WORK="$W"; eval "$(fn_of "$INNER" cleanup_scratch)"; trap cleanup_scratch EXIT; echo "$W" >"$T/wdir"; exit $rc ); r=$?
  [ "$r" = "$rc" ] && [ ! -e "$(cat "$T/wdir")" ] && ok "workspace removed and exit status $rc preserved" || bad "workspace/exit status wrong for rc=$rc"
done

# cosign hand-over: reuse only when the digest matches THIS script's pin; otherwise fall back to a download
ec="$(fn_of "$INNER" ensure_cosign)"; [ -n "$ec" ] || bad "ensure_cosign missing"
run_ec() { # $1 = handed file content ; prints "reused" | "download"
  ( cd "$T" && INSTALL_WORK="$(mktemp -d "$T/iw.XXXX")" && export TMPDIR="$INSTALL_WORK" && ARCH=amd64 COSIGN_VERSION=0 CURL_NET_FLAGS=() COSIGN_BIN="" COSIGN_SHA256_AMD64="$2"
    log() { :; }; warn() { :; }; die() { echo "download"; exit 0; }
    command() { return 1; }; curl() { echo download; exit 0; }
    printf '%s' "$1" >"$T/handed"; chmod 700 "$T/handed"; SINGBOX_VPN_COSIGN_BIN="$T/handed"
    eval "$ec"; ensure_cosign; echo reused )
}
good="$(printf 'cosign-bytes' | sha256sum | awk '{print $1}')"
[ "$(run_ec 'cosign-bytes' "$good")" = reused ] && ok "bootstrap cosign reused when its digest matches" || bad "matching cosign not reused"
[ "$(run_ec 'tampered' "$good")" = download ] && ok "bootstrap cosign with a different digest is NOT trusted" || bad "tampered cosign was reused"
exit $F
