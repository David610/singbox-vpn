#!/usr/bin/env bash
# Regression test: the release workflow's "Publish root install.sh" step and
# the "Build combined checksum file" step that follows it must work together.
#
# The first step once recorded the bootstrap script's checksum as
# `dist/install.sh`; the second step runs from inside dist/ and verifies
# SHA256SUMS there, so `sha256sum -c` could not find that path and the publish
# job failed after every artifact had already been built and attested. That
# step shipped after v1.1.0, so no release had exercised it until v1.1.0-rc.7.
#
# This test executes the real `run:` scripts extracted from release.yml
# against a fake dist/, so it fails if either step's text regresses.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
WORKFLOW="$REPO_ROOT/.github/workflows/release.yml"
[ -f "$WORKFLOW" ] || { echo "FAIL: $WORKFLOW not found" >&2; exit 1; }

extract_run_block() {
  python3 - "$WORKFLOW" "$1" <<'PY'
import re
import sys

path, prefix = sys.argv[1], sys.argv[2]
lines = open(path, encoding="utf-8").read().splitlines()

start = None
for index, line in enumerate(lines):
    match = re.match(r"\s*-\s+name:\s+(.*?)\s*$", line)
    if match and match.group(1).startswith(prefix):
        start = index
        break
if start is None:
    sys.exit(f"step not found: {prefix}")

cursor = start + 1
while cursor < len(lines) and not re.match(r"\s*run:\s*\|\s*$", lines[cursor]):
    if re.match(r"\s*-\s+(name|uses):", lines[cursor]):
        sys.exit(f"step has no run block: {prefix}")
    cursor += 1
if cursor >= len(lines):
    sys.exit(f"step has no run block: {prefix}")

indent = None
body = []
for line in lines[cursor + 1:]:
    if line.strip() == "":
        body.append("")
        continue
    current = len(line) - len(line.lstrip())
    if indent is None:
        indent = current
    if current < indent:
        break
    body.append(line[indent:])
print("\n".join(body))
PY
}

publish_step="$(extract_run_block 'Publish root install.sh')"
combine_step="$(extract_run_block 'Build combined checksum file')"
if [ -z "$publish_step" ] || [ -z "$combine_step" ]; then
  echo "FAIL: could not extract the release workflow steps" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/src" "$work/dist"
printf '#!/usr/bin/env bash\necho bootstrap\n' >"$work/src/install.sh"

(
  cd "$work/dist"
  printf 'binary archive' >singbox-vpn-x86_64-unknown-linux-gnu.tar.gz
  printf 'source archive' >singbox-vpn-src.tar.gz
  printf '{}' >singbox-vpn-sbom.cdx.json
  printf '{}' >singbox-vpn-licenses.json
  # The build job writes every other checksum with a bare file name, from
  # inside dist/.
  for f in ./*.tar.gz singbox-vpn-sbom.cdx.json singbox-vpn-licenses.json; do
    f="${f#./}"
    sha256sum "$f" >"$f.sha256"
  done
)

(cd "$work" && bash -c "$publish_step") || {
  echo "FAIL: the 'Publish root install.sh' step failed" >&2
  exit 1
}
(cd "$work" && bash -c "$combine_step") || {
  echo "FAIL: 'Build combined checksum file' could not verify SHA256SUMS" >&2
  exit 1
}

(
  cd "$work/dist"
  grep -Eq ' [ *]install\.sh$' SHA256SUMS || {
    echo "FAIL: SHA256SUMS does not list install.sh by its bare file name" >&2
    exit 1
  }
  if grep -q 'dist/' SHA256SUMS; then
    echo "FAIL: SHA256SUMS contains a directory-qualified path" >&2
    exit 1
  fi
  sha256sum -c SHA256SUMS >/dev/null
)

echo "PASS: release bootstrap checksum steps are consistent"
