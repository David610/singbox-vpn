#!/usr/bin/env bash
# Own public address (I-15 / I-31): detection and deployment.toml backfill,
# shared by install.sh and update.sh. Requires the caller to define log, warn,
# die, DEPLOYMENT_TOML and PUBLIC_HOST, and to have sourced preflight.sh.
# Sourced, never executed.

# ---------------------------------------------------------------------
# Own public address (I-15). Records the node's actual public IPv4/IPv6 in
# deployment.toml so the renderer emits the own-address reject rule (a tunnel
# user must not reach the node's own services through its public address)
# whatever form PUBLIC_HOST takes. Evidence order: operator-supplied
# PUBLIC_IPV4/PUBLIC_IPV6, then addresses configured on this host's
# interfaces, then (only when no interface carries one, or several do) an
# address two independent services agree on. DNS is only a tie-break between
# addresses already proven local; it is never evidence of ownership.
# ---------------------------------------------------------------------
resolve_own_public_addresses() {
  OWN_PUBLIC_IPV4=""
  OWN_PUBLIC_IPV6=""
  local fam var supplied local_addrs egress dns pick extra candidate count
  for fam in 4 6; do
    var="PUBLIC_IPV$fam"
    supplied="${!var:-}"
    pick=""
    if [ -n "$supplied" ]; then
      [ "$(preflight_ip_family "$supplied" 2>/dev/null)" = "$fam" ] \
        || die "$var='$supplied' is not a valid IPv$fam address literal."
      preflight_is_public_ip "$supplied" \
        || warn "$var=$supplied is not a public address: the own-address reject rule will not protect a public service with it."
      pick="$supplied"
    else
      local_addrs="$(preflight_local_public_addrs "$fam")"
      count=0
      [ -z "$local_addrs" ] || count="$(printf '%s\n' "$local_addrs" | wc -l)"
      egress=""
      if [ "$fam" = 4 ] && [ "$count" -ne 1 ]; then
        egress="$(preflight_detect_egress_ip_confirmed 4 2>/dev/null)" || egress=""
        if [ -z "$egress" ] && [ -n "${PUBLIC_IP:-}" ] && preflight_is_public_ip "$PUBLIC_IP"; then
          egress="$PUBLIC_IP"
        fi
      fi
      candidate="$(printf '%s\n%s\n' "$local_addrs" "$egress" | sed '/^$/d' | sort -u)"
      if [ -n "$candidate" ]; then
        dns="$(preflight_resolve_host_addrs "$PUBLIC_HOST" 2>/dev/null || true)"
        pick="$(printf '%s\n' "$candidate" | grep -Fxf <(printf '%s\n' "$dns") 2>/dev/null | head -n 1 || true)"
        [ -n "$pick" ] || pick="${egress:-$(printf '%s\n' "$candidate" | head -n 1)}"
        extra="$(printf '%s\n' "$candidate" | grep -Fxv "$pick" | tr '\n' ' ' || true)"
        [ -z "$extra" ] || warn "this host has more public IPv$fam addresses than deployment.toml can record: ${extra}- the own-address reject rule covers only $pick."
      fi
    fi
    if [ "$fam" = 4 ]; then OWN_PUBLIC_IPV4="$pick"; else OWN_PUBLIC_IPV6="$pick"; fi
  done
  export OWN_PUBLIC_IPV4 OWN_PUBLIC_IPV6
  if [ -z "$OWN_PUBLIC_IPV4" ] && [ -z "$OWN_PUBLIC_IPV6" ]; then
    if [ -f "$DEPLOYMENT_TOML" ] || [ "${SINGBOX_VPN_ALLOW_UNKNOWN_OWN_ADDRESS:-0}" = 1 ]; then
      warn "could not establish this node's own public address: tunnel users will be able to reach services bound to it until public_ipv4/public_ipv6 are set in $DEPLOYMENT_TOML (then run: vpn-admin render-config)."
    else
      die "could not establish this node's own public IPv4/IPv6 address, so the own-address protection (tunnel users must not reach this node's own services) cannot be enabled. Re-run with PUBLIC_IPV4=<this server's public address> (and PUBLIC_IPV6=... if it has one), or set SINGBOX_VPN_ALLOW_UNKNOWN_OWN_ADDRESS=1 to install WITHOUT that protection."
    fi
  else
    log "own public address: IPv4=${OWN_PUBLIC_IPV4:-none} IPv6=${OWN_PUBLIC_IPV6:-none}"
  fi
}

# Writes public_ipv4/public_ipv6 as top-level keys of the rendered file $1,
# after subscription_host (or ahead of the first table). Nothing is written
# when both values are empty.
insert_own_address_keys() {
  local file="$1" v4="${2:-}" v6="${3:-}" tmp
  [ -n "$v4$v6" ] || return 0
  tmp="$file.addr.$$"
  awk -v v4="$v4" -v v6="$v6" '
    function emit() {
      if (done) return
      print "# This node'"'"'s own public address(es), recorded by install.sh: the tunnel-user"
      print "# reject rule for the node itself is rendered from them. Update on IP change."
      if (v4 != "") print "public_ipv4 = \"" v4 "\""
      if (v6 != "") print "public_ipv6 = \"" v6 "\""
      done = 1
    }
    { if (!done && $0 ~ /^[[:space:]]*\[/) emit(); print }
    !done && $0 ~ /^subscription_host[[:space:]]*=/ { emit() }
    END { emit() }
  ' "$file" >"$tmp" && cat "$tmp" >"$file" && rm -f "$tmp"
}

# Repair/update of an existing deployment: add the keys only when neither is
# present. A configured value (even one the operator set on purpose) is never
# replaced, and a failed detection never aborts a repair.
backfill_own_public_addresses() {
  [ -f "$DEPLOYMENT_TOML" ] || return 0
  if grep -Eq '^[[:space:]]*public_ipv[46][[:space:]]*=' "$DEPLOYMENT_TOML"; then
    return 0
  fi
  if [ -z "${OWN_PUBLIC_IPV4:-}${OWN_PUBLIC_IPV6:-}" ]; then
    resolve_own_public_addresses
  fi
  if [ -z "${OWN_PUBLIC_IPV4:-}${OWN_PUBLIC_IPV6:-}" ]; then
    warn "$DEPLOYMENT_TOML has no public_ipv4/public_ipv6 and none could be detected."
    return 0
  fi
  local tmp="$DEPLOYMENT_TOML.addr.$$"
  cp -p "$DEPLOYMENT_TOML" "$tmp"
  insert_own_address_keys "$tmp" "${OWN_PUBLIC_IPV4:-}" "${OWN_PUBLIC_IPV6:-}"
  cat "$tmp" >"$DEPLOYMENT_TOML" && rm -f "$tmp"
  log "recorded this node's own public address in $DEPLOYMENT_TOML (IPv4=${OWN_PUBLIC_IPV4:-none} IPv6=${OWN_PUBLIC_IPV6:-none})"
}
