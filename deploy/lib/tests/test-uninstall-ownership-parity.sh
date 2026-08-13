#!/usr/bin/env bash
# Regression test against install/uninstall asymmetry: for every
# ownership-manifest key install.sh WRITES (ownership_mark/
# ownership_set/ownership_list_add/ownership_set_baseline_once), assert
# uninstall.sh actually READS that same key somewhere. This is the
# automated version of the "cross-check every mutation against
# uninstall/rollback coverage" self-review the task asked for — it
# fails loudly the next time someone adds a new tracked fact to
# install.sh without teaching uninstall.sh to act on it, instead of
# relying on a human catching the gap during review.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"
UNINSTALL_SH="$REPO_ROOT/deploy/almalinux/uninstall.sh"

failures=0

# Extract every ownership manifest KEY written by install.sh.
written_keys="$(grep -oE '(ownership_mark|ownership_set|ownership_list_add|ownership_set_baseline_once) [A-Za-z0-9_]+' "$INSTALL_SH" \
  | awk '{print $2}' | sort -u)"

if [ -z "$written_keys" ]; then
  echo "FAIL: found zero ownership_* writes in $INSTALL_SH — this test itself may be broken, or install.sh no longer uses the ownership manifest at all."
  exit 1
fi

echo "--- cross-check: every key install.sh writes is read somewhere in uninstall.sh ---"
while IFS= read -r key; do
  [ -z "$key" ] && continue
  # A few keys are purely internal bookkeeping for install.sh itself
  # (not resource-ownership facts uninstall.sh needs to act on) —
  # explicitly allow-listed here so this test does not demand uninstall
  # read something that was never meant to be consumed by it.
  case "$key" in
    INSTALL_ATTEMPTED)
      continue # only read by install.sh's own on_fatal_error rollback gate
      ;;
  esac
  if grep -q "$key" "$UNINSTALL_SH"; then
    echo "ok: $key is consumed by uninstall.sh"
  else
    echo "FAIL: install.sh records '$key' but uninstall.sh never references it — a resource this key tracks may be left behind (or an operator's pre-existing resource may be wrongly removed). Teach uninstall.sh about it, or add it to this test's allow-list with a comment explaining why it is install.sh-internal only."
    failures=$((failures + 1))
  fi
done <<EOF
$written_keys
EOF

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures key(s) written by install.sh are not consumed by uninstall.sh"
  exit 1
fi
echo "all ownership keys written by install.sh are consumed by uninstall.sh"
