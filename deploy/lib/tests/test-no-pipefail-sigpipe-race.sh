#!/usr/bin/env bash
# I-53 guard. `producer | grep -q ...` under `set -o pipefail` returns the wrong
# answer at random when the producer writes more than one chunk (grep -q exits at
# its first match, the producer gets SIGPIPE, the pipeline reports 141). This was
# the root cause of the intermittent shell-suite failures (measured 14/6000 for a
# 10.7 KB value). The repository must stay free of the pattern, and the lint must
# demonstrably detect it.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LINT="$REPO_ROOT/deploy/lib/lint_pipefail_pipes.py"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT

# 1. the repository is clean
( cd "$REPO_ROOT" && python3 -I "$LINT" >"$T/repo.out" 2>&1 ); rc=$?
[ "$rc" -eq 0 ] && ok "no early-exit grep fed by echo/printf/cat or a large producer under pipefail" || { bad "lint findings:"; sed 's/^/    /' "$T/repo.out" | head -20; }

# 2. the lint detects the pattern (so it cannot silently rot)
cat >"$T/bad.sh" <<'EOF'
set -o pipefail
body="$(cat big)"
echo "$body" | grep -q needle
printf '%s\n' "$body" | grep -qE 'x'
cat file | grep -m1 y
journalctl -u ssh | grep -q Accepted
port="$(sshd -T | awk '/^port /{print $2; exit}')"
EOF
python3 -I "$LINT" "$T/bad.sh" >"$T/bad.out" 2>&1; rc=$?
{ [ "$rc" -eq 1 ] && [ "$(grep -c ': R[123]:' "$T/bad.out")" -eq 5 ]; } && ok "lint flags all five known-bad forms (incl. early-exit awk after a command)" || { bad "lint missed forms:"; cat "$T/bad.out"; }

# 3. safe forms are not flagged
cat >"$T/good.sh" <<'EOF'
set -o pipefail
grep -q needle <<<"$body"
echo "$body" | grep -o x | wc -l
echo "n=$(ss -ltn | grep -Ec sing-box)"
echo "$x" | tr a b
port="$(sshd -T | awk '/^port /{print $2; exit}' || true)"
# echo "$body" | grep -q commented
EOF
python3 -I "$LINT" "$T/good.sh" >"$T/good.out" 2>&1; rc=$?
[ "$rc" -eq 0 ] && ok "here-string and non-early-exit forms are accepted" || { bad "false positive:"; cat "$T/good.out"; }

# 4. files without pipefail are ignored (the hazard needs pipefail)
printf 'echo "$b" | grep -q x\n' >"$T/nopf.sh"
python3 -I "$LINT" "$T/nopf.sh" >/dev/null 2>&1 && ok "scripts without pipefail are not flagged" || bad "flagged a script without pipefail"

# 5. the mechanism itself (documents WHY; large value, many tries, here-string never wrong)
body="$(head -c 20000 /dev/zero | tr '\0' 'a')needle"
wrong=0
for _ in $(seq 1 300); do grep -q needle <<<"$body" || wrong=$((wrong + 1)); done
[ "$wrong" -eq 0 ] && ok "here-string form: 0/300 wrong answers on a 20 KB value" || bad "here-string form gave $wrong wrong answers"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
