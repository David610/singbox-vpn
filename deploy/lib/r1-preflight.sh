#!/usr/bin/env bash
# READ-ONLY preflight for R1 (pushing the branch). Prints the facts the owner approves against and exits non-zero if any
# precondition fails. It never pushes, tags, merges or writes. Re-run immediately before authorizing the push.
#   bash deploy/lib/r1-preflight.sh [--branch NAME] [--base origin/main]
set -uo pipefail
BRANCH="staging/2026-10-20-remediation"; BASE="origin/main"
while [ "$#" -gt 0 ]; do case "$1" in --branch) BRANCH="$2"; shift 2;; --base) BASE="$2"; shift 2;; *) echo "unknown arg $1" >&2; exit 2;; esac; done
cd "$(git rev-parse --show-toplevel)" || exit 2
bad=0; ok() { echo "PASS  $1"; }; no() { echo "FAIL  $1"; bad=$((bad + 1)); }
HEAD_SHA="$(git rev-parse HEAD)"
echo "commit to push : $HEAD_SHA"
echo "branch         : $(git branch --show-current) (expected $BRANCH)"
[ "$(git branch --show-current)" = "$BRANCH" ] && ok "on the intended branch" || no "not on $BRANCH"
[ -z "$(git status --porcelain)" ] && ok "working tree clean (nothing uncommitted would be left out of the review)" || no "uncommitted changes"
git fetch -q origin 2>/dev/null && ok "fetched origin (read-only)" || no "could not fetch origin"
ahead="$(git rev-list --count "$BASE..HEAD" 2>/dev/null || echo ?)"; behind="$(git rev-list --count "HEAD..$BASE" 2>/dev/null || echo ?)"
echo "vs $BASE : $ahead ahead, $behind behind (base $(git rev-parse --short "$BASE" 2>/dev/null))"
[ "$behind" = "0" ] && ok "branch contains $BASE (fast-forwardable)" || no "branch is behind $BASE"
git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1 && no "remote branch $BRANCH already exists (push would update it)" || ok "remote branch does not exist yet (push creates it)"
git ls-remote --tags origin 'v1.3.1-rc.3' | grep -q . && no "tag v1.3.1-rc.3 already exists on the remote" || ok "tag v1.3.1-rc.3 does not exist (R5 not yet possible, none created by R1)"
echo "authors in range:"; git log --format='  %an <%ae>' "$BASE..HEAD" | sort | uniq -c
python3 -I deploy/lib/scan_known_secrets.py --range "$BASE..HEAD" --rev HEAD >/tmp/r1-secret-scan.$$ 2>&1; rc=$?
tail -1 /tmp/r1-secret-scan.$$; rm -f /tmp/r1-secret-scan.$$
[ "$rc" -eq 0 ] && ok "none of the node's live secrets is in the pushed commits or tree" || no "secret scan rc=$rc (1 = a live secret is in the repo, 2 = nothing could be checked)"
python3 -I - <<'PY' && ok "all workflows parse; every 'uses:' is pinned to a 40-hex SHA or a local path" || no "workflow YAML / pinning problem"
import glob, re, sys, yaml
for f in sorted(glob.glob(".github/workflows/*.yml")):
    d = yaml.safe_load(open(f))
    assert d.get("jobs"), f
    for m in re.finditer(r"^\s*-?\s*uses:\s*(\S+)", open(f).read(), re.M):
        u = m.group(1)
        if u.startswith("./") or u.startswith("docker://"): continue
        assert re.search(r"@[0-9a-f]{40}$", u), f"{f}: not SHA-pinned: {u}"
PY
git diff --name-only "$BASE..HEAD" -- .github | sed 's/^/  workflow file changed vs base: /'
echo "ci.yml runs on push to ANY branch with contents:read only; release.yml fires only on tags v*-* and manual dispatch (R1 does neither)."
[ "$bad" -eq 0 ] && echo "R1 PREFLIGHT: ALL PASS (this is NOT an authorization; the owner still has to approve the push)" || { echo "R1 PREFLIGHT: $bad problem(s)"; exit 1; }
