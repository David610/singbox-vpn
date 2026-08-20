#!/usr/bin/env bash
# Regression coverage for the REALITY decoy TLS 1.3 preflight probe's
# output parser. Reproduced field bug: a genuine TLS 1.3 decoy
# (www.google.com) whose later real REALITY handshake (stage 17's actual
# acceptance gate) passed cleanly still got a preflight
# "did not confirm TLS 1.3" WARN, because the probe grepped for a
# 'Protocol.*TLSv1.3' line that plain `openssl s_client` (no `-state`)
# never prints on OpenSSL 1.1.1 or 3.x — the line that IS always present
# for a completed handshake is "New, TLSv1.3, Cipher is ...". These tests
# pin the fixed parser against real captured openssl output shapes, no
# network access required.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

# shellcheck source=/dev/null
. "$ROOT/deploy/almalinux/install.sh" 2>/dev/null || true

pass=0
fail=0
check() {
  local desc="$1" expect="$2" input="$3"
  local rc=0
  reality_decoy_openssl_output_confirms_tls13 "$input" || rc=1
  if [ "$expect" = "confirm" ] && [ "$rc" -eq 0 ]; then
    echo "ok: $desc"
    pass=$((pass + 1))
  elif [ "$expect" = "no-confirm" ] && [ "$rc" -ne 0 ]; then
    echo "ok: $desc"
    pass=$((pass + 1))
  else
    echo "FAIL: $desc (rc=$rc, expected $expect)"
    fail=$((fail + 1))
  fi
}

# Real-shaped output from a genuine OpenSSL 3.x `s_client -tls1_3` probe
# against an actual TLS 1.3 server (captured verbatim shape) — this is
# exactly the www.google.com field-incident case. Note there is NO
# "Protocol  :"/"SSL-Session:" block at all — the old grep pattern could
# never have matched this.
real_openssl3_tls13_output='CONNECTED(00000003)
depth=2 O = Anthropic
verify return:1
---
Certificate chain
 0 s:CN = *.google.com
---
Peer signing digest: SHA256
Peer signature type: RSA-PSS
Server Temp Key: X25519, 253 bits
---
SSL handshake has read 3227 bytes and written 328 bytes
Verification: OK
---
New, TLSv1.3, Cipher is TLS_AES_256_GCM_SHA384
Server public key is 2048 bit
Secure Renegotiation IS NOT supported
Compression: NONE
Expansion: NONE
No ALPN negotiated
Early data was not sent
Verify return code: 0 (ok)
---
DONE'
check "genuine OpenSSL 3.x TLS 1.3 handshake (no SSL-Session block) is confirmed, not a false WARN" \
  confirm "$real_openssl3_tls13_output"

# Older OpenSSL/verbose builds that DO print the SSL-Session block must
# still be recognized (the fix must not narrow correctness, only widen
# it to the common no-session-dump case).
verbose_session_dump_output='---
New, TLSv1.3, Cipher is TLS_AES_256_GCM_SHA384
---
SSL-Session:
    Protocol  : TLSv1.3
    Cipher    : TLS_AES_256_GCM_SHA384
---
DONE'
check "output that also includes the old-style SSL-Session/Protocol block is still confirmed" \
  confirm "$verbose_session_dump_output"

# Session resumption ("Reused") must also count as a confirmed TLS 1.3
# handshake, not just a brand-new one.
reused_session_output='---
Reused, TLSv1.3, Cipher is TLS_AES_256_GCM_SHA384
---
DONE'
check "resumed (Reused) TLS 1.3 session is confirmed" confirm "$reused_session_output"

# A genuine TLS 1.2-only (or failed) server must still NOT be confirmed —
# this fix is about parsing correctness, not about weakening the check
# into always passing.
tls12_output='---
New, TLSv1.2, Cipher is ECDHE-RSA-AES128-GCM-SHA256
---
DONE'
check "a real TLS 1.2 (non-1.3) handshake is correctly NOT confirmed" no-confirm "$tls12_output"

empty_or_garbage_output='CONNECTED(00000003)
140736283355840:error:1404B42E:SSL routines:ST_CONNECT:tlsv1 alert protocol version'
check "connection failure / garbage output is NOT confirmed" no-confirm "$empty_or_garbage_output"

echo
if [ "$fail" -eq 0 ]; then
  echo "reality decoy tls1.3 probe regression passed ($pass checks)"
  exit 0
else
  echo "reality decoy tls1.3 probe regression FAILED ($fail of $((pass + fail)) checks failed)"
  exit 1
fi
