#!/usr/bin/env bash
# Pure, sourceable helper functions for deploy/lib/vpn-benchmark.sh.
# Split out from the main script so the outbound-discovery logic can be
# unit-tested in isolation (deploy/lib/tests/test-vpn-benchmark-lib.sh)
# without spinning up a real sing-box process or a real deployment.
#
# No `set -e`/`set -u` here — this file is `source`d into a caller that
# already sets its own shell options; a sourced file changing them out
# from under the caller is a common footgun.

# Finds the single outbound of a given transport in a rendered sing-box
# client subscription JSON document (as produced by
# `compat_config::render::render_singbox_client_subscription`), and
# prints its `tag` on success.
#
# Deliberately does NOT match by tag/label: `CompatEndpoint::label` is
# operator-configurable free text (e.g. "Germany - Reality" or any other
# string an operator picks — see `crates/compat-config/src/model.rs`),
# never a stable machine-readable identifier. Matching on `type` (plus
# `tls.reality.enabled` to distinguish a REALITY `vless` outbound from a
# hypothetical future non-REALITY one) is the one property the renderer
# actually guarantees, because it comes directly from
# `CompatEndpoint::transport`/`PublicParameters`, not operator-chosen text.
#
# Usage: tag="$(vpn_benchmark_discover_outbound_tag "$sub_json" vless-reality)"
#
# Exit codes:
#   0  exactly one match — tag printed on stdout, nothing on stderr.
#   2  zero matches (this deployment has no endpoint of that transport —
#      a legitimate, expected case, not an error).
#   3  more than one match — AMBIGUOUS. Never guesses; the caller must
#      treat this as a hard failure, not silently pick the first one.
#   4  unknown transport argument (programmer error in the caller).
#   5  `sub_json` is not valid JSON, or has no `.outbounds` array.
#
# Never matches `selector`/`urltest`/`direct` outbounds: those are typed
# `"selector"`/`"urltest"`/`"direct"` in the rendered JSON, which cannot
# match either of this function's `type` filters by construction — this
# is the mechanism that satisfies "never silently benchmark the
# selector/urltest/direct outbound", not a separate exclusion list that
# could fall out of sync with the renderer.
vpn_benchmark_discover_outbound_tag() {
  local sub_json="$1" transport="$2"
  local jq_filter
  case "$transport" in
    vless-reality)
      jq_filter='[.outbounds[]? | select(.type=="vless" and ((.tls.reality.enabled // false) == true))]'
      ;;
    hysteria2)
      jq_filter='[.outbounds[]? | select(.type=="hysteria2")]'
      ;;
    *)
      echo "vpn_benchmark_discover_outbound_tag: unknown transport '$transport' (want vless-reality|hysteria2)" >&2
      return 4
      ;;
  esac

  if ! echo "$sub_json" | jq -e 'has("outbounds") and (.outbounds | type == "array")' >/dev/null 2>&1; then
    echo "vpn_benchmark_discover_outbound_tag: subscription JSON is not valid or has no .outbounds array" >&2
    return 5
  fi

  local matches count
  matches="$(echo "$sub_json" | jq -c "$jq_filter" 2>/dev/null)" || {
    echo "vpn_benchmark_discover_outbound_tag: jq filter failed against subscription JSON" >&2
    return 5
  }
  count="$(echo "$matches" | jq 'length')"

  if [ "$count" -eq 0 ]; then
    return 2
  fi
  if [ "$count" -gt 1 ]; then
    echo "vpn_benchmark_discover_outbound_tag: AMBIGUOUS — found $count outbounds matching transport '$transport' (expected exactly 1); refusing to guess which one to benchmark" >&2
    return 3
  fi

  local tag
  tag="$(echo "$matches" | jq -r '.[0].tag')"
  if [ -z "$tag" ] || [ "$tag" = "null" ]; then
    echo "vpn_benchmark_discover_outbound_tag: matched outbound has no usable tag" >&2
    return 5
  fi
  # Defense in depth, not the primary guarantee (see doc comment above):
  # the type filters above already structurally exclude these, but a
  # future renderer change that reused one of these reserved names for a
  # real per-endpoint tag should still trip this rather than silently
  # benchmark the wrong thing.
  case "$tag" in
    select|auto|direct)
      echo "vpn_benchmark_discover_outbound_tag: matched tag '$tag' is a reserved selector/urltest/direct name, not a real endpoint — refusing to benchmark it" >&2
      return 5
      ;;
  esac
  printf '%s\n' "$tag"
}

# Extracts the bare subscription token from `vpn-admin user create --json`'s
# `subscription_url` field (e.g.
# "https://host:8443/sub/AbC123token?format=hiddify"), for the
# --socks5-hostname hairpin the VLESS+REALITY/Hysteria2 sections of
# vpn-benchmark.sh build against the LOCAL subscription backend.
#
# Fixes a real defect: the previous inline `${sub_url##*/}` stripped only
# up to the last '/', which — because the URL always carries an explicit
# `?format=...` query string (see apps/admin/src/main.rs's
# `subscription_url` doc comment) — left the query string attached to the
# "token" (e.g. "AbC123token?format=hiddify"). vpn-benchmark.sh then
# re-appended its OWN `?format=singbox`, producing a doubled query string
# (".../sub/AbC123token?format=hiddify?format=singbox") that
# services/subscription's `Query<SubQuery>` extractor parses as a single
# literal `format` value that matches none of `singbox`/`uri`/`hiddify`/
# `xray` — a guaranteed 400, which `curl -f` then silently swallowed as
# "could not reach the local subscription backend", permanently and
# silently SKIPPING the entire VLESS+REALITY/Hysteria2 server-side
# protocol-overhead section on every run (see also the malformed-JSON
# path this could otherwise feed into `vpn_benchmark_discover_outbound_tag`
# above, whose un-redirected `jq` calls downstream of that stage can
# surface as a confusing "jq: parse error: Invalid numeric literal" if a
# future caller ever removes the `-f`/`2>/dev/null` guards this defect
# happened to hide behind).
#
# Exit codes: 0 on success (token printed on stdout); 2 if `sub_url`
# doesn't contain "/sub/" at all (malformed input, never guess).
vpn_benchmark_extract_token() {
  local sub_url="$1"
  case "$sub_url" in
    */sub/*) ;;
    *)
      echo "vpn_benchmark_extract_token: URL has no /sub/ path segment: $sub_url" >&2
      return 2
      ;;
  esac
  local token="${sub_url#*/sub/}"
  token="${token%%\?*}"
  if [ -z "$token" ]; then
    echo "vpn_benchmark_extract_token: extracted an empty token from: $sub_url" >&2
    return 2
  fi
  printf '%s\n' "$token"
}
