# ADR-0008: Three-tier signing hierarchy

## Context
Spec §11/§27: compromise of an online configuration/rendezvous server must
not allow arbitrary unsigned code execution or unbounded config forgery.

## Decision
```
offline root key            (ed25519, never touches a network-connected
                              machine; signs release keys only)
        |
release signing key         (signs bundle-signing-key certificates and,
                              in a future transport-runtime, transport
                              module manifests; kept on a build/release
                              machine, not the always-online rendezvous host)
        |
bundle signing key           (the only key the always-online rendezvous
                              process holds; signs individual RelayBundles;
                              rotated frequently; revocable via a
                              release-key-signed revocation list without
                              needing the offline root)
```
Implemented in `crates/crypto::hierarchy` and exercised by
`crates/config::tests::rejects_bundle_signed_by_revoked_key`.

## Alternatives considered
- Single key for everything: rejected — compromising the always-online
  rendezvous process would then compromise the entire trust root.
- Two-tier (root signs bundle key directly): simpler, but then rotating
  the bundle key requires the offline root every time, defeating the
  point of frequent rotation for the online-exposed key.

## Consequences
Key rotation procedure and revocation-list format have a documented,
testable shape (`crates/config::revocation`), and the offline-signing
operational tooling now exists as `apps/keytool` (`vpn-keytool`): a
network-free CLI for `root-init` / `release-issue` / `bundle-issue` /
`revoke-issue` / `verify-chain`, with real key files persisted at mode
0600 (`crypto::KeyPair::save_to_file` / `load_from_file`) instead of an
ephemeral hierarchy generated on every `rendezvous`/`relay-agent` boot.
`rendezvous --key-dir` and `relay-agent --identity-dir` load persisted
material; the fallback ephemeral-generation path is kept only for local
dev convenience and now logs a `warn`, not an `info`. See
`docs/DEPLOYMENT.md` §"Production key management" for the ceremony
runbook and `apps/keytool/tests/ceremony.rs` for the end-to-end
sign/verify/rotate/revoke test against real files.
