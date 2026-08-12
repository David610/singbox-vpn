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
