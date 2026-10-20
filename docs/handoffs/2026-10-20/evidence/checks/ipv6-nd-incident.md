# IPv6 neighbour-discovery incident (I-54) — recorded observations, staging node 62.238.46.190

Evidence class: server verified (observations made live on the staging node), plus the behavioural netns test.

1. After installing the published/branch egress-isolation nftables table: `ip -6 neigh` showed the default gateway `fe80::1 router FAILED`; `ping -6 fe80::1%eth0` 100% loss; `curl -6 https://ifconfig.me` empty. The same host had working IPv6 before the install (baseline probes 07:40 UTC used the IPv6 egress).
2. ufw ruled out: `ufw --force disable` -> IPv6 still dead.
3. `nft delete table inet arcana_egress_isolation` -> gateway ping 2/2, `curl -6` returned the node's IPv6 address immediately; the table restored by `systemctl restart vpn-egress-isolation.service` reproduced the breakage.
4. Mechanism: the table's first rule `meta skuid != <sing-box uid> accept` does not match packets without a socket owner (kernel-originated neighbour discovery), which then hit `ip6 daddr { fe80::/10, ff00::/8, ... } reject`.
5. Fix (uid-scoped positive matches + counters): applied live -> flushed neighbour cache, gateway ping 2/2, `curl -6` = node IPv6, YouTube over IPv6 HTTP 200.
6. Regression: `deploy/lib/tests/test-egress-isolation-ipv6-nd.sh` (two network namespaces, the REAL script's ruleset): red against the original script, green after; also proves the isolation still rejects the sing-box uid and leaves root untouched.
7. Observed later: a published-rc.2 `update.sh --repair` reinstalled the old script and IPv6 died again (`reboot_check.py` capture showed `nft_isolation.uid_scoped: false` and gateway neighbour `INCOMPLETE`); re-applying the branch script restored it. Only a corrected release fixes nodes durably.
