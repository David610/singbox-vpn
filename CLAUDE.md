# Instructions for Claude sessions working in this repository

## YouTube / Shorts fix — protected, read before touching related code

Shorts can fail because **YouTube refuses to serve Shorts media to some
egress IPs, independently per address family** (historical measurements,
2026-10-11/12). The fix routes the
Google/YouTube domain set out through the other node's IP
(`google_egress_hairpin`). It is verified on a real iPhone and must not be
broken.

Before changing anything that routes, renders, deploys or "improves"
Google/YouTube traffic (`google_egress_hairpin`, `GOOGLE_EGRESS_DOMAINS`, exit
`route.rules`, sniffing, QUIC/UDP, IPv6, DNS, transports, compat modes, node
deploys), **read [`docs/YOUTUBE_FIX_INVARIANTS.md`](docs/YOUTUBE_FIX_INVARIANTS.md)
in full** and keep every invariant in it true.

Hard rules (details and reasons are in that file):

- Transport/profile tweaks do not rescue an IPv4 address YouTube refuses.
  An IPv4-good/IPv6-refused exit is a separate case: use the explicit exit
  policy, preserving the plain-exit allow default. Proxy rejection is not
  proof of native iPhone IPv4 recovery. Hiddify discards imported route rules.
- Do not use Cloudflare WARP or any other third-party egress, and do not add
  VPSs or services, without the owner's explicit approval (owner decision,
  2026-10-11).
- Keep the literal-IPv6 reject with `no_drop`, sniff first, a single hairpin
  route for the whole domain set, and no direct fallback for it.
- Never print, log or commit the hairpin credential; pass it with `--uuid-stdin`.
- Do not hand-edit a rendered node config and leave it; the reconcile timer
  reverts it and restarts sing-box. Do not re-enable the provisioning agent on
  `ru1` while its control plane is unreachable.
- "Fixed" means the native iPhone YouTube app plays Shorts and normal videos
  through Hiddify. A browser run, `curl`, `playabilityStatus: OK` or green
  tests are not acceptance.

The tests named in the invariants file enforce these rules, and
`crates/compat-config/tests/youtube_invariants_doc.rs` fails if that file, its
enforcing tests or this pointer are removed. If you must change an invariant,
change the test and the doc in the same commit and explain why.

## Working conventions

- Rust toolchain is pinned in `rust-toolchain.toml`. Before declaring work
  done: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test`.
- Evidence classes used in the docs: code verified, server verified, browser
  verified, native device verified, user reported, unverified hypothesis. Do
  not promote a claim above the evidence you actually have.

Current live node inventory is unverified. The named de1/ru1 deployment and
agent-disable decision are dated historical records; do not infer current
production health or re-enable a deliberately disabled agent. See
[exit IPv6 policy](docs/EXIT_IPV6_POLICY.md) for the unreleased opt-in mode
and exact-candidate device/VPS gates.
