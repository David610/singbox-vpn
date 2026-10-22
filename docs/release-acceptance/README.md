# Stable release acceptance records

This directory contains one human-readable, machine-checked acceptance record per stable release.

Do not add a stable acceptance record until the exact release candidate named by the record has completed the required VPS, device, and certificate-renewal acceptance in `../RELEASE.md`.

Required exact lines:

```text
Release: vX.Y.Z
Decision: ACCEPTED
Accepted RC: vX.Y.Z-rc.N
Accepted RC commit: <40-character commit SHA>
VPS acceptance: PASS
Device acceptance: PASS
Certificate renewal: PASS
```

Also record links/notes needed to reproduce the decision, but never include credentials, subscription tokens, passwords, private keys, REALITY private material, or TLS private keys.

For the standalone literal-IPv6 policy (v1.3.0) and its native iPhone/VPS
acceptance gates, see [exit policy](../EXIT_IPV6_POLICY.md). Rejection is not
proof of native IPv4 retry. Published v1.2.0 evidence does not accept v1.3.0.

## Machine-readable evidence (required for every stable release after v1.3.0)

The text lines above are not sufficient on their own. Each new stable release
also needs `docs/release-acceptance/<tag>.evidence.json`, validated by
`deploy/lib/release_evidence.py --require PRODUCTION_READY` in the release
workflow. The file binds tag, commit, artifact digests, CI run, VPS lifecycle
result, the native-device matrix (reality/hysteria2 x wifi/cellular, each with
device, OS, app version, >=10 consecutive Shorts, the known-failing Short and an
evidence reference), the deployed configuration fingerprint against the
approved one, and the security gates.

Levels: `CODE_QUALIFIED` < `SERVER_QUALIFIED` < `DEVICE_VERIFIED` <
`PRODUCTION_READY`. The validator computes the level the evidence supports; a
claimed level above it is an error. A **waiver is a limited-risk decision, not a
verification**: it is recorded under `waivers` (scope, reason, approver, date,
risk), can never raise a level, and a release that needs one cannot reach
`PRODUCTION_READY` (publish it as `SERVER_QUALIFIED` with the waiver stated).

v1.0.0-v1.3.0 predate this gate. v1.3.0's "Device acceptance: PASS" rests on
user-reported, reduced evidence with the full matrix waived (see its record);
read it as `SERVER_QUALIFIED` with a waiver. Thresholds and sample sizes:
`../operations/ACCEPTANCE_THRESHOLDS.md`.
