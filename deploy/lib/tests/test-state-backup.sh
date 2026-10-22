#!/usr/bin/env bash
# deploy/lib/backup/state-backup.sh: encrypted off-host deployment-state backup. Hermetic: a temp tree stands in for
# /etc/vpn etc., real `age` encrypts, a stub `sftp` "uploads" into a temp dir. Nothing on the host is read or written.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
S="$ROOT/deploy/lib/backup/state-backup.sh"
fail=0; ok() { echo "ok: $1"; }; bad() { echo "FAIL: $1"; fail=$((fail + 1)); }
command -v age >/dev/null && command -v age-keygen >/dev/null || { echo "FAIL: age/age-keygen not installed (required)"; exit 1; }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT

# fake deployment state with a known secret value that must never appear in plaintext outside the encrypted archive
SECRET="test-secret-$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"
mkdir -p "$T/root/etc/vpn/compat/reality" "$T/root/var/lib/singbox-vpn" "$T/root/etc/letsencrypt/live/x"
printf 'private_key = "%s"\n' "$SECRET" >"$T/root/etc/vpn/compat/reality/keys.toml"; chmod 600 "$T/root/etc/vpn/compat/reality/keys.toml"
echo 'users = 3' >"$T/root/etc/vpn/deployment.toml"
echo cert >"$T/root/etc/letsencrypt/live/x/privkey.pem"
echo '{"release":"x"}' >"$T/root/var/lib/singbox-vpn/install-state.json"

age-keygen -o "$T/owner.key" 2>/dev/null; age-keygen -y "$T/owner.key" >"$T/recipients.txt"
ssh-keygen -q -t ed25519 -N '' -C owner-backup -f "$T/owner_ssh"            # the Termius-style recipient
mkdir -p "$T/remote"
cat >"$T/sftp" <<'F'
#!/usr/bin/env bash
# stub: executes put/get/mkdir from the -b batch file against $FAKE_REMOTE
batch=""; while [ $# -gt 1 ]; do case "$1" in -b) batch="$2"; shift 2;; *) shift;; esac; done
while read -r cmd a b; do case "$cmd" in
  put) cp "$a" "$FAKE_REMOTE/${b##*/}" ;; get) cp "$FAKE_REMOTE/${a##*/}" "$b" ;; -mkdir|mkdir) : ;;
  *) [ -z "$cmd" ] || { echo "stub sftp: $cmd" >&2; exit 1; } ;; esac; done <"$batch"
F
chmod +x "$T/sftp"
run() { BACKUP_ROOT="$T/root" BACKUP_RECIPIENTS="$T/recipients.txt" BACKUP_SFTP_TARGET="u@storage.example:fin1" \
        BACKUP_SFTP_KEY="$T/owner_ssh" BACKUP_SFTP_KNOWN_HOSTS="$T/kh" SFTP_BIN="$T/sftp" FAKE_REMOTE="$T/remote" \
        TMPDIR="$T/tmp" bash "$S" "$@"; }
mkdir -p "$T/tmp" "$T/out"; : >"$T/kh"

out="$(run create "$T/out" 2>&1)"; rc=$?
arch=("$T/out"/*.tar.age); f="${arch[0]}"; [ -e "$f" ] || f=""
{ [ $rc -eq 0 ] && [ -n "$f" ] && [ -f "$f.sha256" ]; } && ok "create writes an encrypted archive and its checksum" || bad "create: rc=$rc $out"
[ "$(head -c 21 "$f")" = "age-encryption.org/v1" ] && ok "archive is age ciphertext" || bad "not age"
if grep -rqa -- "$SECRET" "$T/out" "$T/tmp" 2>/dev/null || grep -qa -- "$SECRET" <<<"$out"; then bad "plaintext secret leaked outside the ciphertext"; else ok "secret value appears nowhere in plaintext (output dir, temp dir, stdout)"; fi
[ "$(stat -c %a "$f")" = 600 ] && ok "archive mode 0600" || bad "archive mode $(stat -c %a "$f")"
[ -z "$(ls -A "$T/tmp")" ] && ok "no temporary files left" || bad "temp leftovers: $(ls -A "$T/tmp")"

run verify "$f" >/dev/null 2>&1 && ok "verify accepts the intact archive" || bad "verify rejected intact archive"
printf 'X' | dd of="$T/tampered.tar.age" bs=1 seek=200 conv=notrunc 2>/dev/null; cp "$f.sha256" "$T/tampered.tar.age.sha256"
sed -i "s#$(basename "$f")#tampered.tar.age#" "$T/tampered.tar.age.sha256"
run verify "$T/tampered.tar.age" >/dev/null 2>&1 && bad "verify accepted a tampered archive" || ok "verify rejects a modified archive"

out="$(run upload "$f" 2>&1)"; rc=$?
{ [ $rc -eq 0 ] && cmp -s "$f" "$T/remote/$(basename "$f")" && grep -q 'VERIFIED' <<<"$out"; } && ok "upload verifies the remote copy by downloading it back" || bad "upload: rc=$rc $out"
printf 'Z' >>"$T/remote/$(basename "$f")"
cat >"$T/sftp2" <<F
#!/usr/bin/env bash
FAKE_REMOTE="$T/remote" "$T/sftp" "\$@"; r=\$?; printf 'Z' >>"$T/remote/$(basename "$f")"; exit \$r
F
chmod +x "$T/sftp2"
out="$(SFTP_BIN="$T/sftp2" BACKUP_ROOT="$T/root" BACKUP_RECIPIENTS="$T/recipients.txt" BACKUP_SFTP_TARGET=u@s:fin1 BACKUP_SFTP_KEY="$T/owner_ssh" BACKUP_SFTP_KNOWN_HOSTS="$T/kh" FAKE_REMOTE="$T/remote" TMPDIR="$T/tmp" bash "$S" upload "$f" 2>&1)"; rc=$?
# the stub corrupts after each transfer, so the downloaded copy differs: upload must fail
[ $rc -ne 0 ] && ok "upload fails when the downloaded-back copy differs" || bad "corrupted remote accepted: $out"

mkdir -p "$T/drill"
out="$(run restore-drill "$f" "$T/owner.key" "$T/drill" 2>&1)"; rc=$?
{ [ $rc -eq 0 ] && grep -q 'RESTORE DRILL PASS' <<<"$out" && cmp -s "$T/root/etc/vpn/compat/reality/keys.toml" "$T/drill/etc/vpn/compat/reality/keys.toml"; } \
  && ok "restore drill decrypts with the owner key and every file matches its manifest hash" || bad "drill: rc=$rc $out"
grep -qa -- "$SECRET" <<<"$out" && bad "drill printed secret content" || ok "drill prints no secret content"
[ "$(stat -c %a "$T/drill/etc/vpn/compat/reality/keys.toml")" = 600 ] && ok "restored file keeps mode 0600" || bad "mode not kept"
out="$(run restore-drill "$f" "$T/owner.key" "$T/drill" 2>&1)"; [ $? -ne 0 ] && ok "drill refuses a non-empty target" || bad "drill overwrote a non-empty dir"
out="$(run restore-drill "$f" "$T/owner.key" /etc/vpn 2>&1)"; [ $? -ne 0 ] && ok "drill refuses a live system path" || bad "drill accepted /etc/vpn"
age-keygen -o "$T/wrong.key" 2>/dev/null; mkdir -p "$T/drill2"
run restore-drill "$f" "$T/wrong.key" "$T/drill2" >/dev/null 2>&1 && bad "wrong key decrypted" || ok "a wrong identity cannot decrypt"

# ssh-ed25519 recipient (Termius-generated dedicated backup key) works end to end
cp "$T/owner_ssh.pub" "$T/recipients.txt"; rm -f "$T/out"/*
run create "$T/out" >/dev/null 2>&1; arch=("$T/out"/*.tar.age); f2="${arch[0]}"; mkdir -p "$T/drill3"
run restore-drill "$f2" "$T/owner_ssh" "$T/drill3" 2>&1 | grep -q 'RESTORE DRILL PASS' && ok "ssh-ed25519 recipient: encrypt and restore drill pass" || bad "ssh recipient path"

# recipients file must hold public keys only
cp "$T/owner.key" "$T/recipients.txt"
run create "$T/out" >/dev/null 2>&1 && bad "accepted a PRIVATE key as recipient" || ok "refuses a recipients file containing a private key"
: >"$T/recipients.txt"
run create "$T/out" >/dev/null 2>&1 && bad "accepted empty recipients" || ok "refuses to create without a recipient"

[ "$fail" -eq 0 ] && echo "PASS: test-state-backup.sh" || { echo "FAIL: $fail"; exit 1; }
