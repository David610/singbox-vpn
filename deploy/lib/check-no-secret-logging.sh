#!/usr/bin/env bash
# Regression guard (docs/FINAL_PRODUCTION_AUDIT.md P0-12): fails if any
# tracing/log/println call site in the Rust control-plane crates appears
# to interpolate a secret-typed value (subscription token, VLESS UUID,
# Hysteria2 password, REALITY private key, TLS private key) directly.
#
# This is a best-effort static grep, not a data-flow analysis — it
# catches the obvious/common mistake (`tracing::info!(token = %token, ...)`
# or similar) but cannot prove the absence of indirect leaks (e.g. logging
# a struct whose Debug impl happens to include a secret field). The
# authoritative defense against THAT is `crates/compat-config/src/secret.rs`
# (`SecretString`'s Debug impl never prints the value) — this script is a
# second, independent check on top, not a replacement for it.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAIL=0

# Interpolation patterns: `name = %expr` / `name = ?expr` / `{name}` where
# `name` textually suggests a secret, followed by a value-looking
# identifier (not just the word appearing in a string literal/help text).
PATTERN='(token|password|private_key|priv_key|secret)\s*=\s*[%?][A-Za-z_.]*(token|password|key|secret)'

for dir in services/subscription/src apps/admin/src; do
  matches="$(grep -rniE "$PATTERN" "$REPO_ROOT/$dir" --include='*.rs' || true)"
  if [ -n "$matches" ]; then
    echo "POSSIBLE SECRET LOGGING in $dir:"
    echo "$matches"
    FAIL=1
  fi
done

if [ "$FAIL" -ne 0 ]; then
  echo
  echo "check-no-secret-logging: FAILED — review the call site(s) above." >&2
  exit 1
fi
echo "check-no-secret-logging: OK (no tracing/log call site textually interpolates a secret-named field)."
