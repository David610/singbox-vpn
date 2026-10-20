#!/usr/bin/env bash
# ssh-harden.sh must never disable password login without logged proof of an
# independent public-key login with the exact fingerprint, must validate and
# verify the effective setting, and must roll back. Stubs only; no host change.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
H="$REPO_ROOT/deploy/lib/ssh-harden.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/dropin.d" "$T/bin"
: >"$T/log"
# sshd stub: -T reflects the drop-in unless FORCE_YES (an earlier override); -t fails if BAD_CONF exists.
cat >"$T/bin/sshd" <<EOF
#!/usr/bin/env bash
case "\$1" in
  -t) [ ! -e "$T/BAD_CONF" ] ;;
  -T) if [ -e "$T/dropin.d/00-singbox-vpn-hardening.conf" ] && [ ! -e "$T/FORCE_YES" ]; then echo "passwordauthentication no"; else echo "passwordauthentication yes"; fi
      echo "permitrootlogin yes"; echo "kbdinteractiveauthentication no" ;;
esac
EOF
chmod +x "$T/bin/sshd"
export SSH_HARDEN_USER=root SSH_HARDEN_AUTH_KEYS="$T/ak" SSH_HARDEN_DROPIN_DIR="$T/dropin.d" \
  SSH_HARDEN_BACKUP_DIR="$T/state" SSH_HARDEN_SSHD="$T/bin/sshd" \
  SSH_HARDEN_LOG_CMD="cat $T/log" SSH_HARDEN_RELOAD_CMD=true
ssh-keygen -q -t ed25519 -N '' -f "$T/k1" -C t1; ssh-keygen -q -t ed25519 -N '' -f "$T/k2" -C t2
FP1="$(ssh-keygen -l -f "$T/k1.pub" | awk '{print $2}')"; FP2="$(ssh-keygen -l -f "$T/k2.pub" | awk '{print $2}')"
DROP="$T/dropin.d/00-singbox-vpn-hardening.conf"

bash "$H" add-key --pubkey-file "$T/k1" >/dev/null 2>&1 && bad "accepted a PRIVATE key file" || ok "private key refused"
bash "$H" add-key --pubkey "garbage" >/dev/null 2>&1 && bad "accepted garbage" || ok "garbage refused"
bash "$H" add-key --pubkey-file "$T/k1.pub" >/dev/null && ok "public key added" || bad "add-key failed"
bash "$H" add-key --pubkey-file "$T/k1.pub" >/dev/null; [ "$(grep -c . "$T/ak")" = 1 ] && ok "add-key is idempotent" || bad "duplicate key line"
[ "$(stat -c %a "$T/ak")" = 600 ] && ok "authorized_keys mode 0600" || bad "authorized_keys mode wrong"

bash "$H" disable-passwords >/dev/null 2>&1 && bad "disabled passwords with NO proof of key login" || ok "refused: no key login logged"
[ ! -e "$DROP" ] && ok "no drop-in written without proof" || bad "drop-in written without proof"

echo "Accepted publickey for root from 203.0.113.9 port 5 ssh2: ED25519 $FP2" >>"$T/log"
bash "$H" disable-passwords >/dev/null 2>&1 && bad "accepted a login by a DIFFERENT key as proof" || ok "refused: logged login was another key's fingerprint"
echo "Accepted password for root from 203.0.113.9 port 6 ssh2" >>"$T/log"
bash "$H" verify >/dev/null 2>&1 && bad "a password login counted as key proof" || ok "password login is not proof"

: >"$T/BAD_CONF"; echo "Accepted publickey for root from 203.0.113.9 port 7 ssh2: ED25519 $FP1" >>"$T/log"
bash "$H" disable-passwords >/dev/null 2>&1 && bad "applied a config sshd -t rejects" || ok "sshd -t failure: not applied"
[ ! -e "$DROP" ] && ok "rolled back after sshd -t failure" || bad "drop-in left after sshd -t failure"
rm -f "$T/BAD_CONF"

: >"$T/FORCE_YES"
bash "$H" disable-passwords >/dev/null 2>&1 && bad "reported success while an override keeps passwords on" || ok "effective value still yes: reported failure"
[ ! -e "$DROP" ] && ok "rolled back when effective value did not change" || bad "drop-in left despite no effect"
rm -f "$T/FORCE_YES"

bash "$H" disable-passwords >/dev/null 2>&1 && ok "disable-passwords succeeds with proof + valid config" || bad "disable-passwords failed with proof"
grep -q '^PasswordAuthentication no' "$DROP" && "$T/bin/sshd" -T | grep -q 'passwordauthentication no' && ok "effective passwordauthentication=no" || bad "not effective"
bash "$H" rollback >/dev/null 2>&1; [ ! -e "$DROP" ] && "$T/bin/sshd" -T | grep -q 'passwordauthentication yes' && ok "rollback restores password login" || bad "rollback failed"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
