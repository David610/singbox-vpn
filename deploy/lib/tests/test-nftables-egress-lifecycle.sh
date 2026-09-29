#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cat >"$TMP/id" <<'MOCK'
#!/usr/bin/env bash
if [ "$1" = "-u" ] && [ "$#" -eq 1 ]; then echo 0; else echo 991; fi
MOCK
cat >"$TMP/nft" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$NFT_LOG"
if [ "$1" = "list" ]; then [ -f "$NFT_PRESENT" ]; exit; fi
if [ "$1" = "delete" ]; then rm -f "$NFT_PRESENT"; exit; fi
if [ "$1" = "-f" ]; then cat >>"$NFT_RULES"; touch "$NFT_PRESENT"; exit; fi
exit 1
MOCK
chmod +x "$TMP/id" "$TMP/nft"
export PATH="$TMP:$PATH" NFT_LOG="$TMP/log" NFT_RULES="$TMP/rules" NFT_PRESENT="$TMP/present"
SCRIPT="$ROOT/deploy/almalinux/nftables-egress-isolation.sh"
"$SCRIPT"
"$SCRIPT"
"$SCRIPT" --remove
"$SCRIPT" --remove
rg -q '^table inet arcana_egress_isolation' "$NFT_RULES"
rg -q 'ip daddr' "$NFT_RULES"
rg -q 'ip6 daddr' "$NFT_RULES"
! rg -q 'flush ruleset|delete table (ip|ip6) ' "$NFT_RULES" "$NFT_LOG"
rg -q 'vpn-egress-isolation.service' "$ROOT/deploy/almalinux/install.sh"
rg -q 'vpn-egress-isolation.service' "$ROOT/deploy/almalinux/update.sh"
rg -q 'vpn-egress-isolation.service' "$ROOT/deploy/almalinux/uninstall.sh"
rg -q '^ExecStop=.* --remove$' "$ROOT/deploy/almalinux/systemd/vpn-egress-isolation.service"
echo 'nftables egress lifecycle tests passed'
