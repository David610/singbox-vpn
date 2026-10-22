#!/usr/bin/env bash
# I-27: a Windows checkout with core.autocrlf=true turned shell scripts and
# systemd units into CRLF files that bash and systemd reject. .gitattributes
# pins those files to LF. Functional: clone the committed tree with
# core.autocrlf=true (which converts text files on any OS) and look for CR.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
command -v git >/dev/null 2>&1 || { echo "line endings: SKIP (needs git)"; exit 0; }
git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1 || { echo "line endings: SKIP (not a git checkout)"; exit 0; }
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

mapfile -t guarded < <(git -c safe.directory='*' -C "$ROOT" ls-files -- '*.sh' '*.py' '*.service' '*.timer' '*.path' '*.env' '*.template' '*.toml' 'bin/singbox-vpn-uninstall')
[ "${#guarded[@]}" -gt 50 ] && ok "found ${#guarded[@]} tracked scripts, units, templates and configs to guard" || fail "found only ${#guarded[@]} files to guard"

missing=0
for f in "${guarded[@]}"; do
  attr="$(git -c safe.directory='*' -C "$ROOT" check-attr eol -- "$f")"
  [[ "$attr" == *": eol: lf" ]] || { missing=$((missing + 1)); [ "$missing" -le 3 ] && echo "  no eol=lf for $f ($attr)"; }
done
[ "$missing" -eq 0 ] && ok "every guarded file has eol=lf in .gitattributes" || fail "$missing guarded file(s) lack eol=lf"

if git -c safe.directory='*' clone -q --no-hardlinks -c core.autocrlf=true "$ROOT" "$TMP/win" 2>"$TMP/clone.err"; then
  crlf=0
  for f in "${guarded[@]}"; do
    [ -f "$TMP/win/$f" ] || continue
    if grep -q $'\r' "$TMP/win/$f"; then crlf=$((crlf + 1)); [ "$crlf" -le 3 ] && echo "  CRLF in $f"; fi
  done
  [ "$crlf" -eq 0 ] && ok "a core.autocrlf=true checkout has no CRLF in scripts, units or templates" || fail "$crlf file(s) came out CRLF"
  bash -n "$TMP/win/install.sh" 2>/dev/null && ok "install.sh from that checkout still parses under bash" || fail "install.sh from the autocrlf checkout does not parse"
else
  fail "could not clone the repository: $(cat "$TMP/clone.err")"
fi

[ "$failures" -eq 0 ] || { echo "line endings: $failures failure(s)"; exit 1; }
echo "line endings: PASS"
