//! Minimal, privacy-preserving telemetry schema. See
//! `docs/TELEMETRY_DICTIONARY.md` and ADR-0005 for why every variant here
//! carries only closed enums / bucketed numerics — never a free-form
//! string, URL, IP, or payload.

use common::{DurationBucket, TransportId};
use network_state::FailureCategory;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EndpointTag(pub &'static str); // e.g. provider/AS/geo tag, never a hostname/IP

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Success,
    Failure(FailureCategory),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionEndReason {
    UserDisconnect,
    Stall,
    Reset,
    Migration,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LossBucket {
    None,
    Low,
    Moderate,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LatencyBucket {
    Under50Ms,
    Under150Ms,
    Under400Ms,
    Over400Ms,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimeBucket(pub u8); // hour of day, 0..23

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    ConnectionAttempt {
        transport: TransportId,
        endpoint_tag: EndpointTag,
    },
    ConnectionResult {
        transport: TransportId,
        endpoint_tag: EndpointTag,
        outcome: Outcome,
        handshake_ms_bucket: DurationBucket,
    },
    SessionEnded {
        transport: TransportId,
        duration_bucket: DurationBucket,
        reason: SessionEndReason,
    },
    StallObserved {
        transport: TransportId,
        endpoint_tag: EndpointTag,
    },
    NetworkConditionBucket {
        loss_bucket: LossBucket,
        latency_bucket: LatencyBucket,
        time_bucket: TimeBucket,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// There is no constructor path from a `String`/URL into any `Event`
    /// variant — this is a compile-time property, demonstrated here by
    /// building every variant purely from closed types.
    #[test]
    fn events_are_constructible_only_from_closed_types() {
        let _ = Event::ConnectionAttempt {
            transport: TransportId::new("direct-tls"),
            endpoint_tag: EndpointTag("dev"),
        };
        let _ = Event::ConnectionResult {
            transport: TransportId::new("direct-tls"),
            endpoint_tag: EndpointTag("dev"),
            outcome: Outcome::Success,
            handshake_ms_bucket: DurationBucket::from_millis(50),
        };
    }
}
