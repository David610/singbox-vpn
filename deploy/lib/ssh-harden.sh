#!/usr/bin/env bash
# Safe SSH key migration. Never disables password login until an independent
# public-key login with the exact key fingerprint is PROVEN in sshd's own log.
#
#   ssh-harden.sh status
#   ssh-harden.sh add-key (--pubkey-file FILE | --pubkey 'ssh-ed25519 AAAA...')
#   ssh-harden.sh verify                           # proof: Accepted publickey <fp>
#   ssh-harden.sh disable-passwords                # refuses unless verify passes
#   ssh-harden.sh rollback                         # restore previous sshd drop-in state
#
# Order for the operator: add-key; from ANOTHER machine/session run
# `ssh -i <key> root@host true` (keep the current session open); verify;
# disable-passwords; open yet another session to confirm; keep rollback ready.
# Prints fingerprints only; never prints key material beyond the public key
# the operator supplied.
set -Eeuo pipefail

TARGET_USER="${SSH_HARDEN_USER:-root}"
AUTH_KEYS_FILE="${SSH_HARDEN_AUTH_KEYS:-$(getent passwd "$TARGET_USER" | cut -d: -f6)/.ssh/authorized_keys}"
DROPIN_DIR="${SSH_HARDEN_DROPIN_DIR:-/etc/ssh/sshd_config.d}"
DROPIN="$DROPIN_DIR/00-singbox-vpn-hardening.conf"
BACKUP_DIR="${SSH_HARDEN_BACKUP_DIR:-/var/lib/singbox-vpn/ssh-harden}"
SSHD_BIN="${SSH_HARDEN_SSHD:-sshd}"
LOG_CMD="${SSH_HARDEN_LOG_CMD:-journalctl -u ssh -u sshd --no-pager -o cat}"
RELOAD_CMD="${SSH_HARDEN_RELOAD_CMD:-systemctl reload ssh}"
STATE="$BACKUP_DIR/state.env"

die() { echo "ssh-harden: ERROR: $*" >&2; exit 1; }
log() { echo "ssh-harden: $*"; }

effective() { "$SSHD_BIN" -T 2>/dev/null | awk -v k="$1" '$1==k{print $2; exit}' || true; }
fp_of() { ssh-keygen -l -f "$1" 2>/dev/null | awk '{print $2}'; }

cmd_status() {
  log "effective passwordauthentication=$(effective passwordauthentication) permitrootlogin=$(effective permitrootlogin) kbdinteractive=$(effective kbdinteractiveauthentication)"
  local n=0
  [ -s "$AUTH_KEYS_FILE" ] && n="$(grep -cE '^(ssh-|ecdsa-|sk-)' "$AUTH_KEYS_FILE" || true)"
  log "authorized public keys for $TARGET_USER: $n"
  if [ -s "$AUTH_KEYS_FILE" ]; then
    ssh-keygen -l -f "$AUTH_KEYS_FILE" 2>/dev/null | awk '{print "  key fingerprint " $2 " " $NF}'
  fi
  log "failed password attempts in current log: $($LOG_CMD 2>/dev/null | grep -c 'Failed password' || true)"
}

cmd_add_key() {
  local keyfile="" tmp
  case "${1:-}" in
    --pubkey-file) keyfile="${2:-}" ;;
    --pubkey) tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT; printf '%s\n' "${2:-}" >"$tmp"; keyfile="$tmp" ;;
    *) die "usage: add-key (--pubkey-file FILE | --pubkey 'ssh-ed25519 AAAA...')" ;;
  esac
  [ -s "$keyfile" ] || die "public key file missing/empty"
  [ "$(grep -cE '^(ssh-|ecdsa-|sk-)' "$keyfile" || true)" = "1" ] || die "expected exactly one public key line"
  grep -qE 'PRIVATE KEY' "$keyfile" && die "that is a PRIVATE key. Never paste it anywhere; supply the .pub file"
  local fp; fp="$(fp_of "$keyfile")"
  [ -n "$fp" ] || die "not a valid OpenSSH public key"
  install -d -m 0700 "$(dirname "$AUTH_KEYS_FILE")"
  touch "$AUTH_KEYS_FILE"; chmod 0600 "$AUTH_KEYS_FILE"
  if ssh-keygen -l -f "$AUTH_KEYS_FILE" 2>/dev/null | awk '{print $2}' | grep -qxF "$fp"; then
    log "key $fp already present (no change)"
  else
    grep -E '^(ssh-|ecdsa-|sk-)' "$keyfile" >>"$AUTH_KEYS_FILE"
    log "added key $fp to $AUTH_KEYS_FILE"
  fi
  install -d -m 0700 "$BACKUP_DIR"
  { echo "KEY_FP=$fp"; echo "KEY_ADDED_AT=$(date -u +%FT%TZ)"; } >"$STATE"
  log "NEXT: from another machine run: ssh -i <your-private-key> $TARGET_USER@<host> true  (keep this session open), then: ssh-harden.sh verify"
}

cmd_verify() {
  [ -f "$STATE" ] || die "no key recorded; run add-key first"
  # shellcheck disable=SC1090
  . "$STATE"
  local logs
  logs="$($LOG_CMD 2>/dev/null || true)"
  if grep -E "Accepted publickey for $TARGET_USER from " <<<"$logs" | grep -qF "$KEY_FP"; then
    log "VERIFIED: sshd logged a successful public-key login for $TARGET_USER with $KEY_FP"
    return 0
  fi
  die "NOT verified: no 'Accepted publickey for $TARGET_USER' log line with $KEY_FP. Log in with the key from another session first. Passwords stay enabled."
}

cmd_disable() {
  cmd_verify
  local pre; pre="$(effective passwordauthentication)"
  install -d -m 0700 "$BACKUP_DIR"
  if [ -f "$DROPIN" ]; then cp -a "$DROPIN" "$BACKUP_DIR/dropin.prev"; echo "HAD_DROPIN=1" >>"$STATE"; else echo "HAD_DROPIN=0" >>"$STATE"; fi
  cat >"$DROPIN.new" <<'CONF'
# Managed by singbox-vpn deploy/lib/ssh-harden.sh. Named 00- so it wins over
# cloud-init's 50-* drop-in (sshd uses the first value it reads).
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
PubkeyAuthentication yes
MaxAuthTries 3
CONF
  mv "$DROPIN.new" "$DROPIN"
  if ! "$SSHD_BIN" -t 2>/dev/null; then
    log "sshd -t rejected the new configuration; rolling back"
    cmd_rollback; die "hardening not applied"
  fi
  $RELOAD_CMD
  local post; post="$(effective passwordauthentication)"
  if [ "$post" != "no" ]; then
    log "effective passwordauthentication is '$post' (expected 'no'); rolling back"
    cmd_rollback; die "an earlier sshd drop-in overrides the hardening; not applied"
  fi
  log "passwords disabled (was: $pre). KEEP THIS SESSION OPEN and confirm a fresh key login now. Rollback: ssh-harden.sh rollback"
}

cmd_rollback() {
  # shellcheck disable=SC1090
  if [ -f "$STATE" ]; then . "$STATE" 2>/dev/null || true; fi
  if [ "${HAD_DROPIN:-0}" = "1" ] && [ -f "$BACKUP_DIR/dropin.prev" ]; then cp -a "$BACKUP_DIR/dropin.prev" "$DROPIN"; else rm -f "$DROPIN"; fi
  "$SSHD_BIN" -t && $RELOAD_CMD
  log "rolled back; effective passwordauthentication=$(effective passwordauthentication)"
}

case "${1:-}" in
  status) cmd_status ;;
  add-key) shift; cmd_add_key "$@" ;;
  verify) shift; cmd_verify "$@" ;;
  disable-passwords) cmd_disable ;;
  rollback) cmd_rollback ;;
  *) sed -n '2,15p' "$0"; exit 2 ;;
esac
