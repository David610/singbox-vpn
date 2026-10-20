# Production configuration contract and drift check

`deploy/contracts/<node>.contract.json` is a non-secret, version-controlled
statement of what a node must look like. `deploy/lib/check_deployment_contract.py`
compares the host with it. **Read-only: it reports differences and never
changes the node.**

    python3 -I deploy/lib/check_deployment_contract.py [--strict] [--json out.json]

| Status | Meaning |
|---|---|
| PASS | observed equals the contract |
| FAIL | observed differs (e.g. SSH password login on, wrong subscription port/host, missing protocol, log level) |
| UNDECIDED | the contract value is `UNDECIDED` — the owner has not approved it yet (egress mode, hairpin target, release tag/commit, routing fingerprint, provider/OS). Reported with what is currently observed |
| UNOBSERVABLE | the value could not be read on this host; never counted as PASS |

Exit 1 on any FAIL; `--strict` also fails on UNDECIDED/UNOBSERVABLE, so a release
or deployment gate can require a fully decided contract. Exit status never means
"the node was changed".

**Routing fingerprint.** sha256 over `route.final` and every route rule that is
not user-specific (`auth_user` rules are skipped), in order. Adding/removing users
does not change it; losing, adding or reordering a policy rule (sniff, literal-IPv6
reject, hairpin route, C-16 rejects) does. Once the owner approves the egress
architecture, record the observed fingerprint in the contract; from then on a
reinstall, update, reboot or regeneration that silently drops or reorders routing
fails the check.

**Observed today (staging, 2026-10-20):** 11 PASS, 2 FAIL (SSH password login and
root login still permitted, I-39), 7 UNDECIDED (provider, OS, YouTube egress mode,
hairpin target, routing fingerprint, release tag/commit), 0 UNOBSERVABLE.
`dns.aaaa_published: false` is deliberate: publish an AAAA only after client->VPN
IPv6 is configured, reachable, secure and tested end to end (see `IPV6_AND_DNS.md`).
