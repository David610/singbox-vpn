#!/usr/bin/env bash
# #167: the evidence collector is read-only, keeps raw data in a root-only
# directory, and prints a redacted summary. Hermetic: stub log command, temp dir.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TOOL="$ROOT/deploy/lib/ssh-auth-evidence.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

cat >"$TMP/log" <<'LOG'
2026-10-22T08:42:11+0000 h sshd[2]: Accepted password for root from 192.0.2.10 port 5001 ssh2
2026-10-22T09:10:30+0000 h sshd[4]: Accepted password for root from 198.51.100.7 port 5003 ssh2
2026-10-22T08:40:09+0000 h sshd[1]: Invalid user hunter2 from 198.51.100.7 port 4003
LOG
mkdir "$TMP/base"
run_tool() { SSH_EVIDENCE_ALLOW_NON_ROOT=1 SSH_EVIDENCE_STAMP=T1 SSH_EVIDENCE_LOG_CMD="cat $TMP/log" SSH_EVIDENCE_OUT_BASE="$TMP/base" bash "$TOOL"; }
if out="$(run_tool 2>"$TMP/err")"; then ok "collector runs"; else fail "collector failed: $(cat "$TMP/err")"; fi

for secret in 192.0.2.10 198.51.100.7 hunter2; do
  if grep -q "$secret" <<<"$out"; then fail "stdout leaked $secret"; else ok "stdout does not contain $secret"; fi
done
grep -q '"flag": "ROOT_PASSWORD_SUCCESS"' <<<"$out" && ok "root password success is flagged" || fail "missing flag"

dir="$TMP/base/ssh-auth-evidence-T1"
[ "$(stat -c %a "$dir")" = 700 ] && ok "evidence directory is mode 0700" || fail "directory mode $(stat -c %a "$dir")"
for f in sshd-events.raw.log private-sources.json; do
  [ "$(stat -c %a "$dir/$f")" = 600 ] && ok "$f is mode 0600" || fail "$f mode $(stat -c %a "$dir/$f")"
done
grep -q 192.0.2.10 "$dir/private-sources.json" && ok "the private map holds the addresses" || fail "private map empty"
extra="$(find "$TMP/base" -type f | grep -vc "$dir/" || true)"
[ "$extra" -eq 0 ] && ok "nothing was written outside the evidence directory" || fail "unexpected files outside it"

# an existing evidence directory is never reused or overwritten
if run_tool >/dev/null 2>"$TMP/dup.err"; then fail "reused an existing evidence directory"; else
  grep -q 'must not already exist' "$TMP/dup.err" && ok "an existing evidence directory is refused" || fail "wrong duplicate error: $(cat "$TMP/dup.err")"; fi
grep -q 192.0.2.10 "$dir/private-sources.json" && ok "the refused run left the first evidence intact" || fail "first evidence damaged"
if [ "$(id -u)" -ne 0 ]; then
  if bash "$TOOL" >/dev/null 2>"$TMP/nonroot.err"; then fail "ran as non-root without the test override"; else
    grep -q 'run as root' "$TMP/nonroot.err" && ok "refuses to run as non-root" || fail "wrong non-root error"; fi
fi
if bash "$TOOL" --bogus >/dev/null 2>&1; then fail "accepted an unknown argument"; else ok "unknown argument rejected"; fi

[ "$failures" -eq 0 ] || { echo "ssh auth evidence: $failures failure(s)"; exit 1; }
echo "ssh auth evidence: PASS"
