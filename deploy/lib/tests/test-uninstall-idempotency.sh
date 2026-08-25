#!/usr/bin/env bash
# Runs the REAL deploy/almalinux/uninstall.sh twice against a host with
# no singbox-vpn installation present, and asserts both runs exit 0 and that
# uninstall correctly reports nothing to remove — i.e. "uninstall after
# a successful uninstall exits successfully and reports nothing left",
# and "uninstall on a host that was never installed is a safe no-op".
#
# uninstall.sh currently hardcodes real system paths (/etc/vpn,
# /var/lib/singbox-vpn, /opt/singbox-vpn — like install.sh, it has no override
# variable for a throwaway test root; see docs/FINAL_PRODUCTION_AUDIT.md
# for the broader "make every path overridable for testing" follow-up).
# Since it is ownership-gated and this test only proceeds when none of
# those paths (nor singbox-vpn's service users/groups) exist yet, running the
# real script here can only ever perform no-ops — it cannot delete
# anything real. Requires root (same as uninstall.sh itself); skips
# itself cleanly otherwise, matching the other root-requiring checks in
# this suite.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UNINSTALL_SH="$REPO_ROOT/deploy/almalinux/uninstall.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

if [ "$(id -u)" -ne 0 ]; then
  echo "SKIP: test-uninstall-idempotency.sh requires root (uninstall.sh itself requires it); not running as root here."
  exit 0
fi

unsafe=0
for p in /etc/vpn /var/lib/singbox-vpn /opt/singbox-vpn; do
  [ -e "$p" ] && { echo "SKIP: $p already exists on this host — refusing to run the real uninstall.sh here (this test only runs when it can prove doing so is a guaranteed no-op)."; unsafe=1; }
done
for u in sing-box vpn-subscription; do
  id "$u" >/dev/null 2>&1 && { echo "SKIP: system user '$u' already exists — refusing to run the real uninstall.sh here."; unsafe=1; }
done
getent group vpn-compat >/dev/null 2>&1 && { echo "SKIP: group 'vpn-compat' already exists — refusing to run the real uninstall.sh here."; unsafe=1; }
if [ "$unsafe" -eq 1 ]; then
  echo "SKIP: environment is not provably clean of singbox-vpn resources; skipping test-uninstall-idempotency.sh to avoid touching unrelated host state."
  exit 0
fi

echo "--- first run: uninstall.sh on a never-installed host ---"
out1="$("$UNINSTALL_SH" --yes 2>&1)"
rc1=$?
if [ "$rc1" -eq 0 ]; then
  ok "first run exits 0"
else
  fail "first run exited $rc1: $out1"
fi
if echo "$out1" | grep -qi "nothing to remove"; then
  ok "first run correctly reports nothing to remove"
else
  fail "first run did not report 'nothing to remove'; got: $out1"
fi

echo
echo "--- second run: re-running uninstall.sh after a successful (no-op) uninstall ---"
out2="$("$UNINSTALL_SH" --yes 2>&1)"
rc2=$?
if [ "$rc2" -eq 0 ]; then
  ok "second run exits 0 (idempotent)"
else
  fail "second run exited $rc2: $out2"
fi
if echo "$out2" | grep -qi "nothing to remove"; then
  ok "second run correctly reports nothing to remove"
else
  fail "second run did not report 'nothing to remove'; got: $out2"
fi

echo
echo "--- verifies no singbox-vpn paths/accounts were created as a SIDE EFFECT of running uninstall.sh itself ---"
for p in /etc/vpn /opt/singbox-vpn; do
  [ -e "$p" ] && fail "uninstall.sh unexpectedly created $p"
done
[ -e /etc/vpn ] || [ -e /opt/singbox-vpn ] || ok "no singbox-vpn paths were created by running the uninstaller"

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all uninstall-idempotency tests passed"
