//! Golden vectors for the route engine.
//!
//! `fixtures/platform-v2/route-engine/*.json` hold inputs and the exact
//! outputs this reference implementation produces. Any other
//! implementation (the Tamara client port) must reproduce every
//! `expected` value from the same `input`. Changing engine behaviour
//! therefore shows up as a fixture diff that must be reviewed, and bumps
//! `ScoringPolicy::version`.
//!
//! Regenerate after an intentional change:
//! `PLATFORM_CORE_UPDATE_GOLDEN=1 cargo test -p platform-core --test golden`.

use platform_core::failover::{Event, FailoverController, FailoverPolicy, SessionMode};
use platform_core::failure::{classify, ClassifierPolicy, FailureInput};
use platform_core::health::{HealthStore, Observation};
use platform_core::route::RouteCatalog;
use platform_core::scoring::{rank, ScoringPolicy};
use platform_core::transport::TransportKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/platform-v2/route-engine")
}

#[derive(Serialize, Deserialize)]
struct RankingInput {
    catalog: RouteCatalog,
    network_id: String,
    now_ms: u64,
    observations: Vec<Observation>,
}

#[derive(Serialize, Deserialize)]
struct FailoverInput {
    catalog: RouteCatalog,
    mode: SessionMode,
    supported_transports: Vec<TransportKind>,
    network_id: String,
    events: Vec<Event>,
}

fn ranking(input: &Value) -> Value {
    let input: RankingInput = serde_json::from_value(input.clone()).unwrap();
    let policy = ScoringPolicy::default();
    let mut store = HealthStore::default();
    for o in &input.observations {
        store.record(o, &policy.health);
    }
    let candidates: Vec<_> = input.catalog.routes.iter().collect();
    let decision = rank(&input.catalog, &candidates, &store, &input.network_id, input.now_ms, &policy);
    serde_json::to_value(decision).unwrap()
}

fn failover(input: &Value) -> Value {
    let input: FailoverInput = serde_json::from_value(input.clone()).unwrap();
    let mut c = FailoverController::new(
        FailoverPolicy::default(),
        ScoringPolicy::default(),
        input.mode,
        input.supported_transports.into_iter().collect(),
        input.catalog,
        input.network_id,
    );
    let steps: Vec<Value> = input
        .events
        .into_iter()
        .map(|e| {
            let actions = c.handle(e);
            json!({ "actions": actions, "state": c.state() })
        })
        .collect();
    Value::Array(steps)
}

fn classification(input: &Value) -> Value {
    let cases: Vec<FailureInput> = serde_json::from_value(input.clone()).unwrap();
    let policy = ClassifierPolicy::default();
    Value::Array(cases.iter().map(|c| serde_json::to_value(classify(c, &policy)).unwrap()).collect())
}

fn compute(kind: &str, input: &Value) -> Value {
    match kind {
        "ranking" => ranking(input),
        "failover" => failover(input),
        "classification" => classification(input),
        other => panic!("unknown golden kind {other}"),
    }
}

#[test]
fn route_engine_golden_vectors() {
    let update = std::env::var("PLATFORM_CORE_UPDATE_GOLDEN").as_deref() == Ok("1");
    let mut files: Vec<_> = std::fs::read_dir(dir())
        .expect("fixtures/platform-v2/route-engine exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(files.len() >= 5, "expected the committed golden set, found {}", files.len());
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["policy_version"], ScoringPolicy::default().version, "{path:?}: engine version changed; regenerate and review");
        let actual = compute(doc["kind"].as_str().unwrap(), &doc["input"]);
        if update {
            doc["expected"] = actual;
            std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
            continue;
        }
        assert_eq!(doc["expected"], actual, "{path:?} diverged from the reference implementation");
    }
}
