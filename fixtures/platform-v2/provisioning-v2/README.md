# Provisioning schema v2 fixtures

Synthetic documents for client conformance. Every key and credential here is
fake (repeating byte patterns or `FAKE`/`BAKE`/`CAKE` strings).

* `0x-*.json` must parse and validate. A client must build routes from `01` and
  `02`, and must parse `03` while skipping the route through the unknown
  transport.
* `9x-invalid-*.json` must be refused by a conforming client.

Checked by `crates/provisioning-contract/tests/v2_fixtures.rs`.
