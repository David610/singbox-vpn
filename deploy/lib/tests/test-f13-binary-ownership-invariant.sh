#!/usr/bin/env bash
# Static regression guard for F13 (safe uninstall ownership): the
# uninstaller must never remove /usr/local/bin/sing-box (or the
# provisioning agent binary) purely because the path exists — only when
# the ownership manifest says singbox-vpn itself put it there. This is a
# permanent, no-root, source-level check (existence-only removal of a
# shared system path is exactly the F13 regression this batch closes) —
# root-requiring end-to-end coverage of the same invariant (real
# pre-existing binary survives a real install+uninstall cycle) lives in
# test-uninstall-hardening.sh / test-uninstall-ownership-checkpoint2.sh /
# test-ambiguous-preexisting-residue.sh and runs under sudo in CI.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UNINSTALL_SH="$REPO_ROOT/deploy/almalinux/uninstall.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- sing-box binary removal is gated on SINGBOX_BIN_PRE_EXISTED, not mere existence ---"
# The removal line for /usr/local/bin/sing-box must appear inside a
# branch guarded by a SINGBOX_BIN_PRE_EXISTED check, and that guard must
# come from the ownership manifest (ownership_get), not a fresh stat/-e
# check invented at uninstall time (which would defeat the whole point:
# an attacker or an unrelated package could plant the file between
# install and uninstall and it would still be "pre-existing" as far as a
# fresh -e check is concerned).
awk '
  /ownership_get SINGBOX_BIN_PRE_EXISTED/ { in_guard = 1; guard_line = NR }
  in_guard && /rm -f \/usr\/local\/bin\/sing-box"?[^.]/ { found = 1 }
  /^fi$/ && in_guard && NR > guard_line + 1 { in_guard = 0 }
  END { exit(found ? 0 : 1) }
' "$UNINSTALL_SH" \
  && ok "sing-box binary removal is textually inside the SINGBOX_BIN_PRE_EXISTED-gated branch" \
  || fail "sing-box binary removal is not gated on SINGBOX_BIN_PRE_EXISTED in uninstall.sh — a pre-existing binary could be deleted"

echo "--- provisioning-agent binary removal is gated on AGENT_BINARY_INSTALLED + AGENT_BINARY_PRE_EXISTED ---"
if grep -q 'ownership_get AGENT_BINARY_INSTALLED' "$UNINSTALL_SH" \
  && grep -q 'ownership_get AGENT_BINARY_PRE_EXISTED' "$UNINSTALL_SH"; then
  ok "provisioning-agent binary removal checks both ownership facts"
else
  fail "provisioning-agent binary removal is missing an ownership-manifest gate"
fi

echo "--- no unconditional 'rm -f /usr/local/bin/sing-box' outside the gated branch ---"
# A second, ungated occurrence of the same rm would silently bypass the
# guard above.
count="$(grep -c 'rm -f /usr/local/bin/sing-box;' "$UNINSTALL_SH" || true)"
[ "${count:-0}" -eq 1 ] && ok "exactly one sing-box binary removal site, and it is gated" || fail "expected exactly 1 'rm -f /usr/local/bin/sing-box;' occurrence, found ${count:-0}"

echo "--- fixed system paths this script removes are never built from manifest string concatenation ---"
# Defense-in-depth against path-substitution/symlink-style attacks via a
# tampered manifest: the binary paths themselves must be literal string
# constants in the script, not "$SOME_MANIFEST_VAR/sing-box" or similar,
# so a corrupted or hostile ownership.env entry cannot redirect a
# binary-path rm at all (only the boolean pre-existed/installed facts are
# manifest-sourced for these two paths — the path itself never is).
if grep -Eq '/usr/local/bin/sing-box"?\$\{?[A-Z_]+' "$UNINSTALL_SH"; then
  fail "sing-box binary path appears to be built from a manifest/variable suffix — path-substitution risk"
else
  ok "sing-box binary path is a literal constant, not manifest-derived"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all F13 binary-ownership invariant checks passed"
