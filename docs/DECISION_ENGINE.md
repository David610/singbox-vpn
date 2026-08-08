# DECISION_ENGINE.md

## Scoring model (`crates/policy`)

Each `(transport, endpoint)` pair has a `Score` updated from observed
outcomes, not a single global counter mixing everything together:

```rust
pub struct Score {
    ewma_success: f32,       // exponentially-weighted moving average, recent-weighted
    total_attempts: u32,
    consecutive_failures: u16,
    last_failure_category: Option<FailureCategory>,
    quarantined_until: Option<Instant>,
}
```

Update rule on outcome `o` (success or failure with category):

```
ewma_success' = alpha * outcome_value(o) + (1 - alpha) * ewma_success
```

`alpha = 0.3` (recent observations matter more, but history isn't erased by
one blip) — chosen and documented here rather than a hand-wavy multi-term
weighted sum, because a single bounded EWMA in `[0,1]` is easy to reason
about, easy to test the bounds of, and avoids tuning five separate
coefficients no one can justify. `outcome_value` is `1.0` for success,
`0.0` for a transport/endpoint-attributable failure, and *skipped entirely*
(no update) for `LocalNetworkFailure` per the invariant above.

Quarantine: after `consecutive_failures >= 3` for an endpoint, it is
excluded from selection for a randomized cooldown (`60s..300s`, jittered so
many clients don't retry in lockstep) rather than deleted. A single
client's quarantine is purely local state — see RENDEZVOUS_DESIGN.md for
why local failure must not become global delisting.

## Selection: weighted, non-deterministic, policy-constrained

`policy::select_next(candidates, rng)` does **not** try transports in a
fixed `A -> B -> C` order. It:

1. Filters out quarantined and capability-incompatible candidates.
2. Weights remaining candidates by `max(ewma_success, floor)` where
   `floor = 0.05` so a currently-low-scoring transport is *not* fully
   memory-holed (censorship conditions change; needs occasional
   re-evaluation) but is picked rarely.
3. Draws via weighted random sampling (`rand::distributions::WeightedIndex`)
   seeded from the process CSPRNG — not a deterministic hash of time or
   endpoint id, specifically so an observer cannot predict the client's
   next choice from watching prior choices (spec §8's fingerprintable
   probing-state-machine warning).
4. Adds bounded random jitter (0–2s) before the next connection attempt
   after a failure, instead of a fixed backoff constant, for the same
   reason.

This is deliberately simple and testable (property tests assert selection
never returns a quarantined or incompatible candidate, and that over many
draws the empirical distribution tracks the weights within tolerance) —
per spec §8, no unexplained ML black box for the MVP.

## What this does not do

No cross-client learning/model sharing exists yet (would require the
deferred control plane/measurement aggregation service). Today the engine
only learns from the local client's own observations plus whatever the
rendezvous-provided relay metadata already encodes (declared capabilities,
provider/AS tags) — it does not yet fold in corroborated global signals.
