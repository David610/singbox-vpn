#!/usr/bin/env bash
# Read-only sshd authentication evidence for the owner's host-trust decision
# (issue #167). Run as root from the PROVIDER CONSOLE (or an existing trusted
# session), ideally after a provider snapshot. It changes nothing on the host
# except creating one new root-only directory for the raw evidence.
#
#   ssh-auth-evidence.sh [--since 'YYYY-MM-DD HH:MM'] [--expected FILE]
#
# stdout is REDACTED (sources are labels S1, S2, ...; no addresses, no invalid
# user names, no keys) and is safe to paste into the issue. The raw logs and the
# label->address map stay in the new 0700 directory; copy that off the host over
# an encrypted channel and never commit or paste it.
set -Eeuo pipefail
umask 077

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SINCE=""
EXPECTED=""
LOG_CMD="${SSH_EVIDENCE_LOG_CMD:-}"
OUT_BASE="${SSH_EVIDENCE_OUT_BASE:-/root}"

die() { echo "ssh-auth-evidence: ERROR: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --since) SINCE="${2:?--since needs a value}"; shift 2 ;;
    --expected) EXPECTED="${2:?--expected needs a file}"; shift 2 ;;
    -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[ -z "$EXPECTED" ] || [ -r "$EXPECTED" ] || die "cannot read --expected file"
command -v python3 >/dev/null 2>&1 || die "python3 is required"
[ "${SSH_EVIDENCE_ALLOW_NON_ROOT:-0}" = 1 ] || [ "$(id -u)" -eq 0 ] || die "run as root (the logs are root-only)"

stamp="${SSH_EVIDENCE_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
out="$OUT_BASE/ssh-auth-evidence-$stamp"
mkdir -m 0700 "$out" 2>/dev/null || die "cannot create $out (it must not already exist)"

collect() {
  if [ -n "$LOG_CMD" ]; then
    bash -c "$LOG_CMD" || true
    return
  fi
  local since_args=()
  [ -z "$SINCE" ] || since_args=(--since "$SINCE")
  journalctl -u ssh -u sshd --no-pager -o short-iso "${since_args[@]}" 2>/dev/null || true
  local f
  for f in /var/log/auth.log /var/log/auth.log.1 /var/log/secure /var/log/secure-*; do
    [ -r "$f" ] && cat "$f"
  done
  for f in /var/log/auth.log.*.gz; do
    [ -r "$f" ] && zcat "$f" 2>/dev/null || true
  done
}

collect >"$out/sshd-events.raw.log"
[ -s "$out/sshd-events.raw.log" ] || echo "ssh-auth-evidence: WARNING: no sshd events found; absence of logs is itself not evidence of absence of logins (logs can be rotated or removed)" >&2
if [ -z "$LOG_CMD" ] && command -v last >/dev/null 2>&1; then
  last -i -F -n 200 >"$out/last.raw.txt" 2>/dev/null || true
fi

args=(--input "$out/sshd-events.raw.log" --private-out "$out/private-sources.json")
[ -z "$EXPECTED" ] || args+=(--expected "$EXPECTED")
python3 -I "$HERE/ssh_auth_summary.py" "${args[@]}"

{
  echo
  echo "ssh-auth-evidence: raw evidence is in $out (mode 0700, root only)."
  echo "ssh-auth-evidence: copy it off this host over an encrypted channel; do NOT paste, commit or upload it."
  echo "ssh-auth-evidence: match S1, S2, ... to your own addresses using private-sources.json, then record the decision (docs/qualification/2026-10-22/HOST-TRUST-DECISION.md)."
} >&2
