# COMPATIBILITY_SECURITY_REVIEW.md

Adversarial self-review of the compatibility (VLESS+REALITY/Hysteria2)
stack, per spec §58. Findings are recorded honestly, including the ones
not fully fixed — see "Remaining gaps" at the end.

**Updated by the production-hardening pass** (see
`docs/PRODUCTION_HARDENING_PLAN.md` for the full fix list). Several
items below that were previously flagged as gaps are now fixed —
marked `[FIXED]` inline — and several new findings from that pass are
folded in. This review is still describing a single-VPS deployment that
has not been run against a real AlmaLinux host/Android client — see
"Remaining gaps".

## As a DPI/censor

- **What can identify each transport?** VLESS+REALITY looks like a
  normal TLS 1.3 handshake to whatever `handshake_server` is configured
  (a real, popular site) — REALITY's entire design point is that an
  active MITM/replay probe against the disguise target gets a real
  certificate/response, not a fake one, because sing-box actually
  relays the initial handshake to the real site. Hysteria2 is QUIC and
  looks like generic HTTP/3 traffic; sing-box's `masquerade` option
  (not yet configured by `render_singbox_server_config` — see gaps
  below) can make failed-auth connections serve real-looking HTTP/3
  content instead of an obvious reset.
- **UDP blocked entirely?** Hysteria2 fails; VLESS+REALITY (TCP/443)
  keeps working — this is the whole reason both transports were chosen
  (independent failure modes, matching the native stack's
  direct-tls/noise-quic split). Client-side, sing-box's `urltest`
  selector (configured in `render_singbox_client_subscription`) routes
  around the failed one automatically.
- **TCP 443 reset / SNI-blocked?** Hysteria2 (UDP) keeps working. This
  is failure-independence at the protocol-family level, not just
  endpoint level — see `tests/` in `crates/compat-config` for what's
  actually asserted (rendering correctness); true network-level
  failure-independence for this specific pair has **not** been proven
  with an executed test (unlike the native stack's
  `tests/tests/failure_independence.rs`) because this sandbox cannot run real
  sing-box/network-namespace tests. Documented gap, not claimed solved.
  `render.rs`'s
  `hysteria2_unavailable_reality_only_profile_remains_fully_usable` test
  closes a narrower, structural slice of this: it asserts that a
  rendered subscription still contains a fully usable REALITY outbound,
  selector, and route when a caller simply omits the Hysteria2 endpoint
  (modeling "Hysteria2 is unavailable" at the render layer). This is
  **not** a real UDP-blocked network test — no packets are sent, no
  sing-box process runs — it only proves the generated *profile* doesn't
  become broken/unusable when one transport is missing from it. The real
  network-level gap above is unchanged.
- **UDP/443 blocked or throttled?** Some networks/ISPs block or
  rate-limit UDP outright (including for reasons unrelated to
  censorship, e.g. carrier NAT/QoS policies) — Hysteria2 depends on
  UDP being usable at all, not just "not specifically blocked". This is
  why Hysteria2 stays an optional secondary transport (`docs/
  SUPPORTED_PRODUCT.md`) and is never the sole or default transport.
- **REALITY handshake target ("decoy")** — choosing a well-known site
  for `REALITY_HANDSHAKE_SERVER` is camouflage against passive/naive
  inspection, not a guarantee. It does not make the connection
  invisible, does not make it indistinguishable from real traffic to an
  active adversary doing timing/volume analysis, and does not carry any
  country-specific promise — see `docs/ALMALINUX_DEPLOYMENT.md`'s
  selection guidance (no universal safe default; validated against the
  live REALITY protocol acceptance test, not assumed safe). There is no
  hardcoded default handshake target in this codebase.

## As an active censor probing the endpoint

- Unauthenticated REALITY probes get real TLS behavior from the disguise
  site (this is sing-box's implementation, unmodified — not something
  this codebase adds risk to).
- **[FIXED]** Hysteria2 now sets `masquerade` (type `file`, serving a
  placeholder static page) when the installer-created masquerade
  directory exists, so unauthenticated/invalid connections see a
  plausible HTTP response instead of a distinctive auth-reject
  signature. This only changes what an *unauthenticated probe* observes
  — it does not hide that QUIC/UDP:443 is open and is not equivalent to
  REALITY's live-relay disguise (Hysteria2 has no such mechanism).

## As a compromised-VPS attacker

- **Which secrets are exposed?** REALITY private key
  (`/etc/vpn/compat/reality/private.key`, mode 0600, root-owned), every
  active user's VLESS UUID + Hysteria2 password (`users.json`, mode
  0640, root:vpn-subscription) — a full VPS compromise yields all
  current user credentials and the REALITY key, exactly as expected for
  a single-VPS deployment (spec never promised protection against full
  host compromise, see `docs/THREAT_MODEL.md`).
- **Can offline root signing keys remain offline?** Yes — the
  compatibility stack has no relationship to the native
  root→release→bundle signing hierarchy at all (spec §5/§28 hard
  boundary, enforced by keeping `compat-config` a fully separate crate
  with no dependency on `crypto::hierarchy` or `config`). Compromising
  this VPS cannot forge a native signed `RelayBundle`.
- **Documented, intentional exception**: `users.json` is 0640 (not
  0600) so the non-root `vpn-subscription` service user can read it —
  see the doc comment on `compat_config::store::save_users_atomic`.
  This means compromising the `vpn-subscription` service user (a much
  smaller attack surface than root — no shell, no write access, network
  sandboxed to loopback via `IPAddressAllow=127.0.0.0/8 ::1/128` in the
  systemd unit) yields read access to all user credentials. This is
  the necessary tradeoff for the subscription service to function
  without running as root; the alternative (running the whole
  subscription service as root) is strictly worse.
- **[FIXED]** `config.json` (the rendered sing-box server config —
  contains the REALITY private key, every active user's VLESS UUID, and
  Hysteria2 password in cleartext) previously inherited the process
  umask when written by `apply_config_atomically` (often 0644,
  world-readable, on a root-run installer). It is now always written
  0640 root:sing-box, including its `.bak` rollback copy, and
  `deploy/almalinux/update.sh`'s rollback path no longer hardcodes 0644
  when restoring it. See `docs/PRODUCTION_HARDENING_PLAN.md` #2/#29.
- **[FIXED]** The pre-hardening installer put `reality/` and
  `hysteria/` under groups `sing-box` never belonged to
  (`vpn-subscription`/`root`), meaning the `sing-box` service user could
  not actually read the REALITY private key or the Hysteria2 TLS
  key/cert it needs to start — every directory sing-box must read is
  now group-owned `sing-box` directly. See
  `docs/PRODUCTION_HARDENING_PLAN.md` #1.

## As a malicious subscriber

- **Can they enumerate users?** No new IDs, no incrementing counters
  are exposed anywhere in the subscription API; `GET /sub/{token}`
  returns an identical generic 404 for "token doesn't exist", "token
  belongs to a disabled user", and "token belongs to an expired user"
  (`services/subscription::get_subscription`, tested in
  `unknown_token_returns_generic_404` /
  `disabled_user_token_returns_404_not_a_distinguishable_error`).
- **Can they retrieve another user's subscription?** No — lookup is by
  presented-token hash match only; there is no user-ID-based lookup
  endpoint.
- **Can they brute-force subscription tokens?** Tokens are 160-bit
  CSPRNG values (`generate_subscription_token`), infeasible to guess.
  Per-source-IP rate limiting (20 burst, 0.5/s refill) slows a
  single-IP brute force to a crawl but a **distributed** brute force
  across many source IPs is not mitigated by this in-process limiter —
  flagged as needing a real
  edge/CDN rate limiter in front of any public deployment, not solved
  by this codebase alone. Given 160 bits of entropy, distributed
  brute force is still computationally infeasible regardless (this is
  a defense-in-depth gap, not an exploitable weakness at current
  entropy).

## As an SRE

- **What survives reboot?** `sing-box` and `vpn-subscription` are
  `enabled` systemd units (`WantedBy=multi-user.target`); REALITY
  key/short_id and `users.json` are on-disk, not regenerated at
  startup.
- **[FIXED] Does `user disable` actually stop the disabled user's
  traffic?** Previously: no. Every `vpn-admin user
  create/disable/enable/remove/rotate-*` command rewrote and validated
  `config.json` but never reloaded the running `sing-box` process — a
  disabled user's credentials remained accepted by the live server
  until something else restarted it. `regenerate_singbox_config` now
  reloads (`systemctl reload-or-restart sing-box`) and verifies the
  service is active after every config-affecting mutation; on
  reload/health failure it restores the previous config and reports
  failure rather than claiming the mutation succeeded. See
  `docs/PRODUCTION_HARDENING_PLAN.md` #4.
- **What happens after a failed config update?**
  `apply_config_atomically` never touches the live `config.json` if
  `sing-box check` fails on the staged temp file — tested
  (`apply_atomically_rejects_invalid_config_and_leaves_existing_file_untouched`).
  `update.sh` additionally rolls back binaries if the post-update health
  check fails, and now re-verifies health after the rollback itself
  before reporting success.
- **What happens after certificate expiration?** Only relevant to the
  subscription reverse proxy's cert (REALITY doesn't terminate its own
  TLS the way a normal HTTPS server does — see
  `docs/COMPATIBILITY_VERSIONS.md`). certbot's renewal timer handles
  this; documented, not automated by this codebase (nginx/certbot are
  external, well-maintained components per spec §13/§27).
- **What happens if sing-box crashes?** `Restart=on-failure` +
  `RestartSec=2` in `sing-box.service` recovers an ordinary crash within
  seconds, including a `SIGKILL`/OOM kill. An explicit, generous
  `StartLimitIntervalSec=300`/`StartLimitBurst=8` (same pattern in
  `vpn-subscription.service`) means a burst of transient startup
  failures — a slower-than-usual boot-time race, for example — survives
  instead of the unit parking itself `failed` after systemd's old
  implicit 10s/5-restart default. If a unit ever does exhaust that
  budget, `vpn-service-watchdog.timer` (a small, low-frequency,
  systemd-native timer — no custom monitoring service) periodically
  clears a stuck `failed` state and asks systemd to try again, so a
  recoverable service is never left permanently down after a transient
  failure; a deliberate `systemctl stop` is unaffected (that state is
  `inactive`, never `failed`). `vpn-health-check`/`vpn doctor`/`vpn
  status` all report `failed` explicitly (distinct from merely
  `inactive`) and a restart count when systemd exposes one, so an
  operator can see this happened even after it self-heals.

## As a censor blocking the VPS's IP/ASN or the subscription domain

This is the hard failure domain a single-VPS deployment cannot design
its way out of, and v1.0 does not try to:

- **IP/ASN blocking.** REALITY and Hysteria2 both terminate on the same
  VPS IP. Blocking that one IP (or the whole hosting ASN, a common
  escalation) takes down **both** transports at once — the two
  protocols are only independent against protocol-specific detection
  (SNI blocking, UDP blocking), not against an IP-level block. `urltest`
  client-side selection (`docs/CLIENT_PROTOCOL_BEHAVIOR.md`) cannot help
  here: it picks between endpoints that are both unreachable.
- **Subscription domain/IP blocking.** If the subscription HTTPS
  hostname (or its IP) is blocked separately from the REALITY/Hysteria2
  listeners, clients can't fetch or refresh their profile even if the
  VPN transports themselves are still reachable. `vpn-admin user links`
  (added this session) is the out-of-band answer: it prints the raw
  `vless://`/`hysteria2://` URIs directly from server-side key material
  over SSH, with no dependency on the subscription host/port at all, so
  an admin can relay a working profile through any other channel when
  the subscription domain specifically is what's down.
- **What v1.0 does NOT do about full-VPS/IP blocking.** No multi-node
  control plane, no automatic failover VPS, no fleet/rendezvous system —
  see `docs/RECOVERY.md` for the deliberately manual recovery procedure
  (fresh VPS + fresh domain + fresh credentials + manual redistribution).
  This is a real, accepted limitation of a `<=10`-user single-VPS
  deployment, not a solved problem.
- **Real-world effectiveness is unmeasured.** None of the claims above
  (or elsewhere in this document) about what a specific censor in a
  specific country actually does have been tested against a real
  network in that country — see `docs/DEVICE_ACCEPTANCE_TESTS.md`'s
  Telegram-specific matrix, which stays "not yet tested" until someone
  runs it on a real connection.

## Remaining gaps (not fixed this session, documented not hidden)

1. No automated network-level failure-independence test for this
   compatibility pair (the native stack has one; this pair does not,
   because it requires a real sing-box binary + real network
   namespaces, unavailable in this sandbox). `deploy/almalinux/
   acceptance-test.sh` documents the exact commands for a privileged
   runner but does not execute them.
2. Distributed (many-source-IP) subscription brute force is not rate
   limited beyond per-IP — acceptable given 160-bit token entropy, but
   nginx now applies its own `limit_req` on `/sub/` in front of the
   Rust service's own limiter as a second layer (see
   `docs/PRODUCTION_HARDENING_PLAN.md` #8), which helps but does not
   fully solve distributed abuse — a real edge/CDN limiter is still the
   right answer for a public deployment beyond a single VPS.
3. None of this session's fixes have been run against a real AlmaLinux
   9 host, a real VPS, or a real Hiddify Android client — see
   `docs/PRODUCTION_HARDENING_PLAN.md`'s status markers and the final
   engineering report for exactly what's implemented-but-unverified
   versus fully verified.

Fixed this session (previously listed here as gaps):

- Hysteria2 `masquerade` — configured (see "As an active censor
  probing the endpoint" above).
- `vpn-admin` credential rotation — `rotate-vless`, `rotate-hysteria`,
  `rotate-credentials` added alongside the existing `rotate-token`.
- User mutations not reaching the running server — fixed via
  reload-and-verify with rollback (see "As an SRE" above).
