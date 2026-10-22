# Rust dependency review and decisions (issue #166)

Dated 2026-10-22, against `main` at `b3061c5` (workspace, 265 packages in
`Cargo.lock`, pinned toolchain Rust 1.94.1). This records what was checked and
what was decided; it is not a perpetual clean bill of health.

## Done

| Item | Result | Evidence |
|---|---|---|
| Yanked `yoke-derive` 0.8.3 | Replaced by 0.8.4, one package in `Cargo.lock`, no blanket audit suppression | `cargo audit` warned on the old lockfile and not on the new; PR #176 |
| `rustls-webpki` 0.103.16, `webpki-roots` 1.0.10 (RUSTSEC-2026-0382 / 0381 fixes) | Preserved. `Cargo.lock` was never replaced with a stale PR lockfile | `cargo tree -i rustls-webpki`: reached only through `reqwest` in `provisioning-agent` |
| `cargo audit` on main | Succeeds | CI job `audit`, run 39855914650, job 120094180084 |
| Full tests, clippy, release build, real sing-box interoperability | Succeed on `b3061c5` | same run: `test` 120094180260, `clippy` 120094180366, `release-build` 120094180125, `singbox-validate` 120094180288 (12 real interop tests, manifest-checked) |
| SBOM | The release SBOM omitted the shipped `vpn-provisioning-agent` and with it the TLS stack. Fixed (new finding I-91, PR #177): 142 -> 193 components, includes `rustls-webpki` 0.103.16 and `webpki-roots` 1.0.10 | `deploy/lib/tests/test-release-sbom.sh`; local dry run |

## Checked, no action needed

- **Licenses.** All 265 packages carry license metadata (the SBOM generator fails
  closed otherwise). Expressions are permissive: MIT, Apache-2.0, ISC, BSD-2/3,
  Unicode-3.0, Zlib, CDLA-Permissive-2.0, Unlicense, BSL-1.0, and `OR` choices.
  Two crates offer `LGPL-2.1-or-later` only as one alternative of
  `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; the permissive choice applies.
- **MSRV.** The highest `rust-version` declared in the graph is 1.88.0
  (`time`, `time-core`, `time-macros`), below the pinned 1.94.1.
- **Checksums.** All 259 registry crates in `Cargo.lock` have a registry SHA-256 (checked); the other 6 packages are the workspace crates;
  the SBOM carries them.

## Decisions

### Bulk lockfile updates (PR #161): NOT_PLANNED
`cargo update --dry-run` would move 71 packages to newer compatible versions
(tokio, serde_json, thiserror, proptest, sha2, libc and many transitive crates).
None is tied to an advisory: `cargo audit` is clean. The proposed #161 update
also pulls new transitive paths (`bit-set`/`bit-vec`, `borsh`, `proc-macro-crate`)
that would need their own review. A churn-only update of that size is not a
release requirement and is not made. Reassess immediately if a relevant
advisory is published, and then update only the affected crates.

### Application `rand` 0.8 -> 0.10 (PR #121): NOT_PLANNED
The application uses `rand` 0.8.7 in exactly one place: `OsRng.fill_bytes` in
`crates/compat-config/src/credentials.rs` (6 call sites), which generates
subscription tokens, VLESS UUIDs, Hysteria2 and obfuscation passwords and REALITY
short ids. There is no advisory against 0.8.7 and `cargo audit` is clean.
Moving to 0.10 changes the trait and `OsRng` paths in security-sensitive
credential generation for no functional or security benefit today, and #121
changed the requirement without adapting the call sites (its CI failed).
Not scheduled. If it is ever done deliberately, it needs: every call site
adapted, a test that the output is full-entropy and correctly sized for each
credential type, and no lockfile-only shortcut. The `rand` 0.10 already present
transitively (through dev/test dependencies) is separate and unchanged.

## Not done

- No fresh exact-commit Security run for a hypothetical bulk update, because none
  was made.
- Dependabot/GitHub alert inventory is not accessible with the available token,
  so "no open alerts" is not claimed.
- A second `cargo audit` run on the new lockfile hit HTTP 503 from the registry
  while checking yanked crates and is not counted as evidence; the CI `audit`
  job above is.
