use common::{EndpointId, TransportId};
use criterion::{criterion_group, criterion_main, Criterion};
use network_state::FailureCategory;
use policy::{Candidate, ScoreBoard};
use rand::rngs::StdRng;
use rand::SeedableRng;

fn bench_scoreboard(c: &mut Criterion) {
    let candidates: Vec<Candidate> = (0..10)
        .map(|i| Candidate {
            transport: TransportId::new(if i % 2 == 0 {
                "direct-tls"
            } else {
                "noise-quic"
            }),
            endpoint: EndpointId(format!("relay-{i}")),
        })
        .collect();

    c.bench_function("observe_failure + select_next (10 candidates)", |b| {
        let mut rng = StdRng::seed_from_u64(42);
        b.iter(|| {
            let mut board = ScoreBoard::new();
            for c in &candidates {
                board.observe_failure(c, FailureCategory::TcpReset, &mut rng);
            }
            board.select_next(&candidates, &mut rng)
        })
    });
}

criterion_group!(benches, bench_scoreboard);
criterion_main!(benches);
