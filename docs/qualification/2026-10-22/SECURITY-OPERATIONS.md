# Security and operations report

**No current host security assertion is possible. No host was changed.**
Runtime access reports no SSH credentials or outbound identities; TCP grants
are empty. Shell Git/HTTPS cannot connect to the configured proxy. GitHub
connector access works. Neither this environment nor GitHub historical logs
establishes current VPS health, DNS, certificate state or installed services.

The 2026-10-22 handoff records five successful root password logins requiring
owner confirmation: 78.109.69.17 at 08:42, 08:44, 09:10 UTC, and
185.194.184.53 at 13:07, 13:58 UTC. These are historical host-access evidence,
not new observations. Until explained, treat the host as possibly compromised
and stop host changes. Owner should use the provider console to preserve auth
logs/snapshot, identify legitimate sessions and inspect access/persistence.
If any login is unknown, establish a trusted rebuild and rotate exposed
credentials through an explicit response plan; do not install onto or simply
"harden" an untrusted image. No attacker attribution is established.

Historical rebuilt-host facts: Ubuntu 22.04.5; no VPN runtime/old user
credentials; password root SSH enabled; authorized_keys empty; ~34 GB free.
**All current values are UNVERIFIED.** The old installer evidence from Ubuntu
26.04 and the destroyed host's credentials/backups cannot qualify this host.

Before password authentication is disabled, install the owner's public key,
verify it through an independent session and `ssh-harden.sh verify`, and keep
a working provider recovery console. The tool's log check supplements, not
replaces, the owner's independent-session verification. No SSH change is
authorized by this report.

GitHub ruleset 22482226 actively prevents deletion/force pushes and requires
a PR; bypass actors are empty. It has no required-status-check rule.
Classic protection read returned 403, so effective legacy requirements remain
unobservable. Release ruleset 22482307 blocks v* tag update/deletion/force
updates, with no bypass actors. Owner must verify/require CI aggregate and
both Security checks. No setting was changed.

Current main's cargo-audit job succeeds after RUSTSEC-2026-0381/0382
corrections; this is a dated exact-SHA result, not a perpetual clean bill of
health. CodeQL succeeds. #148 retains pinned actions and release signatures.
The REALITY decoy buffering defect remains an upstream availability risk.
No new exception to a high/critical advisory was approved.

Operations gates still requiring exact-candidate staging:

- Both authenticated transports; UDP/QUIC/TCP/DNS/IPv4/IPv6; tunnel-to-host
  and own-address isolation; ordinary host IPv6 neighbor discovery after
  install/update/repair/reboot; public/provider firewall reachability.
- Subscription auth, user removal/revocation/expiry, token and transport
  rotation, negotiation/import/refresh, URL/TLS/HSTS correctness and generic
  errors. Never retain real URLs/tokens in evidence or installer transcripts.
- systemd service users/permissions, restart limits/watchdog, interrupted
  update recovery and rollback, zero unexplained doctor FAIL, certificate
  issuance plus a real attempted renewal and restored firewall/nginx state.
- Independent encrypted off-host state backup: dedicated owner-held recipient,
  pinned storage host key, restricted uploader, verified upload/download hash,
  independent retention/snapshots and a disposable restore drill. Local tool
  fixtures are not production backup proof. The upload key may delete remote
  files; independent snapshots/retention must resist compromised-node deletion.
- Shorts monitor install/check and a delivered alert plus recovery. No chosen
  external alert destination or real restored-host delivery is verified.
- Disk/inode headroom before build, bounded caches and cleanup, sustained
  resource/load measurements, connection churn and outage recovery.

Standalone scope A is a locally managed data plane/subscription server.
Arcana scope B additionally needs a deployed API, DB/migrations, working
accounts/entitlements, provisioning, subscriptions, billing and Telegram.
The paired PRs show code progress, including SQL CI replay; they do not prove
these services are deployed or functioning. No billing/Telegram/API mutation,
request using user credentials, or provisioning-agent activation was performed.

Keep raw security/capture/install evidence out of version control. Proposed
changes must be scanned before commit/upload; synthetic fixture digests and
documentation examples must be distinguished from real secrets. The old
snapshot retains historical issue states rather than silently closing them.
