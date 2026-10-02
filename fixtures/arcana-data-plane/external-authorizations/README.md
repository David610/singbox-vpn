# External authorization contract fixtures

These fixtures mirror the `vpn-web` PR #63 `fixtures/arcana-data-plane/` contract as described for the October 2026 rollout. They are versioned here so `singbox-vpn` validates both the exact legacy response currently emitted by `vpn-web/main` and the proposed schema-v2 follow-up without cross-repository access at test time.

`v2-stale-revision.json` deliberately repeats revision 10: tests use it after a simulated control-plane advance to prove the agent acknowledges the exact applied revision rather than an implicit "latest" revision. Files with `invalid` in their name are negative privacy-schema fixtures and must never parse.

`v2-empty.json` is the canonical initial node snapshot: revision `0` is valid,
authoritative, applied as an empty external set, and acknowledged only after live
verification. Transport or validation failure is never synthesized as this fixture.
