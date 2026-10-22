#!/usr/bin/env bash
# Regression (I-44): nginx does NOT inherit server-level add_header into a
# location that defines its own add_header. The subscription vhost set HSTS at
# server level, but /sub/ and /v1/provision/ both add Cache-Control, so HSTS
# was silently dropped from every credential-bearing response.
# Rule enforced: every location block with an add_header must itself carry
# Strict-Transport-Security.
set -Eeuo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
T="$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template"
python3 -I - "$T" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
bad = []
for m in re.finditer(r'location\s+[^{]+\{(.*?)\n    \}', text, re.S):
    body = m.group(1)
    head = m.group(0).split('{')[0].strip()
    if 'add_header' in body and 'Strict-Transport-Security' not in body:
        bad.append(head)
if bad:
    print("FAIL: location(s) with add_header but no HSTS (server-level HSTS is not inherited):", bad)
    sys.exit(1)
print("ok: every location that sets add_header also sets HSTS")
PY
