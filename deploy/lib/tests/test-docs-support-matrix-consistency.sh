#!/usr/bin/env bash
# Lightweight, deterministic documentation-drift check (docs/INSTALLATION.md
# §10 / task requirement: "prevent obvious support-doc drift"). Not a
# markdown parser — plain grep/file-existence checks only.
#
# Guards against:
#   - README.md silently going back to claiming AlmaLinux is the only
#     supported server, or losing its link to the canonical docs.
#   - docs/INSTALLATION.md disappearing while still linked from README.
#   - every relative markdown link README.md points at actually resolving
#     to a real file in the repo.
#   - every OS_ID deploy/lib/os.sh explicitly branches on being mentioned
#     somewhere in docs/SUPPORTED_PRODUCT.md's support matrix, so a new
#     os.sh case can't be added without a corresponding doc update.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
README="$REPO_ROOT/README.md"
INSTALLATION="$REPO_ROOT/docs/INSTALLATION.md"
SUPPORTED_PRODUCT="$REPO_ROOT/docs/SUPPORTED_PRODUCT.md"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- canonical docs exist ---"
[ -f "$README" ] && ok "README.md exists" || fail "README.md is missing"
[ -f "$INSTALLATION" ] && ok "docs/INSTALLATION.md exists" || fail "docs/INSTALLATION.md is missing"
[ -f "$SUPPORTED_PRODUCT" ] && ok "docs/SUPPORTED_PRODUCT.md exists" || fail "docs/SUPPORTED_PRODUCT.md is missing"

echo
echo "--- README.md does not overclaim or underclaim server support ---"
if grep -qE 'AlmaLinux 9 x86-64 VPS' "$README" 2>/dev/null; then
  fail "README.md still states AlmaLinux 9 x86-64 as the sole requirement (the installer supports more than one distribution now)"
else
  ok "README.md does not claim AlmaLinux is the only supported server"
fi
if grep -qiE 'supports (most|any) linux distributions?' "$README" 2>/dev/null; then
  fail "README.md uses vague unqualified support language ('supports most/any Linux distributions')"
else
  ok "README.md does not use vague unqualified distribution-support language"
fi
if grep -qF 'docs/INSTALLATION.md' "$README" 2>/dev/null; then
  ok "README.md links to docs/INSTALLATION.md"
else
  fail "README.md does not link to docs/INSTALLATION.md"
fi
if grep -qF 'docs/SUPPORTED_PRODUCT.md' "$README" 2>/dev/null; then
  ok "README.md links to docs/SUPPORTED_PRODUCT.md"
else
  fail "README.md does not link to docs/SUPPORTED_PRODUCT.md"
fi

echo
echo "--- every relative docs/*.md and LICENSE link in README.md resolves to a real file ---"
# Extracts the (path) part of every markdown [text](path) link pointing at
# something under docs/ or LICENSE — deliberately ignores http(s):// links.
link_count=0
while IFS= read -r link; do
  [ -n "$link" ] || continue
  link_count=$((link_count + 1))
  if [ -f "$REPO_ROOT/$link" ]; then
    ok "README.md link resolves: $link"
  else
    fail "README.md links to '$link', which does not exist in the repo"
  fi
done < <(grep -oE '\]\((docs/[A-Za-z0-9_./-]+\.md|LICENSE)\)' "$README" | sed -E 's/^\]\(//; s/\)$//' | sort -u)
[ "$link_count" -gt 0 ] || fail "found zero docs/*.md or LICENSE links in README.md — link-extraction regex may be broken"

echo
echo "--- every OS_ID deploy/lib/os.sh explicitly branches on is mentioned in docs/SUPPORTED_PRODUCT.md ---"
# The explicit (non-generic-fallback) IDs os.sh matches by name, one per
# line — deliberately excludes the generic ID_LIKE-fedora/-debian fallback
# branches, which have no single OS_ID of their own to check.
check_os_id_documented() {
  local os_id="$1" doc_pattern="$2"
  if grep -qiE "$doc_pattern" "$SUPPORTED_PRODUCT"; then
    ok "OS_ID '$os_id' (deploy/lib/os.sh) is mentioned in docs/SUPPORTED_PRODUCT.md"
  else
    fail "OS_ID '$os_id' is explicitly branched on in deploy/lib/os.sh but never mentioned in docs/SUPPORTED_PRODUCT.md — the support matrix is drifting from the installer"
  fi
}
# os.sh's raw /etc/os-release ID values vs. the human-readable name the
# support matrix actually spells them out with.
check_os_id_documented almalinux almalinux
check_os_id_documented rocky rocky
check_os_id_documented rhel rhel
check_os_id_documented centos centos
check_os_id_documented amzn 'amazon linux'
check_os_id_documented ubuntu ubuntu
check_os_id_documented debian debian

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
