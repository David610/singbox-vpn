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
