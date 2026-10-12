# v1.3.0 release preparation — procedure and instructions

Companion to [RELEASE_ISSUE_LEDGER_2026-10-12.md](RELEASE_ISSUE_LEDGER_2026-10-12.md).
This is a preparation record, not an acceptance record: it authorises **no**
tag, release dispatch, deployment, DNS/firewall/sysctl change or credential
operation. Every step below that touches infrastructure or publishes something
needs the owner's explicit go-ahead. Placeholders are written `<LIKE-THIS>`;
the real SHAs, run URLs and versions are recorded only in
`docs/DEVICE_ACCEPTANCE_TESTS.md` and `docs/release-acceptance/v1.3.0.md`
after the fact (those are the paths RELEASE.md lets change after RC acceptance).

## 1. Proposed release identity

**Proposal: SemVer core `1.3.0`** (RC tags `v1.3.0-rc.N`, stable `v1.3.0`).
A patch release is not appropriate: the public scope adds an operator-facing
configuration table (`[egress]`), CLI commands (`egress show|set|clear`), a new
`doctor` identity check, `user links` selection by name, new manifest fields
and acceptance-workflow inputs. None of it breaks a v1.2.0 deployment
(defaults unchanged, schema version unchanged), so no major bump.

Version-coupled files for the bump (done in this PR; `check-release-version.sh`
and the offline gate enforce them): `[workspace.package].version`,
`Cargo.lock`, `fixtures/arcana-node/node-capabilities-v1.json` `node_release`.
The RC suffix lives only in the tag. Rust 1.94.1 and sing-box 1.14.1 pins are
unchanged; no dependency PR (#121, #147, #148) is included.

## 2. Freeze and RC creation (owner)

1. Merge this PR through the normal reviewed path (required CI + Security).
2. From the merge commit, verify locally:
   `deploy/lib/check-release-version.sh v1.3.0-rc.1` and
   `bash deploy/lib/tests/test-offline-full-ci-equivalence.sh`.
3. Create and push the **annotated** tag `v1.3.0-rc.1` on that commit. The
   Release workflow builds in the pinned AlmaLinux 8.10 container (glibc ≤ 2.28),
   runs the packaged archive in AlmaLinux 8 and 9 containers, produces SBOM and
   attestations and publishes a **pre-release**. Never move or reuse a tag; a
   failed acceptance means a new RC (`-rc.2`) from a new commit.
4. Record the exact RC commit SHA (`git rev-parse v1.3.0-rc.1^{commit}`).

After the RC exists, implementation, deployment, workflow, dependency and
Cargo files are frozen; any change there requires a new RC and a repeat of
every gate below.

## 3. Disposable lifecycle acceptance (needs authorised hosts)

Hosts must be throwaway: **AlmaLinux 9 x86_64** (the supported target) **and**
**Ubuntu 24.04 x86_64** (the owner's OS; best-effort tier, so evidence is
required before any support wording). Never the production host.

For each, obtain the host's SSH public key out-of-band (provider console) and
build a `known_hosts` line, then dispatch
`.github/workflows/vps-acceptance.yml` with: `ssh_host`, `domain`,
`version=v1.3.0-rc.1`, `verify_egress_policy=true`, `ssh_known_hosts=<LINE>`,
`skip_reboot=false`. Three kinds of run per OS family (runs 1–2 use the workflow, run 3 the script):

| # | Inputs | Proves |
|---|---|---|
| 1 | `version=v1.3.0-rc.1` | fresh install, repair/reconcile, reboot, renewal, uninstall/reinstall, egress stage 16c |
| 2 | `version=v1.2.0`, `update_to_version=v1.3.0-rc.1` | published stable → RC update, injected-failure rollback, policy persistence |
| 3 | `deploy/almalinux/hybrid-upgrade-acceptance.sh`, once per scenario (`normal`, `hybrid`, `hybrid-policy`), each on its own baseline-prepared host — **not** the lifecycle workflow, which installs fresh and so can never be a hybrid | the three real v1.2.0 → RC transitions, credential preservation, policy persistence, downgrade refusal (see §6) |

The workflow refuses a release run without the egress stage, with the reboot
skipped, or without a verified host key, and independently requires the stage
16c PASS and `LIFECYCLE GATE: PASS` in the log. Local equivalent (owner
execution, after explicit disposable-host authorisation):

```sh
bash deploy/almalinux/lifecycle-acceptance.sh --host USER@DISPOSABLE_HOST \
  --domain DISPOSABLE_DOMAIN --i-understand-this-is-destructive \
  --version v1.3.0-rc.1 --verify-egress-policy --known-hosts-file ./known_hosts
```

An existing installation needs the separate
`--allow-destroy-existing-singbox-vpn-install`. v1.1.0 → RC keeps the special
updater path documented in `docs/INSTALLATION.md`.

## 4. Native iPhone acceptance (needs the owner's device)

Same immutable RC on a disposable or owner-authorised host. Full procedure and
evidence template: `docs/NATIVE_IPHONE_IPV6_ACCEPTANCE.md`; the matrix to fill
is in `docs/DEVICE_ACCEPTANCE_TESTS.md` ("v1.3.0 exact-candidate native
matrix"). Minimum: REALITY and Hysteria2, each on Wi-Fi and cellular, via the
normal HTTPS subscription; compare `allow` and `reject_google`; `sAcElROnYIE`
plus ≥ 5 consecutive Shorts; long video with seeking; 15 minutes sustained;
reconnect, idle, Wi-Fi↔cellular handover, client reboot, authorised server
reboot, subscription refresh; ordinary browsing and non-Google IPv6; client-ISP
IPv6 and DNS leak checks recorded separately. Also regression-test the existing
accepted-IP hairpin (no change to its target, credential or owner decisions).
Browser emulation is not a substitute.

If the device gate fails, **do not widen the claim**: capture sanitised
evidence (which transport, which family, whether the app retried), prepare the
next RC only for a justified change, and repeat §3–§4. If no single-server
mechanism satisfies native playback within the constraints, the release is
BLOCKED and the Shorts claim stays unmade (the policy's default remains
`allow`, so shipping it harms nobody who does not opt in).

## 5. Stable record (only after every gate passes)

Create `docs/release-acceptance/v1.3.0.md` with the exact lines required by
`docs/RELEASE.md`:

```text
Release: v1.3.0
Decision: ACCEPTED
Accepted RC: <v1.3.0-rc.N>
Accepted RC commit: <40-character SHA of that RC>
VPS acceptance: PASS
Device acceptance: PASS
Certificate renewal: PASS
```

plus run URLs, the OS/architecture of each lifecycle host, and the sanitised
device matrix. The stable tag must be a documentation-only descendant of the
accepted RC (allowed: `README.md`, `docs/DEVICE_ACCEPTANCE_TESTS.md`, the
record itself). Verify the `main` ruleset and the `v*` tag protection in
repository settings before tagging. Publication is **manual**: dispatch the
Release workflow from the stable tag with `tag=v1.3.0` and
`confirm_stable_release=true`. RC and stable binaries are built separately from
different commits; do not claim byte-identical artifacts unless the two
`SHA256SUMS` are compared and match.

After publication: verify the stable assets, checksums, attestations and the
bootstrap's stable selection, then run one fresh install through the public
one-command bootstrap on a disposable host.

## 6. Production upgrade (separate authorisation)

Use the supported package, never copied executables. **Which updater runs
matters**: a v1.2.0 host's installed updater (`/opt/singbox-vpn`) predates the
egress guard and the identity recording and cannot be changed. For a normal
v1.2.0 host, a host with development executables copied over it, and a host
already using `reject_google`, run the **target release's own updater from its
checksum-verified source archive** (the documented path in
`docs/INSTALLATION.md` "Updating a host that is still on v1.1.0" — it applies to
any host whose installed updater predates the target):

```sh
TAG=v1.3.0
B=https://github.com/David610/singbox-vpn/releases/download/$TAG
W=/root/singbox-vpn-updater; rm -rf "$W"; install -d -m 0700 "$W/dl" "$W/src"
cd "$W/dl" && curl -fsSL -o SHA256SUMS "$B/SHA256SUMS" && curl -fsSL -o singbox-vpn-src.tar.gz "$B/singbox-vpn-src.tar.gz"
grep ' singbox-vpn-src.tar.gz$' SHA256SUMS | sha256sum -c -        # must print OK
cd "$W/src" && tar xzf ../dl/singbox-vpn-src.tar.gz --strip-components=1
sudo bash deploy/almalinux/update.sh --version "$TAG"
sudo vpn-admin doctor --protocol --require-protocol
sudo vpn-admin doctor      # "release identity: recorded stable release v1.3.0 ..." (recorded by this updater)
sudo vpn-admin egress show # policy unchanged
```

**Do not run `update.sh --repair` from the v1.2.0 tree first** (or
`--version v1.2.0`): that re-installs the recorded v1.2.0 executables and drops
the policy and the hand-copied build. After this update `/opt/singbox-vpn` holds
the new tree, and every later update/repair is guarded.

This exact sequence is scripted for a disposable host in
`deploy/almalinux/hybrid-upgrade-acceptance.sh` (`--scenario normal|hybrid|hybrid-policy`),
which starts from an existing baseline (it never uninstalls or reinstalls first),
fingerprints credentials, compares the installed executable with the published
release independently of the manifest, runs the sequence above, repeats a repair
from the new tree and proves a downgrade is refused. Run each scenario once on
its own prepared baseline host before touching the owner's host (§3, run 3).

`update.sh` is transactional (staged verification, switch, protocol
acceptance, commit; automatic rollback on failure). Afterwards verify: the
"release identity" doctor line names the recorded stable release with matching
hashes (an installer-recorded match is not independent provenance — the updater's
source-archive checksum and attestation checks are what establish that); users and `[egress]` are preserved;
REALITY and Hysteria2 handshakes; subscription refresh; one native playback test
(Shorts **and** TikTok); and observe one full reconcile (10 min) and watchdog
cycle, then the bounded soak below.

**Soak (bounded, measurable).** On the disposable host and again on the owner's
host after cutover:

```sh
sudo bash deploy/lib/soak-sampler.sh --duration 5400 --interval 30 --output /root/soak.csv
python3 deploy/lib/soak_evaluate.py /root/soak.csv      # exit 0 = PASS
```

The criteria (≥ 90 minutes, 100 % availability, no unplanned sing-box restart,
bounded RSS/fd growth, ≥ 8 successful reconciles, watchdog active, every
protocol self-test passed) are fixed in the evaluator before the run; missing
data fails. Keep a client connected throughout so grant/credential expiry is
exercised, and record any disconnect with its time.

**The website is a separate component.** This release does not deploy the
frontend, Pages Functions or database. Before the stable record is written, name
the accepted frontend commit/build, Functions version and applied migration level
(see the ledger's component manifest) and deploy them by their own procedure;
the "agreed website journeys" gate (ledger P-5…P-7) must pass against that exact
set, and a public smoke repeated after cutover. A backend release alone is not
completion.

**Rollback.** A failed update rolls back by itself. A deliberate downgrade to
v1.2.0 is refused while `[egress]` is present (v1.2.0 would ignore it
silently); to proceed on purpose run `vpn-admin egress clear` first — which
re-exposes literal-IPv6 Shorts failures on an IPv6-refused exit — then
`update.sh --version v1.2.0`. Never move a release tag or replace assets under
an existing tag to fix a bad release; publish a new version.

## 7. Release notes (draft)

**v1.3.0 — standalone exit IPv6 policy and deployment hardening**

- New opt-in `[egress] ipv6_literal` policy (`allow` default, `reject_google`,
  `reject_all`) with `vpn-admin egress show|set|clear`, for an exit whose IPv4
  address YouTube accepts but whose IPv6 range it refuses. Rejection closes the
  flow; native-app recovery is accepted per release in the device record.
- `vpn-admin doctor`: release-identity check (stable / RC / development /
  inconsistent / UNKNOWN); loopback resolver no longer reported as a public
  listener; firewall judged by the recorded backend (ufw hosts no longer warn);
  external reachability always reported as UNKNOWN.
- `update.sh` refuses to switch an `[egress]` deployment to a build that would
  silently ignore it; installer/updater record executable hashes.
- `vpn user links` accepts a unique user name; ambiguous names are rejected.
- `vpn-admin … | head` exits quietly (status 141) instead of panicking.
- Acceptance workflow: egress stage, reboot and verified SSH host keys are
  mandatory for release runs; stage evidence is checked independently.
- Debian/Ubuntu: the stock nginx welcome site no longer holds TCP/80.
- Unchanged: accepted-IP hairpin, C-16 isolation, subscription/provisioning
  contracts, schema versions, defaults.
