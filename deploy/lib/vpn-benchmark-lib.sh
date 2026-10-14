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
# literal `format` value that matches none of `singbox`/`uri`/`hiddify`
# — a guaranteed 400, which `curl -f` then silently swallowed as
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

# ---------------------------------------------------------------------
# Isolated (non-mutating) protocol benchmark helpers.
#
# The default tunnel layer of vpn-benchmark.sh runs a DISPOSABLE sing-box
# server on loopback with throwaway key material and a throwaway
# credential, plus a sing-box client against it. Nothing in the user
# store, the rendered production config, or the running sing-box.service
# is read for secrets or changed, so the run cannot restart the data plane
# or disconnect anyone. These builders produce the two documents; they are
# pure (jq only) so deploy/lib/tests/test-vpn-benchmark-lib.sh can check
# their shape without a sing-box binary.
#
# The inbound shapes mirror compat_config::server::render_singbox_server_config
# (VLESS+REALITY with xtls-rprx-vision and multiplex accepted; Hysteria2
# with TLS, optional Salamander and optional bandwidth caps) and the client shapes
# mirror compat_config::render's contract outbounds (uTLS chrome
# fingerprint, REALITY public key/short id, Hysteria2 with verified TLS).
# The disposable server keeps the C-16 idea in miniature — resolve, then
# reject private/loopback destinations — so a local process holding the
# throwaway credential still cannot use it to reach node-local services.
# ---------------------------------------------------------------------

# Reads one `key = value` from a `[section]` of a simple TOML file (the
# shapes deployment.toml uses for [reality]/[hysteria2]). Prints the value
# without quotes; prints nothing if absent. Never evaluates the file.
# Usage: vpn_benchmark_toml_value FILE SECTION KEY
vpn_benchmark_toml_value() {
  local file="$1" section="$2" key="$3"
  [ -r "$file" ] || return 0
  awk -v want="[$section]" -v key="$key" '
    /^[[:space:]]*\[/ {
      line = $0; gsub(/[[:space:]]/, "", line); in_section = (line == want); next
    }
    in_section {
      line = $0
      sub(/#.*/, "", line)
      split(line, kv, "=")
      k = kv[1]; gsub(/[[:space:]]/, "", k)
      if (k == key) {
        v = substr(line, index(line, "=") + 1)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
        gsub(/^"|"$/, "", v)
        print v
        exit
      }
    }
  ' "$file"
}

# Server document for the disposable instance.
# Usage: vpn_benchmark_isolated_server_config REALITY_PORT HY2_PORT \
#          REALITY_PRIVATE_KEY SHORT_ID UUID HANDSHAKE_SERVER HANDSHAKE_PORT \
#          HY2_PASSWORD CERT_PATH KEY_PATH [OBFS_PASSWORD] [UP_MBPS DOWN_MBPS]
vpn_benchmark_isolated_server_config() {
  local reality_port="$1" hy2_port="$2" private_key="$3" short_id="$4" uuid="$5"
  local handshake_server="$6" handshake_port="$7" hy2_password="$8" cert="$9" key="${10}"
  local obfs="${11:-}" up="${12:-}" down="${13:-}"
  jq -n \
    --argjson rport "$reality_port" --argjson hport "$hy2_port" \
    --arg pk "$private_key" --arg sid "$short_id" --arg uuid "$uuid" \
    --arg hs "$handshake_server" --argjson hsp "$handshake_port" \
    --arg hpw "$hy2_password" --arg cert "$cert" --arg key "$key" \
    --arg obfs "$obfs" --arg up "$up" --arg down "$down" '
    ({type: "hysteria2", tag: "hysteria2-in", listen: "127.0.0.1", listen_port: $hport,
      users: [{name: "benchmark", password: $hpw}],
      tls: {enabled: true, certificate_path: $cert, key_path: $key}}
     | if $obfs != "" then .obfs = {type: "salamander", password: $obfs} else . end
     | if ($up != "" and $down != "")
       then .up_mbps = ($up|tonumber) | .down_mbps = ($down|tonumber)
       else . end) as $hy2
    | {
        log: {level: "fatal", timestamp: true},
        inbounds: [
          {type: "vless", tag: "vless-reality-in", listen: "127.0.0.1", listen_port: $rport,
           users: [{name: "benchmark", uuid: $uuid, flow: "xtls-rprx-vision"}],
           tls: {enabled: true, server_name: $hs,
                 reality: {enabled: true, handshake: {server: $hs, server_port: $hsp},
                           private_key: $pk, short_id: [$sid]}},
           multiplex: {enabled: true}},
          $hy2
        ],
        outbounds: [{type: "direct", tag: "direct"}],
        route: {
          final: "direct",
          rules: [
            {action: "resolve"},
            {ip_is_private: true, action: "reject", method: "default"}
          ]
        }
      }'
}

# Client document for one transport of the disposable instance.
# Usage: vpn_benchmark_isolated_client_config vless-reality SOCKS_PORT \
#          SERVER_PORT PUBLIC_KEY SHORT_ID UUID HANDSHAKE_SERVER
#        vpn_benchmark_isolated_client_config hysteria2 SOCKS_PORT \
#          SERVER_PORT HY2_PASSWORD CERT_PATH SERVER_NAME [OBFS_PASSWORD]
vpn_benchmark_isolated_client_config() {
  local transport="$1" socks_port="$2" server_port="$3"
  local outbound
  case "$transport" in
    vless-reality)
      outbound="$(jq -n --argjson port "$server_port" --arg pub "$4" --arg sid "$5" \
        --arg uuid "$6" --arg sni "$7" '
        {type: "vless", tag: "bench", server: "127.0.0.1", server_port: $port, uuid: $uuid,
         flow: "xtls-rprx-vision",
         tls: {enabled: true, server_name: $sni,
               utls: {enabled: true, fingerprint: "chrome"},
               reality: {enabled: true, public_key: $pub, short_id: $sid}}}')"
      ;;
    hysteria2)
      outbound="$(jq -n --argjson port "$server_port" --arg pw "$4" --arg cert "$5" \
        --arg sni "$6" --arg obfs "${7:-}" '
        {type: "hysteria2", tag: "bench", server: "127.0.0.1", server_port: $port, password: $pw,
         tls: {enabled: true, server_name: $sni, insecure: false, certificate_path: $cert}}
        | if $obfs != "" then .obfs = {type: "salamander", password: $obfs} else . end')"
      ;;
    *)
      echo "vpn_benchmark_isolated_client_config: unknown transport '$transport'" >&2
      return 4
      ;;
  esac
  jq -n --argjson socks "$socks_port" --argjson ob "$outbound" '
    {log: {level: "fatal"},
     inbounds: [{type: "mixed", tag: "in", listen: "127.0.0.1", listen_port: $socks}],
     outbounds: [$ob],
     route: {final: "bench"}}'
}
