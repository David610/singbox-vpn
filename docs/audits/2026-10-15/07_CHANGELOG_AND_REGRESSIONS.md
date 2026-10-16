# 07 - Changelog and regressions

Nothing is merged. All commits are on dedicated branches. Each fix was written test-first
where practical and the failing result was observed before the fix was applied.

## singbox-vpn - branch `fix/shadowrocket-base64-subscription-link` (base `main` 56193f9)

| Commit | Purpose | Files | Test and result |
|---|---|---|---|
| a54a2e9 | `vpn user create/rotate-token` print and `--json` carry `subscription_url_shadowrocket` (`?format=uri&encoding=base64`), same token/credentials | `apps/admin/src/main.rs`, `apps/admin/tests/cli.rs` | `user_create_offers_a_base64_link_for_shadowrocket_without_a_second_credential`: red (missing key), green; `cli.rs` 58/58. Live: body decodes to exactly the plain feed |
| c1c0a7f | `reload_or_restart` runs `systemctl reset-failed` first so administrative restarts are not refused by `StartLimitBurst` | `apps/admin/src/service.rs` | `a_burst_of_administrative_reloads_is_never_refused_by_the_start_limit`: red at "reload 9" (the real-world number), green; Linux service tests 18/18. Live: 12 restarts in <5 min, 0 failures |
| 63ad42c | doctor's `exit_egress_policy_matches` accepts the own-public-address reject rule; one shared helper builds it for renderer and doctor | `crates/compat-config/src/server.rs`, `crates/compat-config/tests/relay_role_policy.rs` | `doctor_accepts_the_own_public_address_rule_it_tells_operators_to_enable`: red, green (also fails if the rule is missing, weakened or moved); `relay_role_policy` 80/80, lib 184/184 on Linux |
| 40d2f4d | `docs/INSTALLATION.md`: per-app link table with honest evidence column; Hiddify iOS missing-file explanation; which operations restart sing-box | `docs/INSTALLATION.md` | `test-docs-support-matrix-consistency.sh` passes |
| (this audit) | `docs/audits/2026-10-15/` | docs only | - |

Verification outside the unit tests: GitHub CI on c1c0a7f is green (full Linux run);
the run on 40d2f4d was still in progress when this was written.

## singbox-vpn - PR #156 `audit/2026-10-15-primary-client-contract`

| Commit | Purpose | Verification |
|---|---|---|
| e9e1d05 | replace the remaining archived-Tamara references (user-visible CLI text, provisioning-contract crate docs, `SUPPORTED_PRODUCT.md`, `REACHABLE_FIRST_HOP_ARCHITECTURE.md`); state precisely that Tamara Next reads one value (a relay's first-hop `detour`) from the embedded `singbox_config` | claim checked against `tamara-next/lib/infrastructure/provisioning/singbox_vpn_contract_adapter.dart`; text-only change except one `println!` string |

Review of the PR's existing three commits: every changed line is accurate; its central claim
(Tamara Next builds its own Core config) was verified in `SingBoxConfigBuilder`. It left the
stale references above and over-stated "does not use the embedded config".

## vpn-web - PR #83 `audit/2026-10-15-client-import-compat` (existing head 8742fcc)

| Commit | Purpose | Verification |
|---|---|---|
| cb3b608 | Hysteria2 link uses the node's certificate name and its Salamander password; Hysteria2 omitted when unknown; endpoint tests; goldens change only on the Hysteria2 line of hiddify/shadowrocket/links | renderer tests red then green; `functions/sub/__tests__/token.test.js` (9 tests) mutation-checked: deleting the wiring fails the main test |
| 7416429 | `tryClientSubscription` (never throws) used in render paths; clipboard failure message restored | `subscription-client-link.test.ts` red then green; `tsc --noEmit` clean |
| 4970a82 | endpoint test: 43-char base64url REALITY key with `-` and `_` unchanged in every format | passes |

Full vpn-web suite on the PR branch before the last two commits: 125 files, 1329 tests pass;
eslint 0 errors (10 pre-existing warnings). CI on 4970a82 is green.

Review of the PR's existing content: client-specific URL selection, IPv6 bracketing and the
opt-in Base64 are correct and token-preserving. Defects found in it: the throwing helper in
four render paths and the merged clipboard error (fixed above). Not addressed: the `xray`
format and INCY's VLESS-only declaration (see `issues.json`).

## Regressions

None observed from these changes. Known pre-existing test failures on a Windows checkout,
unrelated to them: `udp_probe_tests::cert_expiry_days_...` (host `openssl.exe` points at a
missing PostgreSQL config), three `cli.rs` tests that run CRLF shell scripts under bash,
and the privileged `two_hop_system`/`egress_family_interop` tests (need `CAP_NET_ADMIN`
and a sing-box binary; CI runs them).

## Behaviour changes to know about

- A vpn-web subscription for a node with no known obfuscation password no longer contains a
  Hysteria2 line (it used to contain a broken one). Operators will see VLESS-only output until
  the node's agent has reported its obfuscation password.
- Administrative restarts no longer count toward `StartLimitBurst`; automatic crash restarts
  still do.
- The doctor now FAILs if the own-address rule is missing when `public_ipv4`/`public_ipv6`
  are set (previously the reverse).

# Round 2: server hardening (branch `fix/server-hardening-rc2`, for v1.3.1-rc.2)

Commits on top of `main` 0b038a0: 9ca903e (I-15), 0c7118a (I-18), a4e8f16 and 10ad1b0 (I-16),
f070601 (I-17). No `Cargo.toml` version change: this repository keeps the SemVer core (`1.3.1`) in the
workspace and the prerelease only in the tag.

## What changed

- **Installer records the node's own public addresses (I-15).** `resolve_own_public_addresses`
  takes `PUBLIC_IPV4`/`PUBLIC_IPV6`, then interface addresses, then (NAT or floating IPs) an address
  two independent services agree on; DNS only breaks ties between addresses already proven local.
  The keys are written as top-level keys: the old sslip.io path appended `public_ipv4` after the last
  `[subscription]` table, where TOML reads it as `subscription.public_ipv4` and serde ignores it. A
  fresh install that cannot establish an address fails with an actionable message
  (`SINGBOX_VPN_ALLOW_UNKNOWN_OWN_ADDRESS=1` opts out loudly); a repair backfills missing keys only.
- **`doctor --protocol` handshakes Hysteria2 as well as REALITY (I-18).** The throwaway client uses
  Salamander obfuscation and the real hostname, and verifies the certificate against the system trust
  store like a real client. The overall L5-6 result passes only if both transports do.
- **The requested source is the one that is built (I-16).** The persistent tree
  (`/opt/singbox-vpn`) is reused only when its content digest equals the requested source. Otherwise
  the request is staged, verified and swapped in with two renames; `recover_interrupted_source_swap`
  undoes a swap killed between them and discards stale leftovers; a failed switch restores the previous
  tree. The bootstrap accepts a full 40-character commit SHA as `--ref`, refuses an archive whose
  embedded commit differs, validates `--ref` characters, and passes `SINGBOX_VPN_SOURCE_COMMIT`/`_REF`
  to the installer. The install-state manifest records `source_ref`, `source_commit` and
  `source_tree_sha256`, and doctor's release line shows them (`main @ f07060139e86 (source tree ...)`).
- **Source builds no longer warn that they are inconsistent (I-17).** The manifest also records
  `vpn_sha256`; doctor compares `vpn` against it and falls back to the admin hash for manifests that
  predate it and for release installs (which copy one file to both names). The running binary may match
  either recorded hash.

## Regressions found and fixed during the round

- 10ad1b0: the first version of the I-16 bootstrap check refused every real pinned-SHA install (see D7
  in `03`). It failed closed, so nothing was installed.
- The I-16 fixture for `test-persist-source-tree.sh` lacked `deploy/almalinux/install.sh`, which the new
  completeness check requires, and its failed-switch mock also failed the rollback move.
- A reworded log line ("UNVERIFIED singbox-vpn branch source") is pinned by
  `test-release-reproducibility.sh`; the original wording was restored.

## Behaviour changes to know about

- `install.sh --ref <40-hex>` now means "this exact commit"; any other `--ref` is still a branch name.
- An install whose archive does not carry the requested commit is refused before anything is changed.
- Manifests gain `source_ref`, `source_commit`, `source_tree_sha256` and `vpn_sha256`. Older manifests
  keep working; the doctor treats a missing `vpn_sha256` as "same as `vpn-admin`".
- A fresh install on a host whose own public address cannot be established now fails instead of
  silently leaving the own-address rule off.
- After an install that aborts mid-repair, the deployment stays "not yet accepted", and the next run
  mints a fresh subscription token for the first user (existing installer behaviour, seen during the
  crash-safety test).

## Known limits of this round

- A ref whose own installer predates a4e8f16 still reuses the persistent tree: the check lives in the
  installer of the ref being installed. Reproduced on the real VPS by requesting `main` over a candidate
  tree.
- The production updater (`update.sh --version`) refuses a dev-channel install by design, so an
  injected-failure update with rollback against a published release (`update.sh` with
  `SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch`) could not be run in this round. It was run
  afterwards against the published v1.3.1-rc.2 (see "Round 3" in `08_RELEASE_ACCEPTANCE.md`). The
  dev-rebuild path does not call that hook.

## Round 3 summary (published v1.3.1-rc.2, tag on d64ea56)

- No code change after the merge. Release artifacts, checksums, attestations and the source archive were verified
  independently; the installed binaries equal the published ones.
- Findings: **I-31** (a plain `update.sh` does not record the node's own addresses; upgraded hosts stay exposed
  until `install.sh` is re-run), **I-32** (rc.1's updater rollback fails when `sing-box` restarts were close
  together; rc.2 does not), I-33 (cosign downloaded twice with a 300 s cap), I-34 (subscription keeps advertising
  the old Hysteria2 port), I-35 (intermittent single-probe misses in my client), I-36 (Let's Encrypt 5/168 h).
- The first injected-failure rollback attempt on rc.1 -> rc.2 **failed** and is kept in the evidence next to the
  clean re-run.
- Test environment: on a Windows checkout with `core.autocrlf=true` shell tests fail on CRLF
  (I-27). Under WSL with line endings normalised, all 71 `deploy/lib/tests` pass (3 of them need root).
