#!/usr/bin/env bash
# Local release qualification with honest results.
#
# Runs the repository's own gates in order, keeps the COMPLETE log of each, and
# writes one summary that distinguishes PASS / FAIL / NOT-RUN. Design rules:
#   * every command's real exit status is captured (no `| tail` hiding a
#     failure: output goes to a file first, the status is read directly);
#   * a gate whose prerequisite is missing is NOT-RUN, never PASS, and makes the
#     overall verdict FAIL unless --allow-not-run is given;
#   * the exact commit (and whether the tree is clean) and tool versions are
#     recorded next to the results, and each log's SHA-256 is listed.
#
#   bash deploy/lib/qualify-local.sh --out /path/to/evidence-dir \
#        [--sing-box /path/to/pinned/sing-box] [--python /path/to/venv/python] [--allow-not-run]
#
# From an exported tree (no .git) set QUALIFY_COMMIT=<full sha> so the evidence names the exact commit.
# Run it as root on a Linux host (the privileged fixtures and the two-hop system
# tests need root/CAP_NET_ADMIN) and from an exported or git checkout of the exact
# commit under test. It changes nothing outside --out and the build directory.
set -uo pipefail

OUT=""; SING_BOX=""; PYTHON_BIN=""; ALLOW_NOT_RUN=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --sing-box) SING_BOX="$2"; shift 2 ;;
    --python) PYTHON_BIN="$2"; shift 2 ;;
    --allow-not-run) ALLOW_NOT_RUN=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$OUT" ] || { echo "--out DIR is required" >&2; exit 2; }
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2
mkdir -p "$OUT"
[ -z "$(ls -A "$OUT" 2>/dev/null)" ] || { echo "refusing to reuse a non-empty --out directory" >&2; exit 2; }

SUMMARY="$OUT/SUMMARY.tsv"
printf 'gate\tresult\texit\tseconds\tlog_sha256\n' >"$SUMMARY"
{
  echo "commit: ${QUALIFY_COMMIT:-$(git rev-parse HEAD 2>/dev/null || echo 'UNKNOWN (exported tree: set QUALIFY_COMMIT)')}"
  echo "tree_clean: $(if [ -n "$(git status --porcelain 2>/dev/null)" ]; then echo no; else echo yes-or-not-git; fi)"
  echo "workspace_version: $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1 || true)"
  echo "date_utc: $(date -u +%FT%TZ)"
  echo "uid: $(id -u)"
  echo "os: $(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")"
  echo "rustc: $(rustc --version 2>&1)"
  echo "cargo: $(cargo --version 2>&1)"
  echo "sing-box: ${SING_BOX:+$("$SING_BOX" version 2>&1 | head -1 || true)}"
  echo "pinned sing-box: $(sed -n 's/^SINGBOX_VERSION=//p' deploy/lib/versions.env)"
} >"$OUT/ENVIRONMENT.txt"

# I-81: one shared, bounded Cargo target and a disk/inode budget before any build;
# the build lock is held for the whole run so a concurrent prune cannot remove it.
log() { echo "[qualify] $*"; }
warn() { echo "[qualify] WARN: $*" >&2; }
die() { echo "[qualify] $*" >&2; exit 2; }
# shellcheck source=deploy/lib/preflight.sh
. "$ROOT/deploy/lib/preflight.sh"
preflight_cargo_build_env || die "cannot prepare the shared build cache $SINGBOX_VPN_BUILD_CACHE"
preflight_check_build_budget "$SINGBOX_VPN_BUILD_CACHE" || die "insufficient disk headroom; nothing was built"
exec {QUALIFY_LOCK_FD}>>"$SINGBOX_VPN_BUILD_CACHE/.lock"
flock -n "$QUALIFY_LOCK_FD" || die "another build holds $SINGBOX_VPN_BUILD_CACHE/.lock"
echo "cargo_target_dir: $CARGO_TARGET_DIR (jobs $CARGO_BUILD_JOBS)" >>"$OUT/ENVIRONMENT.txt"
trap 'echo "target_dir_mib: $(du -sm "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)" >>"$OUT/ENVIRONMENT.txt"' EXIT

overall=0
record() { # name result exit seconds logfile
  local sha="-"
  [ -f "$5" ] && sha="$(sha256sum "$5" | cut -d' ' -f1)"
  printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$sha" >>"$SUMMARY"
  printf '%-34s %-8s (exit %s, %ss)\n' "$1" "$2" "$3" "$4"
}
gate() { # gate name -- command...   (runs the command, status taken directly)
  local name="$1"; shift; shift
  local log="$OUT/$name.log" start rc
  start="$(date +%s)"
  "$@" >"$log" 2>&1; rc=$?
  if [ "$rc" -eq 0 ]; then record "$name" PASS "$rc" "$(( $(date +%s) - start ))" "$log"
  else record "$name" FAIL "$rc" "$(( $(date +%s) - start ))" "$log"; overall=1; fi
}
not_run() { # name reason
  echo "$2" >"$OUT/$1.log"
  record "$1" NOT-RUN "-" 0 "$OUT/$1.log"
  [ "$ALLOW_NOT_RUN" -eq 1 ] || overall=1
}

gate rustfmt -- cargo fmt --all -- --check
gate clippy -- cargo clippy --locked --workspace --all-targets -- -D warnings
gate rustdoc -- env RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --no-deps

if [ -n "$SING_BOX" ] && [ -x "$SING_BOX" ] && [ -n "$PYTHON_BIN" ] && [ -x "$PYTHON_BIN" ] && [ "$(id -u)" -eq 0 ]; then
  gate cargo-test-workspace -- env SING_BOX_BIN="$SING_BOX" SINGBOX_VPN_REQUIRE_REAL_INTEROP=1 \
    YOUTUBE_TEST_PYTHON="$PYTHON_BIN" SINGBOX_VPN_LOCK_PATH="$OUT/test-state.lock" \
    cargo test --locked --workspace --all-targets --no-fail-fast
  # a run that executed no tests at all is not a pass, whatever cargo's exit status says
  passed="$(grep -E '^test result: ok' "$OUT/cargo-test-workspace.log" | sed -E 's/.* ([0-9]+) passed.*/\1/' | awk '{s+=$1} END {print s+0}')"
  suites="$(grep -cE '^test result:' "$OUT/cargo-test-workspace.log" || true)"
  echo "tests passed: ${passed:-0} in $suites suites" >>"$OUT/ENVIRONMENT.txt"
  if [ "${passed:-0}" -eq 0 ]; then echo "no test executed" >>"$OUT/cargo-test-workspace.log"; overall=1; fi
else
  not_run cargo-test-workspace "needs --sing-box (pinned binary), --python (venv with aioquic) and root; refusing to substitute a run with skipped real-core tests"
fi

if [ "$(id -u)" -eq 0 ]; then
  # Several fixtures source install.sh, whose paths are fixed (/etc/vpn, /opt/singbox-vpn, /var/lib/singbox-vpn): on a host that
  # has an installation they read the LIVE deployment and fail for reasons that are not defects (I-48). On such a host run each
  # one through the clean-host wrapper (private mount namespace, nothing on the host is touched); a retry-pass is a FAIL (exit 3).
  if [ -e /etc/vpn/deployment.toml ] || [ -d /opt/singbox-vpn ] || [ -d /var/lib/singbox-vpn ]; then
    echo "fixtures: installed host detected, running each through deploy/lib/run-shell-tests-clean-host.sh" >>"$OUT/ENVIRONMENT.txt"
    for t in deploy/lib/tests/*.sh; do gate "fixture-$(basename "$t" .sh)" -- bash deploy/lib/run-shell-tests-clean-host.sh "$t"; done
  else
    echo "fixtures: clean host, run directly" >>"$OUT/ENVIRONMENT.txt"
    for t in deploy/lib/tests/*.sh; do gate "fixture-$(basename "$t" .sh)" -- bash "$t"; done
  fi
else
  not_run shell-fixtures "the privileged fixtures need root"
fi
for f in deploy/lib/tests/test_*.py; do
  gate "python-$(basename "$f" .py)" -- bash -c 'cd deploy/lib/tests && python3 -m unittest "$1"' _ "$(basename "$f" .py)"
done
if [ -n "$PYTHON_BIN" ] && [ -x "$PYTHON_BIN" ]; then
  gate egress-tools-python -- "$PYTHON_BIN" deploy/lib/tests/test-egress-tools.py
else
  not_run egress-tools-python "needs --python"
fi
if command -v shellcheck >/dev/null 2>&1; then
  gate shellcheck -- shellcheck -S warning install.sh uninstall.sh bin/singbox-vpn-uninstall deploy/lib/*.sh deploy/almalinux/*.sh deploy/lib/tests/*.sh
else
  not_run shellcheck "shellcheck is not installed"
fi

echo
if [ "$overall" -eq 0 ]; then echo "QUALIFICATION: PASS"; else echo "QUALIFICATION: FAIL (see $SUMMARY)"; fi
echo "QUALIFICATION: $([ "$overall" -eq 0 ] && echo PASS || echo FAIL)" >>"$SUMMARY"
exit "$overall"
