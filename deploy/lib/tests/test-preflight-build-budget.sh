#!/usr/bin/env bash
# I-81 (reconstructed 2026-10-22; the original 3d7c005 is MISSING): disk and
# inode budget before a build, one shared bounded Cargo target, and a scoped,
# dry-run-by-default prune of abandoned build output. Stubs `df`; works in a
# private temp dir; no root, no real build.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
log() { :; }
warn() { :; }
die() { echo "[test-die] $*" >&2; return 1; }
# shellcheck source=deploy/lib/preflight.sh
. "$REPO_ROOT/deploy/lib/preflight.sh"

failures=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; failures=$((failures + 1)); }
expect_rc() { # desc expected_rc command...
  local desc="$1" want="$2" rc=0; shift 2
  "$@" >/dev/null 2>&1 || rc=$?
  if [ "$rc" -eq "$want" ]; then ok "$desc"; else bad "$desc (rc=$rc, want $want)"; fi
}

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

# df stub: FAKE_AVAIL_MIB / FAKE_IFREE drive the answers.
df() {
  case "$1" in
    -*i*) printf 'Filesystem Inodes IUsed IFree IUse%% Mounted\n/dev/x 1000000 1 %s 1%% /\n' "$FAKE_IFREE" ;;
    *) printf 'Filesystem 1M-blocks Used Available Capacity Mounted\n/dev/x 40000 1 %s 1%% /\n' "$FAKE_AVAIL_MIB" ;;
  esac
}

echo "--- budget ---"
FAKE_IFREE=500000
FAKE_AVAIL_MIB=30000
expect_rc "enough space permits the build" 0 preflight_check_build_budget "$T" 10240 8192
FAKE_AVAIL_MIB=18000
expect_rc "space below reserve+estimate blocks the build" 1 preflight_check_build_budget "$T" 10240 8192
FAKE_AVAIL_MIB=30000; FAKE_IFREE=100
expect_rc "inode exhaustion blocks the build" 1 preflight_check_build_budget "$T" 10240 8192
FAKE_IFREE=500000; FAKE_AVAIL_MIB=""
expect_rc "unknown free space fails closed" 1 preflight_check_build_budget "$T" 10240 8192
FAKE_AVAIL_MIB=30000
unset -f df

echo "--- installer/updater source-build guard ---"
FAKE_IFREE=500000; FAKE_AVAIL_MIB=30000
(
  df() { case "$1" in -*i*) printf 'F I U %s\n/dev/x 1 1 %s 1%% /\n' x "$FAKE_IFREE" ;; *) printf 'F 1M U A\n/dev/x 40000 1 %s 1%% /\n' "$FAKE_AVAIL_MIB" ;; esac; }
  unset CARGO_BUILD_JOBS
  preflight_source_build_guard "$T" && [ "${CARGO_BUILD_JOBS:-0}" -ge 1 ] && [ "$CARGO_BUILD_JOBS" -le "$(nproc)" ]
) && ok "guard passes with headroom and bounds CARGO_BUILD_JOBS" || bad "guard with headroom"
(
  df() { case "$1" in -*i*) printf 'F I U %s\n/dev/x 1 1 %s 1%% /\n' x 500000 ;; *) printf 'F 1M U A\n/dev/x 40000 1 %s 1%% /\n' 9000 ;; esac; }
  preflight_source_build_guard "$T"
) >/dev/null 2>&1 && bad "guard allowed a build without headroom" || ok "guard refuses a source build without headroom"
for f in deploy/almalinux/install.sh deploy/almalinux/update.sh; do
  src="$(cat "$REPO_ROOT/$f")"
  n_cargo="$(grep -cE '^[^#]*cargo (build|test) ' <<<"$src" || true)"
  n_guard="$(grep -cE '^[^#]*preflight_source_build_guard ' <<<"$src" || true)"
  [ "$n_cargo" -ge 1 ] && [ "$n_guard" -ge "$n_cargo" ] && ok "$f guards each of its $n_cargo cargo build/test call(s)" || bad "$f: $n_cargo cargo call(s), $n_guard guard(s)"
done

echo "--- cargo env ---"
(
  SINGBOX_VPN_BUILD_CACHE="$T/cache"
  preflight_cargo_build_env
  [ "$CARGO_TARGET_DIR" = "$T/cache/target" ] && [ -f "$T/cache/.singbox-vpn-build-cache" ] || exit 1
  [ "$CARGO_BUILD_JOBS" -ge 1 ] && [ "$CARGO_BUILD_JOBS" -le "$(nproc)" ] || exit 1
) && ok "shared target dir under the managed cache, jobs bounded" || bad "cargo env"
expect_rc "cache root that is a symlink is refused" 1 bash -c '
  log(){ :; }; warn(){ :; }; die(){ return 1; }; . "$1/deploy/lib/preflight.sh"
  mkdir -p "$2/real"; ln -s "$2/real" "$2/link"; SINGBOX_VPN_BUILD_CACHE="$2/link"; preflight_cargo_build_env' _ "$REPO_ROOT" "$T"

echo "--- prune ---"
C="$T/prune"; mkdir -p "$C"; : >"$C/.singbox-vpn-build-cache"
mkdir -p "$C/target/debug" "$C/qualify-old1/debug" "$C/qualify-old2"
: >"$C/qualify-old1/.singbox-vpn-disposable"; : >"$C/qualify-old2/.singbox-vpn-disposable"
mkdir -p "$C/evidence"; echo keep >"$C/evidence/SUMMARY.tsv"            # no marker: never eligible
mkdir -p "$C/repo/.git"; : >"$C/repo/.singbox-vpn-disposable"           # marked but a git repo: never eligible
mkdir -p "$T/outside"; echo keep >"$T/outside/x"; : >"$T/outside/.singbox-vpn-disposable"
ln -s "$T/outside" "$C/qualify-escape"                                  # symlink out of scope

out="$(preflight_prune_build_cache "$C" 2>&1)"
[ -d "$C/qualify-old1" ] && [ -d "$C/qualify-old2" ] && ok "dry run deletes nothing" || bad "dry run deleted"
grep -q 'would remove .*qualify-old1' <<<"$out" && ok "dry run lists the disposable dirs" || bad "dry run listing: $out"

preflight_prune_build_cache "$C" --apply >/dev/null 2>&1 || bad "apply failed"
[ ! -e "$C/qualify-old1" ] && [ ! -e "$C/qualify-old2" ] && ok "marked abandoned output removed" || bad "not removed"
[ -f "$C/evidence/SUMMARY.tsv" ] && ok "unmarked evidence kept" || bad "evidence removed"
[ -d "$C/repo/.git" ] && ok "git repository kept even when marked" || bad "repo removed"
[ -f "$T/outside/x" ] && [ -L "$C/qualify-escape" ] && ok "symlink not followed out of scope" || bad "symlink escape"
[ -d "$C/target/debug" ] && ok "shared target kept by default" || bad "shared target removed"
preflight_prune_build_cache "$C" --apply >/dev/null 2>&1 && ok "repeated prune is safe" || bad "repeat prune failed"

expect_rc "prune refuses a directory that is not a managed cache" 1 preflight_prune_build_cache "$T/outside" --apply
[ -f "$T/outside/x" ] && ok "unmanaged dir untouched" || bad "unmanaged dir touched"
expect_rc "prune refuses /" 1 preflight_prune_build_cache / --apply

# active build: the cache lock is held -> --include-shared refuses, nothing removed
(
  exec 9>"$C/.lock"; flock 9
  rc=0; preflight_prune_build_cache "$C" --apply --include-shared >/dev/null 2>&1 || rc=$?
  [ "$rc" -ne 0 ] && [ -d "$C/target/debug" ]
) && ok "active build (lock held) protects the shared target" || bad "active build not protected"
preflight_prune_build_cache "$C" --apply --include-shared >/dev/null 2>&1
[ ! -e "$C/target" ] && ok "idle shared target removable only with --include-shared" || bad "include-shared"

echo
if [ "$failures" -ne 0 ]; then echo "$failures failure(s)"; exit 1; fi
echo "all preflight build-budget tests passed"
