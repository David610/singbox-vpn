#!/usr/bin/env bash
# Regression coverage for the public repository rename. Runtime identifiers
# such as /opt/vpn1 and VPN1_REPO intentionally remain for compatibility; only
# their public/default repository value is asserted here.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT"

CANONICAL_REPO="David610/singbox-vpn"
OLD_REPO="David610/"'vpn1'
WRONG_REPO='singbox-vpn-''installer'

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local file="$1" expected="$2"
  grep -Fq -- "$expected" "$file" \
    || fail "$file does not contain expected repository contract: $expected"
}

assert_contains install.sh 'VPN1_REPO="${VPN1_REPO:-David610/singbox-vpn}"'
assert_contains uninstall.sh 'VPN1_REPO="${VPN1_REPO:-David610/singbox-vpn}"'
assert_contains deploy/almalinux/install.sh 'VPN1_RELEASE_REPO="${VPN1_RELEASE_REPO:-David610/singbox-vpn}"'
assert_contains deploy/almalinux/update.sh 'VPN1_REPO="${VPN1_REPO_OVERRIDE:-${CURRENT_REPO:-David610/singbox-vpn}}"'

# Fork overrides are a supported bootstrap/update API and must survive a rename.
assert_contains install.sh '--repo)'
assert_contains uninstall.sh '--repo)'
assert_contains deploy/almalinux/update.sh '--repo)'
assert_contains install.sh 'VPN1_RELEASE_REPO="$VPN1_REPO"'

# Check source/docs/workflows without depending on a .git directory, so this
# also works from a release source archive.
while IFS= read -r file; do
  if grep -Fq -- "$OLD_REPO" "$file"; then
    fail "stale public repository reference in ${file#./}: $OLD_REPO"
  fi
  if grep -Fq -- "$WRONG_REPO" "$file"; then
    fail "incorrect repository name in ${file#./}: $WRONG_REPO"
  fi
done < <(
  find . -type f \
    \( -name '*.md' -o -name '*.sh' -o -name '*.yml' -o -name '*.yaml' -o -name '*.rs' \) \
    -not -path './.git/*' -not -path './target/*' -print
)

assert_contains README.md "raw.githubusercontent.com/$CANONICAL_REPO/main/install.sh"
assert_contains install.sh "raw.githubusercontent.com/$CANONICAL_REPO/main/install.sh"
assert_contains uninstall.sh "raw.githubusercontent.com/$CANONICAL_REPO/main/uninstall.sh"
assert_contains deploy/almalinux/lifecycle-acceptance.sh "raw.githubusercontent.com/$CANONICAL_REPO/\$BOOTSTRAP_REF/install.sh"

echo "repository identity regression checks: PASS ($CANONICAL_REPO)"
