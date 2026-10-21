#!/usr/bin/env bash
# qa_profile_pack.sh prints credentials to the terminal ONLY: no files, safe URL shapes, user-id validation.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; S="$HERE/../qa_profile_pack.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT; F=0
ok() { echo "ok   $1"; }; bad() { echo "FAIL $1"; F=$((F + 1)); }
mkdir "$T/bin" "$T/work"
cat >"$T/bin/vpn" <<'EOF'
#!/bin/sh
case "$1 $2" in
  "user rotate-token") echo "{\"id\":\"$3\",\"subscription_url\":\"${FAKE_URL:-https://node.example.com:8443/sub/TOKEN_abc-123}\"}";;
  "user links") echo "vless://U@node.example.com:443?x=1#R"; echo "hysteria2://P@node.example.com:443?sni=node.example.com#H";;
esac
EOF
cat >"$T/bin/qrencode" <<'EOF'
#!/bin/sh
echo "[QR:$(cat)]"
EOF
chmod +x "$T/bin/"*
run() { ( cd "$T/work" && PATH="$T/bin:$PATH" bash "$S" "$@" 2>"$T/err" ); }
before="$(find "$T" -type f | sort | tr '\n' ' ')"
out="$(run user_x)"; rc=$?
[ "$rc" = 0 ] && ok "runs with a valid id" || bad "rc=$rc"
grep -q 'sub/TOKEN_abc-123?format=singbox&compat=hiddify-pinned' <<<"$out" && ok "pinned REALITY URL built" || bad "pinned URL missing"
grep -q '^hysteria2://P@' <<<"$out" && ! grep -q '^vless://' <<<"$out" && ok "Hysteria2 item is the single hysteria2 link (no vless link)" || bad "hysteria2 item wrong"
grep -q 'sub/TOKEN_abc-123?format=hiddify$' <<<"$out" && ok "combined URI-list URL built" || bad "combined URL missing"
[ "$(grep -c '^\[QR:' <<<"$out")" = 3 ] && ok "three QR codes rendered" || bad "QR count"
after="$(find "$T" -type f | sort | tr '\n' ' ')"
[ "$(find "$T/work" -type f | wc -l)" = 0 ] && ok "no file written in the working directory" || bad "wrote files"
! grep -q 'TOKEN_abc' "$T/err" && ok "no credential on stderr" || bad "credential on stderr"
for badid in '' 'a b' 'x;rm -rf /' '../x'; do run "$badid" >/dev/null 2>&1 && bad "accepted id '$badid'" || ok "rejects id '$badid'"; done
FAKE_URL='http://evil.example.com/sub/T' run user_x >/dev/null 2>&1 && bad "accepted a non-https URL" || ok "refuses an unexpected URL shape"
FAKE_URL='https://node.example.com:8443/sub/T?x=$(id)' run user_x >/dev/null 2>&1 && bad "accepted a URL with a query/command" || ok "refuses a URL with extra parts"
exit $F
