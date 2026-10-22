#!/usr/bin/env bash
# Regression (I-41): create_service_users must succeed when the service
# user's same-named primary group already exists (left over by a root-run test
# suite or an interrupted earlier install). Real useradd refuses with exit 9
# ("group X exists") unless told -g, which aborted a fresh install on
# Ubuntu 26.04. Stubs model that refusal; no root required, no host changes.
set -Euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
: >"$T/users"; : >"$T/groups"; : >"$T/marks"; : >"$T/calls"

# shellcheck disable=SC2317
id() { grep -qx "$1" "$T/users"; }
getent() { [ "$1" = group ] && grep -qx "$2" "$T/groups"; }
groupadd() { echo "groupadd $*" >>"$T/calls"; echo "${@: -1}" >>"$T/groups"; }
usermod() { echo "usermod $*" >>"$T/calls"; }
ownership_mark() { echo "$1" >>"$T/marks"; }
useradd() {
  local name="${*: -1}" gflag=0 a
  for a in "$@"; do [ "$a" = "-g" ] && gflag=1; done
  echo "useradd $*" >>"$T/calls"
  if grep -qx "$name" "$T/groups" && [ "$gflag" = 0 ]; then
    echo "useradd: group $name exists - if you want to add this user to that group, use -g." >&2
    return 9
  fi
  echo "$name" >>"$T/users"; grep -qx "$name" "$T/groups" || echo "$name" >>"$T/groups"
}

# Pull the real function out of the installer (no sourcing side effects).
eval "$(sed -n '/^create_service_users() {/,/^}/p' "$REPO_ROOT/deploy/almalinux/install.sh")"

# errexit exactly as in install.sh: an unhandled useradd failure aborts the stage.
# (Run as a plain statement: bash disables errexit inside any `if`/`||` context.)
attempt() { ( set -e; create_service_users ); RC=$?; }
run() { : >"$T/users"; : >"$T/marks"; : >"$T/calls"; attempt; return "$RC"; }

echo "--- clean host ---"
: >"$T/groups"
run; if [ "$RC" -eq 0 ]; then ok "clean host: users created"; else fail "clean host: create_service_users failed"; fi
grep -qx USER_SINGBOX_CREATED "$T/marks" && grep -qx USER_VPNSUB_CREATED "$T/marks" \
  && ok "clean host: both users marked as created (uninstall will remove them)" || fail "clean host: ownership marks missing"

echo "--- orphan same-named groups already exist (the Ubuntu 26.04 failure) ---"
printf 'sing-box\nvpn-subscription\n' >"$T/groups"
run; if [ "$RC" -eq 0 ]; then ok "pre-existing groups: create_service_users succeeded"; else fail "pre-existing groups: create_service_users failed (useradd exit 9)"; fi
{ id sing-box && id vpn-subscription; } && ok "pre-existing groups: both users now exist" || fail "pre-existing groups: users missing"
grep -qx USER_SINGBOX_CREATED "$T/marks" && ok "user mark still recorded" || fail "user mark not recorded"
grep -q 'groupadd .*sing-box' "$T/calls" && fail "must not re-create a pre-existing group" || ok "pre-existing group not re-created"

echo "--- idempotent re-run ---"
attempt; if [ "$RC" -eq 0 ]; then ok "re-run succeeded"; else fail "re-run failed"; fi

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }
echo "all passed"
