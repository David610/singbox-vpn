# Platform v2 — migration plan

Every step is **additive and reversible** until an operator explicitly opts in. No existing user, CLI command, provisioning URL, backup or fallback client stops working because a binary was upgraded.

## 1. Compatibility invariants (tested, not promised)

| Invariant | Test that holds it |
|---|---|
| `/v1/provision/{token}` output is byte-identical for deployments that do not enable new features | existing `contract_fixtures.rs` and subscription tests (unchanged), plus a new "v1 never contains AWG / nodes / routes" test |
| `/sub/{token}` formats are unchanged | existing subscription tests; share-link and sing-box JSON never include AWG |
| `deployment.toml` without `[amneziawg]` stays schema v2 and byte-identical after `config migrate` | `deployment.rs` migration tests |
| `users.json` without AWG credentials stays schema v1 and byte-identical | `store.rs` round-trip tests |
| An **older** `vpn-admin` refuses (never silently drops) state written with AWG enabled | schema guard tests (see §3) |
| Existing CLI commands and exit codes are unchanged | `apps/admin/tests/cli.rs` (unchanged) |
| Backups from before v2 restore on v2; v2 backups with AWG restore only on AWG-aware builds (explicit refusal otherwise) | backup/restore tests |

## 2. Contract versioning

| Step | Change | Old Tamara (`schema_version == 1` only) | Fallback clients |
|---|---|---|---|
| M1 | Add `/v2/provision/{token}` (schema 2: nodes, endpoints, routes, AWG) | Unaffected: never requests `/v2`. If pointed at `/v2`, it fails with its explicit "unsupported schema" error. | Unaffected |
| M2 | `/v1/provision` answers `?schema_version=2` with HTTP 400, pointing to `/v2` | Unaffected | — |
| M3 | Tamara adds a v2 parser behind feature detection: try `/v2`, fall back to `/v1` on HTTP 400/404 | New Tamara works with old servers | — |
| M4 | Server keeps serving v1 for at least two Tamara release cycles after M3 ships to all platforms | — | — |

`schema_version: 1` never gains AWG. Its `singbox_config` invariant (select group == endpoint tags + auto) stays exactly as it is.

## 3. On-disk schema changes

### 3.1 `deployment.toml` — schema 3 only when AWG is enabled

- A new optional `[amneziawg]` section.
- A file containing `[amneziawg]` **must** declare `schema_version = 3`, and `validate()` rejects an `[amneziawg]` section below 3.
- `vpn-admin transport enable amneziawg` writes the section and stamps 3 in one atomic write, after backing up the previous file (`migrate::backup_before_mutate`).
- A pre-v2 binary sees `schema_version = 3 > 2` and refuses to load. That is the intended fail-closed rollback behaviour: it cannot render a config that silently drops AWG peers.
- Files without `[amneziawg]` are never stamped 3; they stay v2 and byte-identical.
- **Rollback path for an operator who enabled AWG and must return to an older build:** `vpn-admin transport disable amneziawg` removes the section, restamps 2, stops `vpn-amneziawg.service` and removes its firewall rules. Only then is the older binary installed.

### 3.2 `users.json` — schema 2 only when any user has AWG credentials

- A new optional per-user `amneziawg` object: client private key (secret), preshared key (secret), allocated addresses, created_at and rotated_at.
- Saving sets `schema_version = 2` iff at least one user carries it; otherwise it stays 1 (byte-identical).
- An older binary sees 2 > 1 → `UnsupportedSchema` refusal. It cannot load and rewrite the file, so it cannot drop credentials.
- `transport disable amneziawg --purge-credentials` removes every user's AWG object and returns the file to schema 1.

### 3.3 Fleet catalog (`/etc/vpn/compat/fleet/fleet.toml`)

- A new file, absent by default. When absent, `/v2` renders a one-node catalog derived from `deployment.toml` (+ `[[peer_endpoints]]` mapped as nodes/endpoints/routes). **No operator action required.**
- `vpn-admin fleet init` writes an explicit catalog equivalent to the derived one; `fleet validate` checks it.
- Existing `[[peer_endpoints]]`/`[[access_paths]]` stay authoritative for v1. The derived v2 catalog maps each peer's `failure_domain` to a node id (`peer:<failure_domain>`) and each relay access path to a two-hop route.

## 4. Operator migration paths

### 4.1 One node, no changes wanted

Upgrade the binary as usual (`update.sh`). Nothing changes. `/v2` becomes available with REALITY + Hysteria2 routes derived from the existing state.

### 4.2 One node → add AmneziaWG

```bash
sudo vpn-admin backup --output /root/pre-awg.tar
sudo /opt/singbox-vpn/deploy/lib/amneziawg.sh install        # pinned build, verified, unit + firewall
sudo vpn-admin transport enable amneziawg --listen-port 51820
sudo vpn-admin user awg issue <user-id>                        # per user; or --all
sudo vpn-admin transport status
```

Rollback: `vpn-admin transport disable amneziawg --purge-credentials && deploy/lib/amneziawg.sh uninstall`.

### 4.3 One node → two nodes (independent providers)

1. Install node B normally (`install.sh --node-id b1`).
2. On B, create the same users (`vpn-admin user create`) and run `vpn-admin user export-credentials --for-authority --output b1-creds.json`.
3. On A (the authority), run `vpn-admin fleet init`, then `vpn-admin fleet node add --from-bundle b1-creds.json --provider … --region … --failure-domain …`.
4. Run `vpn-admin fleet validate`, then `vpn-admin endpoint list`.
5. Clients on `/v2` receive routes for both nodes on their next refresh. `/v1` clients keep what `[[peer_endpoints]]` already gave them.

### 4.4 Self-hosted → managed

Out of scope until a control-plane backend exists. The managed contract (`tamara-next/docs/contracts/managed-control-plane-v1.md`) and the node agent registration protocol (TARGET_ARCHITECTURE §8.1) are the integration points. Self-hosted state is never uploaded automatically.

## 5. Update, rollback and supply chain

- New pinned inputs live in `deploy/lib/versions.env`:
  - `AMNEZIAWG_GO_VERSION`, `AMNEZIAWG_GO_COMMIT`
  - `AMNEZIAWG_TOOLS_VERSION`, `AMNEZIAWG_TOOLS_COMMIT`
  - `GO_TOOLCHAIN_VERSION`, `GO_TOOLCHAIN_SHA256_AMD64`
- The installer verifies the Go toolchain tarball by SHA-256. It clones upstream at the exact commit (`git rev-parse HEAD` must equal the pin), runs `go mod verify` in an isolated module cache, and builds with `-trimpath`. The resulting binary hash is recorded in `install-state.json` for drift detection.
- AWG is **never auto-updated**. Changing the pin needs a commit that updates `versions.env`, passes the netns interop test in CI, and is released normally.
- An AWG protocol-parameter change (e.g. a new header-protection key) is a **client-breaking rotation**. It is executed only by `vpn-admin transport rotate amneziawg` and reported as requiring profile refresh.

## 6. Order of work (dependency order, matches the task)

1. Audit and docs (this directory)
2. Evidence and status framework
3. Transport abstraction
4. AWG
5. Route model and `/v2`
6. Benchmark harness
7. Scoring
8. Failover
9. Multi-node
10. Node agent and control-plane boundary
11. Russian probe
12. Provider drivers
13. Provisioning
14. Replacement
15. Telemetry
16. Client contracts
17. Acceptance tooling
18. Performance
19. Multi-hop evolution

What was actually completed in this pass is in [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).
