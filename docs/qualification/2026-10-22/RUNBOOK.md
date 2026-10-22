# Release, acceptance and rollback runbook

Prepared only; no release, host, DNS, identity or backup configuration has been
changed. Every production mutation needs the owner's explicit approval. Stop
immediately on an unexplained successful SSH login; resolve trust first.

## Prepare the immutable release

1. Review the integration diff and its exact SHA; require full CI and Security
   on that SHA. Keep #160/#161/#121 out of the minimum standalone candidate.
   Owner reviews merge and branch-check enforcement. Do not merge autonomously.
2. After an approved merge, rerun CI/Security on the resulting commit. Confirm
   the version contract: `bash deploy/lib/check-release-version.sh v1.3.1-rc.3`.
   Confirm rc.3 is still unused. Owner separately authorizes tag/publication;
   a v*-rc tag triggers a real GitHub prerelease, not a harmless dry run.
3. Follow `docs/RELEASE.md` and the release workflow: canonical CI, pinned
   AlmaLinux 8.10 build, glibc <=2.28, packaged runtime on AlmaLinux 8/9,
   SBOM/license inventory, checksum and Sigstore/GitHub provenance.
   Record immutable tag commit and SHA-256 of each downloaded artifact.
4. Verify checksums AND provenance against David610/singbox-vpn using the
   repository's installer verification path. Do not execute an unverified
   bootstrap, substitute a dev build for the artifact, or use rc.2 as corrected.
   Any implementation/dependency/workflow change after acceptance requires
   another RC and another complete mandatory cycle.

The older `operations/RC3_RELEASE_SEQUENCE.md` describes an earlier destroyed
installation and approvals that were then pending. Its state/counts are
historical; use the sequence above with the actual refreshed SHA. Never move,
delete or reuse a published tag as rollback.

## Authorized staging installation and lifecycle

Before touching fin1: resolve the SSH observations, independently verify owner
key login and recovery console, verify provider snapshot, current DNS A/AAAA
and certificate requirements, free disk/inodes, exact release source, backup
recipient/storage/host-key trust and a real encrypted upload/restore drill.
The owner selects/authorizes storage; ordering a service is not implicit.
Record each persistent change and its tested rollback before execution.

Use a positively identified disposable host for destructive experiments.
AlmaLinux 9 x86_64 is the supported target; fin1's rebuilt Ubuntu 22.04 also
needs its own real lifecycle run. CI package installation in OS containers is
not a full systemd/firewall/ACME lifecycle qualification.

Inspect harness options before execution:

```sh
bash deploy/almalinux/lifecycle-acceptance.sh --help
bash deploy/lib/check-release-version.sh v1.3.1-rc.3
```

After host/domain/SSH host-key/permissions are explicitly approved, run the
shipped lifecycle harness for a fresh exact-RC install and a supported-old-tag
to exact-RC upgrade with `--verify-egress-policy`, reboot and certificate
renewal. Use the target release's verified updater for older/hybrid installs
as documented in `docs/INSTALLATION.md`; do not run an older repair script.
Do not weaken TLS/SSH verification or omit a requested stage to get PASS.
Run real SIGKILL interruption stages twice and prove each injection fired.
Record source/artifact identity, credentials-preserved fingerprints, protocol
results, own-address/C-16 checks, DNS/family behavior and no residue.

Suppress onboarding credentials with `SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1`.
Identity creation is separately authorized; owner retrieves subscription links
through their private session. Keep raw logs and tokens outside Git/CI uploads.

Post-install checks on the authorized host:

```sh
date -u +%FT%TZ
sha256sum /usr/local/bin/vpn-admin /usr/local/bin/vpn /usr/local/bin/vpn-subscription-svc
vpn-admin doctor --protocol --require-protocol
systemctl --failed
ip -6 neigh
nft list table inet arcana_egress_isolation
bash /opt/singbox-vpn/deploy/lib/monitoring/install-shorts-monitor.sh --check --label exit-direct
```

Also verify the running sing-box executable/pin, current rendered-config
fingerprint, actual release checksums/attestations, permissions/service users,
public reachability on both transports, subscriber mutation/revocation,
expiry/refresh, reconciliation, watchdog, restart/reboot and alert delivery.
Doctor matching locally recorded hashes is not independent artifact proof.
Monitoring/backup check commands do not install those systems by themselves.

## Minimal native test procedure

Prerequisites: accepted staging artifact installed, same physical device and
network, recorded OS/device/Hiddify/core/YouTube versions, synchronized UTC,
no other clients or automated canaries during each capture window. Keep the
same known-available Short and long-form control and alternate A/B/A to reduce
time/session confounding. No cookies, account identifiers, tokens, full URLs
with signatures, screen contents or browsing history are collected.

1. Record release tag/source SHA, artifact digest, running binary SHA-256 and
   non-secret routing fingerprint. Confirm actual selected transport; an
   automatic selector or pinned REALITY-only import is not a Hysteria2 test.
2. On Wi-Fi, VPN off: signed-in and Incognito controls, same Short and normal
   video; record UTC, start delay/stalls/result. Repeat VPN-off controls when
   an item fails; aged/unavailable content is not a VPN failure by itself.
3. REALITY: signed-in 10 consecutive true Shorts; Incognito 10 consecutive
   true Shorts. Each must visibly play; count attempted and played separately.
   Include known failing Short. Test long-form play/seek and >=15 minutes
   continuous playback per mode; reconnect, force-close/reopen YouTube, and
   repeat the failing control after routine reconciliation (>=10 minutes).
4. Repeat step 3 explicitly over Hysteria2. Record continuity, startup delay,
   stalls, recovery and error text. Repeat on cellular and a network handover
   where available. Missing cellular coverage remains BLOCKED, never waived
   into DEVICE_VERIFIED. Check client-owned DNS and IPv4/IPv6/leak behavior.
5. Ensure intended Google egress with synchronized privacy-minimized server
   diagnostics; aggregate family share alone cannot prove a particular flow,
   app, media success or a lack of direct leakage. An approved hairpin requires
   actual no-unwanted-direct-egress verification, including target-down behavior.

If Incognito fails, run short matched signed-in/Incognito/signed-in windows
before any config change. Example capture on the isolated approved host:

```sh
# Replace interface/ports/server addresses with observed deployment values.
# Run in a separate session; no pcap is saved; duration capped at 1800 s.
umask 077
python3 deploy/lib/diagnostics/af_capture.py capture \
  --iface INTERFACE --duration 600 --state /tmp/shorts-window.json \
  --server SERVER_IPV4 --server SERVER_IPV6 --tunnel tcp:REALITY_PORT --tunnel udp:HYSTERIA_PORT
# In the control session, only begin playback after running=true and
# tunnel_seen_recently=true; do not infer that it uniquely identifies the phone.
python3 deploy/lib/diagnostics/af_capture.py status --state /tmp/shorts-window.json
date -u +%s
# Record start/end epoch seconds independently for each signed-in/Incognito run.
python3 deploy/lib/diagnostics/af_capture.py analyze --state /tmp/shorts-window.json --from START_EPOCH --to END_EPOCH
```

Use a current reviewed Google-prefix file via `--google-prefixes` when
available. Record canary windows with repeated `--canary START:END`, or avoid
canaries entirely. No overlap/partial coverage is INCONCLUSIVE. Compare actual
observed failures with controls; do not infer refusal from UMP length. The
existing browser evidence probe can supplement TCP/family comparisons but
cannot reproduce native signed-in state without sensitive account data; do
not collect that data. Do not enable a disabled agent to obtain a hairpin.

Enter actual records in `docs/DEVICE_ACCEPTANCE_TESTS.md` and a release evidence
file. `runtime_binary_sha256` identifies the accepted runtime executable;
each cell's `binary_sha256` must match it. Required cell fields include
`youtube_mode` (`signed_in`/`incognito`), YouTube version,
`continuous_playback_seconds`, and booleans `long_form_playback`, `seeking`,
`reconnect`, `app_restart`, `post_reconcile_playback`,
`google_egress_verified`. False/missing/unmeasured fields stay unqualified.
Validate the completed record with the shipped `release_evidence.py`, binding
the actual RC tag and commit. A valid record is still subject to human review
of its linked observations and artifact provenance.

## Cutover and rollback (only after explicit approval)

Cutover requires all mandatory gates on the same immutable artifact, trusted
host, tested off-host restore and native acceptance. Before cutover save the
verified state/config/certificate/identity backup and fingerprint, exact prior
artifact, provider snapshot and rollback trigger thresholds. Schedule a window;
restarts drop sessions. Check protocols/subscriptions/egress immediately and
after reconciliation/reboot; monitor resource/headroom, failures and alerts.

For an interrupted update, preserve its marker/logs and use the intended
release's `update.sh --recover`; it validates whether rollback or committed
roll-forward is safe. Do not delete markers or copy binaries by hand. Verify
doctor/protocols, exact identity and credentials afterward before retrying.
For a completed bad update, use the supported transactional downgrade only
when capability/schema guards allow it. Never silently clear an egress policy
to make an old binary accept the configuration. Do not return fin1 to rc.2's
known I-54 defect. If no safe previous release exists, withdraw service and
restore the trusted pre-install snapshot/state through the provider console.
Off-host restoration is into empty disposable scratch first, verifies manifest
and permissions, and needs a separate approval before copying to live paths.
Retest both protocols and old subscriptions; never assume restored secrets are
valid until verified. A rebuilt host's lost old identities cannot be recovered
from GitHub source.
