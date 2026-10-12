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
  echo "commit: $(git rev-parse HEAD 2>/dev/null || echo 'not a git checkout (exported tree)')"
  echo "tree_clean: $(if [ -n "$(git status --porcelain 2>/dev/null)" ]; then echo no; else echo yes-or-not-git; fi)"
  echo "workspace_version: $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
  echo "date_utc: $(date -u +%FT%TZ)"
  echo "uid: $(id -u)"
  echo "os: $(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")"
  echo "rustc: $(rustc --version 2>&1)"
  echo "cargo: $(cargo --version 2>&1)"
  echo "sing-box: ${SING_BOX:+$("$SING_BOX" version 2>&1 | head -1)}"
  echo "pinned sing-box: $(sed -n 's/^SINGBOX_VERSION=//p' deploy/lib/versions.env)"
} >"$OUT/ENVIRONMENT.txt"

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
  for t in deploy/lib/tests/*.sh; do gate "fixture-$(basename "$t" .sh)" -- bash "$t"; done
else
  not_run shell-fixtures "the privileged fixtures need root"
fi
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
