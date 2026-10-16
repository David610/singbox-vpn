# 06 - Security and privacy

Scope honesty: this was a functional and reliability investigation. It was **not** a full
security assessment. Items are grouped by what was actually verified.

## Verified properties (server verified unless noted)

- Installer integrity: stable install verified the release's `SHA256SUMS` and the cosign
  attestation; installed `vpn-admin`, `vpn-provisioning-agent` and `vpn-subscription-svc`
  equal the release tarball's binaries; `doctor` records and re-checks the hashes.
- REALITY key pair: `doctor` derives the public key from the private key (X25519) and
  checks the on-disk config equals what would be rendered; a base64url key with `-` and `_`
  (including a leading `-`) round-trips through the subscription and a real client handshake.
- File ownership and modes (`doctor`, L2): sing-box config `root:sing-box 0640`, REALITY
  private key `root:sing-box 0640`, public key/short id/user store `root:vpn-subscription 0640`.
- Network surface after install: sshd, 443/tcp, 443/udp, 8443/tcp (nginx); the
  subscription backend is loopback-only. `doctor` flagged my temporary extra listener,
  so unexpected-listener detection works.
- TLS: certificates valid (89 days), renewal timer present. TLS verification was never
  disabled for any VPN or subscription connection. Two throwaway diagnostics used
  `curl -k`: a probe of Cloudflare's IPv6 literal, and the own-address reachability test
  against the node's nginx (a connect/no-connect check, no credentials sent).
- Revocation: the subscription endpoint returns 404 for revoked, expired and malformed
  tokens (vpn-web endpoint tests). A removed user's credentials are rejected after the
  restart (CLI states so; not separately probed with the old credential).
- vpn-web endpoint never puts internal identifiers, the service key, the obfuscation
  password (in JSON) or other secrets in the body (endpoint test).
- Server-side egress isolation unit and nftables table present after install and removed
  by uninstall.

## Defects found

| ID | Finding | Status |
|---|---|---|
| S-1 | Tunnel users can reach services on the node's own public address on a default install with a DNS `public_host` (R5). A paying customer could probe or attack the node's nginx/ssh surface from inside the tunnel. | fixed on branch `fix/server-hardening-rc2` (9ca903e): a fresh install now records the node's own addresses. Verified on a real VPS (rc.2 round): unmodified `main` lets a tunnel user read the node's sshd banner and nginx; the candidate refuses both (curl rc 35 / 52) while ordinary internet traffic still works, also after a reboot. **Not in any release yet** |
| S-2 | The new "own address" doctor check reported FAIL for a correct hardened config (it told operators to set the fields, then rejected the result). A false FAIL pushes operators to ignore the check. | fixed, commit 63ad42c |
| S-3 | Hysteria2 caps rendered `ignore_client_bandwidth: true` in v1.3.0, locking out every standard client: an availability failure that `sing-box check` and the self-test cannot see. Without caps, a client declaring a large rate can take most of a shared link (per the fix commit's own measurements). | fixed on `main` (9a07503), **not in any release** |
| S-4 | The installer's self-test and `doctor --protocol` exercise REALITY only, so Hysteria2 breakage passes health checks. | fixed on branch (0c7118a): both transports are handshaked with a throwaway client; the overall result passes only if both do. Verified on a real VPS: with a valid certificate for the wrong hostname, baseline `doctor --protocol` exits 0, the candidate exits 1 with the cause. The check is a loopback handshake and does not prove external UDP reachability (I-21). **Not in any release yet** |
| S-5 | vpn-web served Hysteria2 links with the REALITY decoy as SNI, so a verifying client rejected the node's certificate (`x509: certificate is valid for <node host>, not www.cloudflare.com`). Fixed by using the node's certificate name; no certificate verification was weakened. | fixed |
| S-6 | v1.3.0 predates a rustls bump made for RUSTSEC-2026-0346..0349 and `vpn-benchmark` restarting production by default (both on `main`, per their commit messages). I did not independently re-run `cargo audit`. | needs a corrected stable release |
| S-7 | `SINGBOX_VPN_CHANNEL=dev` over an existing install silently reuses the old source tree and records "main" (R6): an operator could believe reviewed code is deployed when it is not. | fixed on branch (a4e8f16, 10ad1b0): the requested source is what gets built, a full commit SHA can be pinned and is verified against the archive, and the manifest/doctor show the exact commit and tree digest. Caveat: a ref whose own installer predates the fix still reuses the persistent tree. **Not in any release yet** |
| S-8 | Install output prints the first user's subscription URL and a QR code that encodes the token (by design, "shown once"); the same text lands in any install log. I printed one in my own session output; test tokens were disposable. | accepted by design; operators should not keep install logs |

## Not tested

DNS leaks, IPv4/IPv6 leaks, fail-open/kill-switch, rate limits, replay resistance, token
unpredictability (statistical), symlink/path attacks in the installer, config injection,
dependency/supply-chain scan, server log contents (secret logging), cross-user isolation
on a live node, Telegram Mini App behaviour.

## Handling of secrets during this work

Credentials were taken from a chat message. A dedicated SSH key was installed and used for
the rest of the session; the root password should be rotated. Test subscription tokens,
UUIDs, REALITY keys and obfuscation passwords were kept in a protected local directory
outside the repository and are not in any commit or in `test-results/` (scanned with the
actual values). Two test tokens appeared in session output; both belong to throwaway users.
