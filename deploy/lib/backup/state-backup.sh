#!/usr/bin/env bash
# Encrypted, off-host backup of singbox-vpn deployment state (identities, REALITY/Hysteria2 material, subscription
# records, certificates, install manifest). Added 2026-10-22, after the fin1 rebuild destroyed the only copy of all of it.
#
#   state-backup.sh create OUT_DIR                     encrypt state to OUT_DIR/<host>-<utc>.tar.age (+ .sha256)
#   state-backup.sh verify FILE                        ciphertext checksum + age header (no key needed)
#   state-backup.sh upload FILE                        sftp FILE + .sha256 off-host, download back, compare: VERIFIED
#   state-backup.sh restore-drill FILE IDENTITY DIR    decrypt into an EMPTY scratch DIR and check every file's hash
#   state-backup.sh prune DIR KEEP                     keep the KEEP newest local archives (remote copies are untouched)
#   state-backup.sh run                                create + verified upload + prune (systemd: vpn-state-backup.*)
#
# Encryption: `age` to the PUBLIC recipients in $BACKUP_RECIPIENTS (an age1... key or an ssh-ed25519 key the owner
# controls). fin1 never holds a decryption key, so neither a stolen backup nor a compromised fin1 exposes old
# backups. Plaintext never touches disk: tar streams straight into age. The embedded BACKUP-MANIFEST.sha256 lets a
# restore prove every file came back intact. A restore into the live tree is a separate, owner-approved step
# (docs/handoffs/2026-10-22/BACKUP.md). This script never writes to /etc/vpn.
#
# Config (environment, or /etc/vpn/backup.env): BACKUP_RECIPIENTS BACKUP_SFTP_TARGET (user@host:dir) BACKUP_SFTP_PORT
# BACKUP_SFTP_KEY (a key restricted to the backup account) BACKUP_SFTP_KNOWN_HOSTS (pinned host key: strict checking)
# BACKUP_PATHS (default below). Tests: BACKUP_ROOT SFTP_BIN.
set -Eeuo pipefail
umask 077

ENV_FILE="${BACKUP_ENV_FILE:-/etc/vpn/backup.env}"
if [ -e "$ENV_FILE" ]; then
  # it is sourced as root: only its owner (root in production) may be able to write it
  [ "$(stat -c %u "$ENV_FILE")" = "$(id -u)" ] || { echo "state-backup: $ENV_FILE is not owned by uid $(id -u); refusing to source it" >&2; exit 1; }
  case "$(stat -c %A "$ENV_FILE")" in ?????w????|????????w?) echo "state-backup: $ENV_FILE is group/world-writable; refusing to source it" >&2; exit 1;; esac
  # shellcheck disable=SC1090
  . "$ENV_FILE"
fi
ROOT="${BACKUP_ROOT:-/}"
RECIPIENTS="${BACKUP_RECIPIENTS:-/etc/vpn/backup-recipients.txt}"
PATHS="${BACKUP_PATHS:-etc/vpn etc/letsencrypt var/lib/singbox-vpn}"
SFTP_BIN="${SFTP_BIN:-sftp}"
TMP_BASE="${TMPDIR:-/tmp}"

die() { echo "state-backup: $*" >&2; exit 1; }
WORK=""; PARTIAL=""
trap 'rm -rf -- "${WORK:-}" "${PARTIAL:-}"' EXIT

check_recipients() {
  [ -s "$RECIPIENTS" ] || die "no recipient: $RECIPIENTS is missing or empty (the owner's PUBLIC key goes there)"
  if grep -qE 'AGE-SECRET-KEY|PRIVATE KEY' "$RECIPIENTS"; then
    die "$RECIPIENTS contains a PRIVATE key; it must hold public recipients only (and that key must not stay on this host)"
  fi
}

cmd_create() {
  local out="${1:?OUT_DIR}" host stamp name present=() p
  check_recipients
  mkdir -p "$out"
  for p in $PATHS; do [ -e "$ROOT/$p" ] && present+=("$p"); done
  [ "${#present[@]}" -gt 0 ] || die "none of the state paths exist under $ROOT ($PATHS): nothing to back up"
  host="$(hostname -s 2>/dev/null || echo host)"; stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  name="$host-state-$stamp.tar.age"
  WORK="$(mktemp -d "$TMP_BASE/state-backup.XXXXXX")"; PARTIAL="$out/$name.tmp"
  # manifest of per-file hashes (hashes of high-entropy secrets reveal nothing); it travels inside the ciphertext
  (cd "$ROOT" && find "${present[@]}" -type f -print0 | sort -z | xargs -0 -r sha256sum) >"$WORK/BACKUP-MANIFEST.sha256"
  tar --create --numeric-owner -C "$ROOT" "${present[@]}" -C "$WORK" BACKUP-MANIFEST.sha256 \
    | age --encrypt -R "$RECIPIENTS" -o "$out/$name.tmp"
  mv "$out/$name.tmp" "$out/$name"
  (cd "$out" && sha256sum "$name" >"$name.sha256")
  echo "created $out/$name ($(wc -l <"$WORK/BACKUP-MANIFEST.sha256") files: ${present[*]})"
}

cmd_verify() {
  local f="${1:?FILE}"
  [ -f "$f" ] && [ -f "$f.sha256" ] || die "missing $f or $f.sha256"
  [ "$(head -c 21 "$f")" = "age-encryption.org/v1" ] || die "$f is not age ciphertext"
  (cd "$(dirname "$f")" && sha256sum --quiet -c "$(basename "$f").sha256") || die "checksum mismatch for $f"
  echo "verified $f"
}

sftp_batch() { # batch-file
  [ -n "${BACKUP_SFTP_TARGET:-}" ] || die "BACKUP_SFTP_TARGET is not configured"
  [[ "$BACKUP_SFTP_TARGET" =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:[A-Za-z0-9._/-]+$ ]] && [[ "$BACKUP_SFTP_TARGET" != *..* ]] \
    || die "BACKUP_SFTP_TARGET must look like user@host:dir (letters, digits, . _ - / only, no '..')"
  [ -r "${BACKUP_SFTP_KNOWN_HOSTS:-}" ] || die "BACKUP_SFTP_KNOWN_HOSTS (pinned storage host key) is not configured"
  "$SFTP_BIN" -b "$1" -o BatchMode=yes -o StrictHostKeyChecking=yes -o IdentitiesOnly=yes \
    -o "UserKnownHostsFile=$BACKUP_SFTP_KNOWN_HOSTS" -i "${BACKUP_SFTP_KEY:?BACKUP_SFTP_KEY}" \
    -P "${BACKUP_SFTP_PORT:-22}" "${BACKUP_SFTP_TARGET%%:*}"
}

cmd_upload() {
  local f="${1:?FILE}" dir base want got
  cmd_verify "$f" >/dev/null
  dir="${BACKUP_SFTP_TARGET#*:}"; base="$(basename "$f")"
  [[ "$base" =~ ^[A-Za-z0-9._-]+$ ]] || die "unsafe archive name $base"
  [[ "${BACKUP_SFTP_TARGET:-}" =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:[A-Za-z0-9._/-]+$ ]] && [[ "$BACKUP_SFTP_TARGET" != *..* ]] \
    || die "BACKUP_SFTP_TARGET must look like user@host:dir (letters, digits, . _ - / only, no '..')"
  WORK="$(mktemp -d "$TMP_BASE/state-backup.XXXXXX")"
  printf -- '-mkdir %s\nput %s %s/%s\nput %s %s/%s\n' "$dir" "$f" "$dir" "$base" "$f.sha256" "$dir" "$base.sha256" >"$WORK/put"
  sftp_batch "$WORK/put" >/dev/null || die "upload failed"
  printf 'get %s/%s %s/%s\n' "$dir" "$base" "$WORK" "$base" >"$WORK/get"
  sftp_batch "$WORK/get" >/dev/null || die "download-back failed"
  want="$(cut -d' ' -f1 "$f.sha256")"; got="$(sha256sum "$WORK/$base" | cut -d' ' -f1)"
  [ "$want" = "$got" ] || die "remote copy differs from the local archive (want $want, got $got)"
  echo "VERIFIED off-host: ${BACKUP_SFTP_TARGET%%:*}:$dir/$base sha256 $got"
}

cmd_restore_drill() {
  local f="${1:?FILE}" id="${2:?IDENTITY}" dir="${3:?DIR}" real n
  cmd_verify "$f" >/dev/null
  [ -d "$dir" ] && [ ! -L "$dir" ] || die "$dir must be an existing directory"
  real="$(cd "$dir" && pwd -P)"
  case "$real" in /|/etc|/etc/*|/var/lib/singbox-vpn*|/opt|/opt/*|/usr*|/boot*|/root) die "refusing to restore into live path $real";; esac
  [ -z "$(ls -A "$real")" ] || die "$real is not empty"
  age --decrypt -i "$id" "$f" | tar --extract --numeric-owner -C "$real" || die "decryption or extraction failed (wrong identity?)"
  (cd "$real" && sha256sum --quiet -c BACKUP-MANIFEST.sha256) >/dev/null 2>&1 || die "restored files do not match the manifest"
  n="$(wc -l <"$real/BACKUP-MANIFEST.sha256")"
  echo "RESTORE DRILL PASS: $n files decrypted into $real and verified against the embedded manifest"
}

cmd_prune() {
  local dir="${1:?DIR}" keep="${2:?KEEP}" all n i
  [[ "$keep" =~ ^[0-9]+$ ]] && [ "$keep" -ge 1 ] || die "KEEP must be a number >= 1"
  [ -d "$dir" ] && [ ! -L "$dir" ] || die "$dir must be a real directory"
  shopt -s nullglob; all=("$dir"/*-state-*.tar.age); shopt -u nullglob   # names sort by their UTC stamp
  n="${#all[@]}"
  for ((i = 0; i < n - keep; i++)); do rm -f -- "${all[$i]}" "${all[$i]}.sha256"; echo "pruned ${all[$i]##*/}"; done
}

cmd_run() {
  local out="${BACKUP_OUT_DIR:-/var/backups/singbox-vpn}" line f
  line="$(cmd_create "$out")"; echo "$line"
  f="${line#created }"; f="${f%% (*}"
  ( cmd_upload "$f" ) || die "upload of $f failed: the off-host copy is NOT current"
  cmd_prune "$out" "${BACKUP_KEEP_LOCAL:-3}"
}

case "${1:-}" in
  create) shift; cmd_create "$@" ;;
  verify) shift; cmd_verify "$@" ;;
  upload) shift; cmd_upload "$@" ;;
  restore-drill) shift; cmd_restore_drill "$@" ;;
  prune) shift; cmd_prune "$@" ;;
  run) shift; cmd_run "$@" ;;
  *) sed -n 2,9p "$0"; exit 2 ;;
esac
