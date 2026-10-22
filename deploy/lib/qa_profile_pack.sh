#!/usr/bin/env bash
# Show the QA subscription profiles for ONE dedicated QA user as terminal QR codes, for a native-client test (iPhone/Android).
# Run it yourself on the VPS, in your own terminal:   sudo bash deploy/lib/qa_profile_pack.sh <qa-user-id>
#
# Prints to the TERMINAL ONLY: writes no file, no log, no clipboard; the QR codes and links contain the user's credentials, so
# do not screenshot the terminal, paste it into a chat, or share the session. Use a dedicated QA user (create one with
# `vpn user create --name qa-<date>` and remove it with `vpn user remove <id>` after the test). Running this MINTS A NEW
# subscription token for that user (the previous URL stops working), exactly like `vpn user rotate-token`.
#
#   1. REALITY only   ?format=singbox&compat=hiddify-pinned   Hiddify: Add profile -> scan. One route, so the transport under
#                                                              test is REALITY for certain.
#   2. Hysteria2 only  the single hysteria2:// share link       Hiddify: Add from clipboard/scan. One route -> Hysteria2 for certain.
#   3. Both            ?format=hiddify                          what a normal user imports (Hiddify then balances new connections).
set -Eeuo pipefail
USER_ID="${1:-}"
[ -n "$USER_ID" ] || { echo "usage: $0 <qa-user-id>   (see: vpn user list)" >&2; exit 2; }
[[ "$USER_ID" =~ ^[A-Za-z0-9_.-]+$ ]] || { echo "refusing: unexpected characters in the user id" >&2; exit 2; }
if command -v qrencode >/dev/null; then QR=1; else QR=0; echo "qrencode is not installed (apt install qrencode): printing links only; use 'Add from clipboard'." >&2; fi
umask 077

url="$(vpn user rotate-token "$USER_ID" --json | python3 -I -c 'import json,sys;print(json.load(sys.stdin)["subscription_url"])')"
[[ "$url" =~ ^https://[^/?#]+(:[0-9]+)?/sub/[A-Za-z0-9_-]+$ ]] || { echo "unexpected subscription URL shape; refusing to print it" >&2; exit 1; }
hy="$(vpn user links "$USER_ID" | grep -m1 '^hysteria2://' || true)"

show() { # $1=title $2=data
  printf '\n=== %s ===\n' "$1"
  if [ "$QR" = 1 ]; then printf '%s' "$2" | qrencode -t ansiutf8 -m 1; fi
  printf '%s\n' "$2"
}
echo "QA profiles for $USER_ID  (credentials: terminal only, do not share)"
show "1. REALITY only (Hiddify, pinned single route)" "${url}?format=singbox&compat=hiddify-pinned"
if [ -n "$hy" ]; then show "2. Hysteria2 only (single share link)" "$hy"; else echo "(no hysteria2 link available for this user)" >&2; fi
show "3. Both transports (URI list)" "${url}?format=hiddify"
echo
echo "After the test remove the QA user:  vpn user remove $USER_ID"
