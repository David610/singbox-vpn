# Documentation audit — 2026-10-23

Scope: every tracked Markdown file in `David610/singbox-vpn` (158) and `David610/vpn-web` (45), plus the operational claims
in them that are checkable offline. Status words follow the mission: PASS, FAIL, BLOCKED, UNVERIFIED, NOT APPLICABLE.

## What was checked and how

| Check | Method | Result |
|---|---|---|
| Relative links and `#anchors` resolve (singbox-vpn) | `deploy/lib/docs-links-check.py` (new, offline, deterministic) | **1 broken link found and fixed** (`docs/INSTALLATION.md` → removed `DEPLOYMENT.md`); now 0 in current docs, 0 in historical records. PASS |
| Same, vpn-web `main` | same tool, `--root` | 0 findings. PASS |
| Repo paths named in code spans / command blocks | same tool, `--code-paths` (advisory) | 23 hits in current docs, triaged below; 3 fixed, rest are deliberate references to removed code, other repositories or upstream docs. UNVERIFIED as "all correct" — the check cannot read intent |
| External URLs | `deploy/lib/docs-external-urls.py` (new, advisory, follows redirects, retries, 401/403/429 = restricted) | triaged below |
| Release status claim ("current stable v1.3.0") | `gh release list` | matches: v1.3.0 is *Latest*, v1.3.1-rc.2 is the newest pre-release, rc.3 is not tagged. PASS |
| Version/tag contract | `check-release-version.sh v1.3.1-rc.3`, `check-workspace-version-consistency.sh` | PASS (6 packages at 1.3.1) |
| Old server addresses presented as current | grep of IPv4 literals in README, INSTALLATION, clients, SUPPORTED_PRODUCT, RELEASE, RECOVERY, DEVICE_ACCEPTANCE_TESTS | only in `DEVICE_ACCEPTANCE_TESTS.md` result tables that are dated 2026-09/10-11 and labelled as past runs. Not changed (historical evidence). PASS |
| Hysteria2 link contract (I-01 / I-02) | vpn-web tests + this repo's renderer | vpn-web emits `sni=<node hostname>`, `obfs=salamander`, `obfs-password=…` (same parameter names as `crates/compat-config/src/render.rs`); fix `cb3b608` is on vpn-web `main`; regression test in `subscription-renderers.test.js` passes (vpn-web suite 1424/1424). **Code verified**; no client import was exercised |

## Fixed in this audit

1. `docs/INSTALLATION.md` linked `DEPLOYMENT.md` (deleted with the native adaptive stack). Bullet removed.
2. `docs/operations/HAIRPIN_TARGET_QUALIFICATION.md` pointed at `HAIRPIN_SIMULATION.md` "(if present)"; it never existed. Now points at `deploy/lib/hairpin_simulation.py` and says it is a simulation.
3. `docs/EXIT_IPV6_POLICY.md` named `docs/configuration/route/rule_action.md` as if it were in this repository; it is upstream sing-box documentation. Reworded.
4. `docs/contracts/ARCANA_PRODUCT_V1.md` said `ARCANA_LINKS_V1.md` lives in "this repo"; it lives in `vpn-web`. Reworded.

## Code-path hits left on purpose (not errors)

| Where | Reference | Why it stays |
|---|---|---|
| `deploy/almalinux/README.md`, `docs/SUPPORTED_PRODUCT.md`, `docs/PERFORMANCE_OPTIMIZATION_PLAN.md`, `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` | `apps/client-daemon`, `apps/cli`, `apps/keytool`, `crates/network-state`, `crates/failure-classifier`, `services/*` | each sentence says these were **removed** (preserved on `archive/native-adaptive-stack-2026`) |
| `docs/B2_EPHEMERAL_AUTH_EVIDENCE.md`, `docs/PROVISIONING_CONTRACT.md` | `docs/ADR/0003-ephemeral-managed-authorization.md` | explicitly "vpn-web `docs/ADR/…`"; exists there |
| `docs/contracts/ARCANA_PRODUCT_V1.md` | `managed-control-plane-v1.md`, `design/ARCANA_FINAL_APP_DESIGN_*.md` | explicitly in `tamara-next` (private repo; not verifiable from here → UNVERIFIED) |
| `docs/RUSSIA_PRODUCTION_INVESTIGATION.md` | `fuzz/Cargo.lock` | a historical session note about an untracked file |

## External URLs (advisory run, 2026-10-23, network from the audit host)

OK or redirected-OK for the large majority. Items needing a human decision:

| URL (source) | Class | Action |
|---|---|---|
| `https://ooni.org/country/RU` (RUSSIA_PRODUCTION_INVESTIGATION.md) | DEAD (404) | historical investigation; leave, or point at ooni.org explorer |
| `https://bearvpn.com/blog/is-tiktok-banned-in-russia/` (TIKTOK_INVESTIGATION.md) | DEAD (410) | third-party article gone; historical citation, leave |
| `https://github.com/David610/singbox-client`, `…/tamara`, `…/tamara-next` | 404 unauthenticated | these are **private** repositories (confirmed in the account's repo list); links in README / fixtures work only when signed in. README linking a private repo is a usability defect for outside readers: UNVERIFIED whether intended |
| `…/.github/workflows/release.yml@refs/tags/…`, `https://token.actions.githubusercontent.com`, `https://gstatic.com/` | 404 | not links: certificate-identity strings, OIDC issuer and a domain name used as data. The tool cannot tell; they are correct as written |
| `https://1.1.1.1`, `https://8.8.8.8`, `https://slsa.dev/provenance/v1` | redirect | resolve; `slsa.dev/spec/v1.1/provenance` is the canonical form (cosmetic) |
| `api.ipify.org`, `api4/api6.ipify.org`, `api.my-ip.io` | 520/disconnect/429 | transient or rate-limited; **UNVERIFIED**, not dead. Used only as IP-echo examples |
| `http://[::1`, `https://[2606:…` fragments | unparseable | placeholders in a runbook |

## Automation added

- `docs-links-check.py` runs as a step of the existing `docs` CI job (no new required-check name; the verdict job already depends on `docs`).
  Unit tests (`test_docs_links_check.py`, 8 tests) cover missing target, missing anchor, duplicate-heading anchors, fenced blocks, external links not flagged,
  historical records marked, code-path mode, and a self-test that this repository is clean.
- `docs-external-urls.py` is **not** a CI gate (network is non-deterministic). Run it manually or on a schedule.

## Not verified (honest limits)

- Commands in prose and fenced blocks were checked only for *file paths that must exist*, not executed. A wrong flag in a documented command is not detected by this audit.
- Whether `docs/clients/*` setup steps work in each client was not tested; that needs the clients (see the device runbook).
- Screenshots/diagrams: none are tracked as images in the audited docs; ASCII diagrams were not compared to code.
- Test-count statements in historical reports (e.g. 1,105 Rust / 183 Python) are dated and were left as history. Only README, `docs/*.md`, `docs/operations` and `docs/clients` were grepped for stale counts (none found); deeper files were not searched.
