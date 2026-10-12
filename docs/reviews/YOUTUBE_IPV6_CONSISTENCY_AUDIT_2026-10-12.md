# YouTube / IPv6 consistency audit — 2026-10-12

Base: clean current main `89fd96ee97359317d9dc625d637e43bef88b404c` (verified
against origin before edits); latest release v1.2.0. Public open PRs #147,
#148 and #121 are dependency updates. GitHub API calls were denied; release
and PR inventory were verified via Git/GitHub public pages. No live node was
accessed. This table dispositions the exact D-1..D-25 catalogue in the merged
diagnostic; historical experiments are preserved rather than recast as new
observations. Code/fixtures and local lab results are not native/VPS acceptance.

| ID | Affected current location | Verified contradiction / historical state | Actual action | Test/evidence | Status | Resolution commit |
|---|---|---|---|---|---|---|
| D-1 | `CLAUDE.md:20` | Global IPv6 prohibition was overbroad. | Scoped IPv4-refused vs family-refused guidance. | Invariant guard + local family lab | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-2 | `docs/YOUTUBE_FIX_INVARIANTS.md:83` | IPv6-useless claim applied only to old refused-IPv4 nodes. | Rewrite INV-2 with explicit defaults and evidence limits. | youtube_invariants_doc + lab | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-3 | `docs/YOUTUBE_FIX_INVARIANTS.md:26` | Generalized old IP measurements. | Add dated standalone per-family report; INV-13. | Per-family source/lab record; native pending | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-4 | `crates/compat-config/src/server.rs:680` | Plain exits lacked a literal policy; hairpin rationale implied retry. | Shared typed opt-in policy; preserve default hairpin bytes. | 79 policy tests; 88-assertion real-core lab; exact doctor check | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-5 | `docs/YOUTUBE_FIX_INVARIANTS.md:12` | Live inventory cannot be inferred from October notes. | Mark dated state/current inventory unverified; retain owner target/decisions. | Historical ledger compared; no production access | HISTORICAL; current inventory UNVERIFIED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-6 | `docs/YOUTUBE_FIX_INVARIANTS.md:199` | v1.1.0 hand patch/stopped timer described as current. | Label historical; distinguish later rc.5 and reported reinstall. | Dated records only | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-7 | `docs/YOUTUBE_FIX_INVARIANTS.md:21` | Hysteria2 report and exact-release acceptance conflated. | Record later both-protocol user report on older runtime; exact candidate PENDING. | Device ledger, published acceptance unchanged | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-8 | `docs/CLIENT_PROTOCOL_BEHAVIOR.md:89` | Client-to-VPS family mistaken for destination egress. | Separate DNS/literals/listeners/ISP leaks; link policy. | Lab domain/literal/IPv6 controls | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-9 | `docs/CLIENT_PROTOCOL_BEHAVIOR.md:195` | All datacenter addresses declared refused. | Correct some addresses/families; raw-core direct leaks client address. | Historical per-IP record; unchanged domain set | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-10 | `crates/compat-config/src/deployment.rs:346` | Peering rationale stale. | Use accepted-IP explanation and invariant pointer. | Source review + renderer regression | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-11 | `apps/admin/src/main.rs:617` | Target plain user on exit differs from relay-only flag. | Clarify flag role and dedicated plain exit target, reference invariants. | CLI help/source review; relay tests | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-12 | `crates/compat-config/src/render.rs:474` | Mechanism comments cited superseded theory. | Current mechanism references point to invariants; retain importer-source history. | Doc guard and unchanged contract tests | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-13 | `docs/YOUTUBE_FINAL_ROOT_CAUSE.md:13` | Old FIXED/peering header contradicted correction. | Label historical/disproved; add per-family correction. | History retained; no invented observation | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-14 | `docs/DEVICE_ACCEPTANCE_TESTS.md:131` | Historical latency explanation looked authoritative. | Explicit disproven label and accepted-IP correction. | Compared with §18; no new device PASS | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-15 | `docs/DEVICE_ACCEPTANCE_TESTS.md:58` | All iOS protocol cells omitted dated verified release evidence. | Link September 27 protocol/refresh/network-switch result; new candidate pending. | Existing exact v1.1.0-rc.6 ledger | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-16 | `docs/DEVICE_ACCEPTANCE_TESTS.md:70` | No YouTube device PASS ignored owner report. | Separate historical report from exact-candidate device gate. | Native checklist PENDING | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-17 | `docs/DEVICE_ACCEPTANCE_TESTS.md:30` | August audit called current. | Mark snapshot historical; dated later ledger remains evidence. | Historical entries preserved | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-18 | `deploy/lib/os.sh:11` | Implementation coverage confused with product tier/v1.0 copy. | AlmaLinux 9 supported; Ubuntu best-effort, fixture coverage distinct. | OS/support-matrix fixtures | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-19 | `deploy/lib/nginx-default.sh:6` | Stock nginx welcome site created unexpected TCP/80 listener. | Disable only checksum-matching package link; ownership-aware rollback/uninstall. | New ownership fixture + public-surface parity + shell regressions | RESOLVED locally; disposable live renewal PENDING | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-20 | `docs/INSTALLATION.md:315` | Two-certificate table contradicted default shared lineage. | Single name/lineage with restricted Hysteria copy; optional separate subscription name. | Installer source review; published records retained | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-21 | `docs/INSTALLATION.md:609` | Missing router NXDOMAIN-cache troubleshooting. | Authoritative/public DNS comparison, mobile/Wi-Fi, negative TTL, preserve TLS. | Historical reported diagnosis; no live DNS changes | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-22 | `apps/admin/src/main.rs:7481` | Mutations probed first customer rather than confined probe; remove wording honest but inconclusive. | Prefer active reserved probe; retain honest inconclusive/fail states; keep remove verb. | New unit + existing CLI/mutation/doctor regressions | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-23 | `docs/EXIT_IPV6_POLICY.md:132` | Subscription-fetch-independent incorrectly implied DNS independence. | Correct help/docs; no raw-IP HTTPS with hostname-only cert. | TLS-valid lab; no new client contract/URI guarantee | RESOLVED prose; general Hiddify address override DEFERRED to client evidence | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-24 | `deploy/lib/egress_families.py:22` | Ad-hoc sampling misclassified mapped inbound peers. | Read-only ownership/direction classifier, bounded aggregate observations only. | Python fixture mapped/inbound/unknown controls | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |
| D-25 | `README.md:200` | PR #149 operator text needed alignment with unreleased opt-in policy. | Distinguish refused IPv4, per-family mode and pending native gate; keep published version. | Docs/metadata consistency fixtures | RESOLVED | `2477bef94c1ab79f91b35df6cb4c5417983c7179` |

The D-23 follow-up is a client-supported endpoint-address override retaining
original TLS SNI/name and certificate verification, accepted on exact Hiddify
versions. It is not safely promised by this server alone. Restoring DNS or
changing networks remains a TLS-valid recovery path; no owner code patch is
needed. Live inventory remains unverified; no inventory decision is required
to review the backwards-compatible code.

Required acceptance is in [EXIT_IPV6_POLICY.md](../EXIT_IPV6_POLICY.md) and
[NATIVE_IPHONE_IPV6_ACCEPTANCE.md](../NATIVE_IPHONE_IPV6_ACCEPTANCE.md). Device,
exact RC CI, fresh disposable lifecycle, v1.2.0 -> candidate update/rollback,
TLS renewal and production family mapping remain PENDING. Exposed historical
root/Cloudflare credentials require owner rotation; none were copied/read
or used here. CODE_READY_FOR_REVIEW/RELEASE_READY/YOUTUBE_SHORTS_FIXED_ON_IPHONE
are not asserted while their prescribed gates lack evidence.

## Commands and observed outcomes

Cloud environment: Debian 13 x86_64, UID 1000, Rust 1.94.1, checksum-verified
sing-box 1.14.1, IPv4/IPv6 loopback available; no sudo/CAP_NET_ADMIN. Initialized
with `. /workspace/.cloud-setup/activate.sh`. Test environment used
`SING_BOX_BIN=/workspace/.cloud-setup/bin/sing-box-local`,
`SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`,
`YOUTUBE_TEST_PYTHON=/workspace/.cloud-setup/probe-venv/bin/python`, and the
writable `SINGBOX_VPN_LOCK_PATH=/workspace/.cloud-setup/test-state.lock`.
The localhost helper uses the verified binary with a supported hosts DNS
configuration; the new lab uses the raw binary with its own controlled DNS.

| Command / target | Observed result | Limit |
|---|---|---|
| `cargo test --locked -p compat-config --test relay_role_policy plain_exit_ipv6_policy_is_explicit_and_scoped` before implementation | FAILED: renderer had 4 rules, expected 6 | Intended regression; new tests compiled against baseline |
| `cargo test --locked --workspace --all-targets` | Admin bins 108 each, CLI 93, relay CLI 15; compat unit 184 and all preceding integrations including policy 79 and all nine established REALITY/Hysteria2 tests passed; two_hop_system 1 passed, 21 failed | Two-hop network cases cannot launch sudo/CAP_NET_ADMIN; full command FAILED, not green CI |
| Final `cargo test --locked -p compat-config --test egress_family_interop -- --nocapture` | PASS, 88 real-core assertions across both transports; IPv6 exercised | Local post-policy redirects only; no actual Google or native iOS evidence |
| `cargo test --locked -p admin --test cli exit_egress_cli` | PASS, 2 | Includes idempotent setter/reconcile, check/restart rollback, archived policy and retained host policy on restore |
| `cargo test --locked -p admin --bin vpn-admin egress_probe_selection_tests` | PASS, 1 | Confined probe preferred; inactive probe falls back honestly |
| `cargo test --locked -p common -p provisioning-contract -p provisioning-agent -p subscription --all-targets -- --skip health_probe::tests::probes_a_real_singbox_direct_outbound_successfully` | PASS: common 8, agent 124, contract 27 + 10 + 4 + 15, subscription 75 + 4 + startup 8 | One existing external gstatic direct-egress test filtered after its earlier run failed: external DNS unavailable and HTTPS proxy 403 |
| `cargo test --locked -p compat-config --test youtube_invariants_doc` | PASS, 6 | INV-13/doc changes guarded with code in same commit |
| `cargo fmt --all -- --check`; `cargo clippy --locked --workspace --all-targets -- -D warnings`; `RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --no-deps` | PASS | Docs build is not a device test |
| `shellcheck -S warning install.sh uninstall.sh bin/singbox-vpn-uninstall deploy/lib/*.sh deploy/almalinux/*.sh deploy/lib/tests/*.sh` | PASS | Final new gate scripts checked as well |
| All `deploy/lib/tests/*.sh` existing fixtures + new nginx/gate fixtures | Most completed; public-surface parity, install/update/repair/rollback, security/ownership, OS, release/version/SBOM fixtures passed; new nginx/gate fixtures passed | persist-source-tree failed on root ownership; uninstall-egress-table failed on read-only /run. Root-only uninstall/container fixtures explicitly skipped. IDN fallback and pcap-tool checks exercised partial paths; unavailable idn/tshark/tcpdump branches unrun |
| `python3 deploy/lib/tests/test-egress-tools.py` | PASS, 4 | Direction/mapped/unknown rows; invalid literal modes and DNS-family controls; credential/nonloopback frontend rejection |
| Exact-candidate RC CI, fresh disposable install, v1.2.0 -> candidate update/rollback, TLS renewal, native iPhone checklist | PENDING / UNRUN | No authorized capable disposable infrastructure/device; no production changes or release dispatch |

Fixture timings on the final local run: reject_google server-first port 443
107–108 ms; non-443 1.5–4.5 ms; successful non-Google TLS 5–7 ms. The IPv6
reject closes TCP without an automatic IPv4 connection; explicit retry works.
UDP rejection produced response absence, not proven iPhone-visible ICMP.
Fragmented/ECH destination sniffing and real Hiddify recovery remain unverified.
