# Release policy

This document is the authoritative release process for singbox-vpn.

The purpose of the policy is simple: a release is not considered production-ready merely because it compiles or because CI is green. Stable releases must also have reproducible real-host and real-device acceptance evidence.

## Evidence levels

Release decisions use the evidence vocabulary in `DEVICE_ACCEPTANCE_TESTS.md`:

- CODE-VERIFIED / CI-VERIFIED: source or CI evidence.
- SERVER-VERIFIED: exercised on a real disposable VPS.
- DEVICE-VERIFIED: exercised on a real client device/network.

A stable release requires all three levels where applicable.

## Release channels

### Development

Unreleased `main` is development code. The stable bootstrap must never silently install development branch source.

### Release candidate

A release-candidate tag contains a SemVer prerelease suffix, for example:

```text
v1.0.0-rc.1
```

Release candidates may be published automatically from a matching prerelease tag after the normal release workflow gates pass. They are marked as GitHub prereleases and are not selected by the normal stable installer.

### Stable

A stable tag has no prerelease suffix, for example:

```text
v1.0.0
```

Stable releases are manual-only. Creating or pushing a stable tag does not by itself publish a stable GitHub Release. The release workflow requires an explicit manual dispatch from that exact tag plus a committed acceptance record.

## Mandatory release sequence

1. Start from a clean, reviewed `main` with green CI and Security workflows.
2. Set the workspace version to the intended RC version, e.g. `1.0.0-rc.1`.
3. Create and push the matching RC tag, e.g. `v1.0.0-rc.1`.
4. Let the release workflow build, attest, publish, and verify the immutable RC artifacts.
5. Run `.github/workflows/vps-acceptance.yml` against that exact immutable RC on a disposable AlmaLinux 9 x86_64 VPS. Do not use `main` and do not use an unverified development-channel run as release evidence.
6. Record the real VPS result in `docs/DEVICE_ACCEPTANCE_TESTS.md`, including date, release tag, commit, OS, architecture, provider/region where appropriate, and the lifecycle result.
7. Run the real-device acceptance checklist against the same RC. At minimum for the primary supported release path, record a real Hiddify client test covering REALITY, Hysteria2, subscription refresh, ordinary browsing, sustained transfer, reconnect/idle behaviour, and any relevant network handover checks.
8. Run certificate-renewal acceptance on the disposable release environment where applicable, including firewall TCP/80 handling and post-renewal service health.
9. If any implementation, deployment, workflow, dependency, or protocol code changes after RC acceptance, publish a new RC and repeat the acceptance sequence. Do not carry acceptance evidence forward across code changes.
10. Once the RC is accepted, change only stable-release metadata/documentation needed to cut the final release. The stable release must remain a metadata/docs-only descendant of the accepted RC.
11. Add `docs/release-acceptance/<stable-tag>.md` using the exact format below.
12. Set the workspace version to the stable version, create the stable tag, and manually dispatch the Release workflow from that exact tag with `confirm_stable_release=true`.
13. Verify the published stable bootstrap and release assets.
14. After publication, run one final smoke installation through the public stable one-command bootstrap.

## Stable acceptance record

Every stable release must contain a file named:

```text
docs/release-acceptance/<stable-tag>.md
```

For example:

```text
docs/release-acceptance/v1.0.0.md
```

The file must contain these machine-checked lines exactly once:

```text
Release: v1.0.0
Decision: ACCEPTED
Accepted RC: v1.0.0-rc.1
Accepted RC commit: <40-character commit SHA>
VPS acceptance: PASS
Device acceptance: PASS
Certificate renewal: PASS
```

It should also include human-readable evidence links/notes, for example:

```text
VPS acceptance run: <GitHub Actions run URL>
Device evidence: docs/DEVICE_ACCEPTANCE_TESTS.md
Accepted by: <maintainer>
Accepted date: YYYY-MM-DD
Notes: <optional>
```

Never put subscription tokens, passwords, private keys, REALITY private material, TLS private keys, or other credentials in the acceptance record.

## Stable-release technical gate

`.github/workflows/release.yml` enforces the following for a stable tag:

- the release must be started manually with `workflow_dispatch`;
- the workflow must be run from the exact tag being published;
- `confirm_stable_release` must be explicitly set to `true`;
- the stable acceptance record must exist;
- the record must identify the exact stable tag and be marked `ACCEPTED`;
- VPS, device, and certificate-renewal acceptance must all be recorded as `PASS`;
- the accepted RC tag must exist;
- the recorded accepted RC commit must match the RC tag's actual commit;
- the accepted RC must be an ancestor of the stable tag;
- changes between the accepted RC and stable tag are restricted to release metadata and acceptance documentation. Implementation/workflow changes require a new RC.

The release workflow still runs the complete canonical CI graph before any release artifacts are built.

## Allowed changes between accepted RC and stable

The stable tag may differ from the accepted RC only in:

- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `docs/DEVICE_ACCEPTANCE_TESTS.md`
- the stable acceptance file itself under `docs/release-acceptance/`

Any change to source code, deployment code, GitHub Actions workflows, systemd units, nginx templates, dependency manifests other than version metadata, or security-sensitive implementation requires another RC and another acceptance cycle.

## Repository governance required for releases

Before treating a stable release as production-ready, repository settings must enforce:

### `main`

- require a pull request before merge;
- require the relevant CI and Security checks to pass;
- disallow force pushes;
- disallow branch deletion;
- do not allow bypass except for emergency recovery with an explicitly documented reason.

### `v*` tags

- prevent mutation/deletion of release tags;
- do not reuse or move a published release tag.

### Workflow changes

Changes under `.github/workflows/**` are supply-chain-sensitive and must receive the same review discipline as production code.

These settings live outside the repository contents and therefore must be verified in GitHub repository settings/rulesets; documentation alone is not protection.

## Release rollback

Never move an existing release tag backward or replace assets under the same version to fix a bad release.

If a release is bad:

1. mark/deprecate it clearly;
2. fix the issue on a new branch/PR;
3. publish a new version;
4. repeat the required release gates appropriate to the change.

## Stop condition

Once the stable release passes the policy above, do not invent additional hardening phases without evidence. Future work should be driven by a real bug, security advisory, upstream compatibility change, OS/client regression, or explicitly approved product requirement.
