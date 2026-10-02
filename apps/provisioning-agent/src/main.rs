mod config;
mod dispatch;
mod external_authorization;
mod health_probe;
mod lease_pool;
mod op_dedup;
mod protocol_probe;
mod report_queue;
mod ssrf_guard;
mod stats;
mod telemetry;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use op_dedup::{OpDedupLog, Outcome as DedupOutcome};
use report_queue::ReportQueue;
#[cfg(all(test, unix))]
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use worker_client::WorkerClient;

const TRAFFIC_INTERVAL: Duration = Duration::from_secs(15);

// `version` makes `--version` print "vpn-provisioning-agent <x.y.z>", the
// shape deploy/lib/binary-version-check.sh verifies for every shipped binary.
#[derive(Parser)]
#[command(name = "vpn-provisioning-agent", version)]
struct Cli {
    #[arg(long, default_value = "/etc/vpn/provisioning-agent.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cfg = AgentConfig::load(&cli.config)
        .with_context(|| format!("loading agent config from {:?}", cli.config))?;
    tracing::info!(node_id = %cfg.node_id, worker_url = %cfg.worker_url, "provisioning agent starting");

    let client = WorkerClient::new(&cfg);
    // Phase 8: /complete and /fail reports are delivered by an independent
    // background task so a wedged Worker endpoint can never block the rest
    // of this loop (heartbeat, health probe, next-job poll, lease expiry).
    let report_queue = ReportQueue::spawn(PathBuf::from(&cfg.report_queue_file), client.clone());
    // A4 (Batch 5): operation-id dedup, opened once and reused across
    // every poll iteration — see `op_dedup.rs` for why and `poll_once`
    // below for how it's consulted before (re-)running a job.
    let op_dedup = OpDedupLog::open(PathBuf::from(&cfg.op_dedup_file));
    // Never permit a bad config value to become a CPU-burning busy loop or a
    // multi-minute provisioning delay.
    let poll_interval = Duration::from_secs(cfg.poll_interval_secs.clamp(1, 60));
    let mut telemetry = telemetry::TelemetrySampler::new();
    let mut next_heartbeat = Instant::now();
    let mut next_traffic = Instant::now();
    let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs.clamp(10, 60));
    let protocol_report: protocol_probe::SharedReport = Default::default();
    if cfg.protocol_probe.is_some() {
        tokio::spawn(protocol_probe::run_loop(
            cfg.clone(),
            protocol_report.clone(),
        ));
    }

    let mut lease_pool = match lease_pool::LeasePool::open(&cfg) {
        Ok(pool) => Some(pool),
        Err(err) => {
            // Never start without the persisted table: forgetting live
            // secrets would stop them from being rotated. Keep the rest of
            // the agent running and surface the problem loudly.
            tracing::error!(error = %err, "lease pool state unreadable; lease pool disabled until fixed");
            None
        }
    };
    let mut external_authorizations = match external_authorization::ExternalReconciler::open(&cfg) {
        Ok(state) => Some(state),
        Err(err) => {
            tracing::error!(error = %err, "external authorization state unreadable; reconciliation disabled until fixed");
            None
        }
    };

    if cfg.clash_api_url.is_none() {
        tracing::info!("clash_api_url not configured — traffic reporting disabled for this node");
    }

    loop {
        let now = Instant::now();

        if now >= next_heartbeat {
            let (probe_ok, probe_latency_ms) = health_probe::probe_data_plane(
                client.http(),
                cfg.clash_api_url.as_deref(),
                cfg.clash_api_secret.as_deref(),
                cfg.clash_probe_outbound.as_deref(),
            )
            .await;
            let mut payload = telemetry.collect(&cfg, probe_ok, probe_latency_ms);
            if let Some(report) = protocol_probe::take_report(&protocol_report) {
                payload["protocol_probe"] = report;
            }
            if let Err(err) = client.heartbeat(&payload).await {
                tracing::warn!(error = %err, "node heartbeat failed");
            }
            next_heartbeat = Instant::now() + heartbeat_interval;
        }

        if now >= next_traffic {
            report_traffic_once(&cfg, &client).await;
            next_traffic = Instant::now() + TRAFFIC_INTERVAL;
        }

        if let Some(pool) = lease_pool.as_mut() {
            // Runs every iteration: expiry enforcement is bounded by the
            // poll interval, and does not depend on the control plane.
            if let Err(err) = pool.tick(&cfg, &client).await {
                tracing::warn!(error = %err, "lease pool tick failed");
            }
        }

        if let Some(external) = external_authorizations.as_mut() {
            // The durable local snapshot is rendered on every process start
            // and whenever its active set changes, so expiry remains bounded
            // by this loop even during a control-plane outage.
            if let Err(err) = external.tick(&cfg, &client).await {
                tracing::warn!(error = %err, "external authorization reconciliation failed");
            }
        }

        match poll_once(&cfg, &client, &report_queue, &op_dedup).await {
            Ok(true) => {
                // A job was processed. Immediately claim the next one instead
                // of sleeping for the idle poll interval; this lets a single
                // node drain a signup/renewal burst as fast as vpn-admin can
                // safely apply the jobs.
                continue;
            }
            Ok(false) => {
                tokio::time::sleep(poll_interval).await;
            }
            Err(err) => {
                tracing::error!(error = %err, "poll iteration failed");
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}

async fn report_traffic_once(cfg: &AgentConfig, client: &WorkerClient) {
    let Some(clash_url) = cfg.clash_api_url.as_deref() else {
        return;
    };

    let sample = match stats::read_traffic(
        client.http(),
        clash_url,
        cfg.clash_api_secret.as_deref(),
    )
    .await
    {
        Ok(sample) => sample,
        Err(err) => {
            tracing::warn!(error = %err, "reading sing-box traffic counters failed");
            return;
        }
    };

    if let Err(err) = client.report_traffic(&sample).await {
        tracing::warn!(error = %err, "reporting traffic sample failed");
        return;
    }

    tracing::debug!(
        bytes_up = sample.bytes_up,
        bytes_down = sample.bytes_down,
        connections_open = sample.connections_open,
        "reported traffic sample"
    );
}

/// Returns true when a job was claimed/processed and false when the queue was
/// empty. The caller uses this to drain bursts without an artificial sleep.
async fn poll_once(
    cfg: &AgentConfig,
    client: &WorkerClient,
    report_queue: &ReportQueue,
    op_dedup: &OpDedupLog,
) -> Result<bool> {
    let Some(job) = client.claim().await.context("claiming a job")? else {
        return Ok(false);
    };
    apply_job(cfg, client, report_queue, op_dedup, &job).await?;
    Ok(true)
}

/// The claimed-job -> (dedup check ->) dispatch -> (dedup record ->) report
/// pipeline, factored out of `poll_once` so it can be exercised directly by
/// tests against a hand-built `Job` without needing to also mock the
/// Worker's `/api/agent/claim` endpoint. This is the actual production code
/// path `poll_once` runs — not a re-implementation of it.
async fn apply_job(
    cfg: &AgentConfig,
    client: &WorkerClient,
    report_queue: &ReportQueue,
    op_dedup: &OpDedupLog,
    job: &worker_client::Job,
) -> Result<()> {
    tracing::info!(job_id = job.id, job_type = %job.job_type, "claimed job");

    // A4 (Batch 5): if this exact job.id was already applied by a prior
    // instance of this loop (including one that crashed/restarted between
    // dispatch::run_job mutating state and report_queue durably recording
    // that it did), do not re-run vpn-admin — replay the recorded outcome.
    // This is what makes CREATE_USER/DELETE_USER/DISABLE_USER/ENABLE_USER/
    // ROTATE_CREDENTIALS/APPLY_NODE_REVISION converge to the same result
    // under redelivery instead of double-mutating (e.g. a second
    // credential on a repeated ROTATE_CREDENTIALS).
    // Only a recorded *success* short-circuits re-application. A recorded
    // failure is deliberately NOT replayed here: `dispatch::run_job`
    // itself already failed to mutate state (nothing to converge with),
    // and a transient cause (e.g. a network blip) may well have cleared
    // by the time the Worker redelivers the same job id — permanently
    // wedging retries on an old failure would be worse than the (bounded,
    // MAX_ATTEMPTS-limited) cost of trying again.
    match op_dedup.lookup(job.id) {
        Ok(Some(DedupOutcome::Complete(result))) => {
            tracing::info!(
                job_id = job.id,
                "job already applied (dedup hit); replaying recorded completion"
            );
            if let Err(err) = report_queue.enqueue_complete(job.id, result) {
                tracing::error!(job_id = job.id, error = %err, "failed to enqueue replayed completion report");
            }
            return Ok(());
        }
        Ok(Some(DedupOutcome::Fail(_))) | Ok(None) => {}
        Err(err) => {
            // Fail open on a corrupt/unreadable dedup log: still apply
            // the job rather than getting stuck, but log loudly, mirroring
            // how the lease pool degrades in `main()` above. Worst case on
            // this path is the pre-A4 behaviour (a possible re-apply under
            // redelivery), not a stuck node.
            tracing::error!(job_id = job.id, error = %err, "op dedup log unreadable; proceeding without dedup for this job");
        }
    }

    match dispatch::run_job(cfg, client, job).await {
        Ok(result) => {
            if let Err(err) = op_dedup.record(job.id, DedupOutcome::Complete(result.clone())) {
                tracing::error!(job_id = job.id, error = %err, "failed to record op dedup outcome (completion)");
            }
            // The side effect already happened; only the Worker's
            // acknowledgement is still pending. Hand it to the
            // independently-retrying report queue and move straight on to
            // the next job — a slow/wedged Worker /complete endpoint must
            // not stall provisioning for every other customer on this node
            // (Phase 8).
            if let Err(err) = report_queue.enqueue_complete(job.id, result) {
                tracing::error!(job_id = job.id, error = %err, "failed to enqueue job completion report");
            }
            tracing::info!(job_id = job.id, "job completed");
        }
        Err(err) => {
            let message = err.to_string();
            tracing::error!(job_id = job.id, error = %message, "job failed after retries");
            // Deliberately not recorded in op_dedup — see the lookup-side
            // comment above: nothing mutated, so there is nothing to
            // dedup against, and a future redelivery should be free to
            // try again.
            if let Err(queue_err) = report_queue.enqueue_fail(job.id, &message) {
                tracing::error!(job_id = job.id, error = %queue_err, "failed to enqueue job failure report");
            }
        }
    }

    Ok(())
}

/// A4 (Batch 5) regression tests: CREATE_USER, ENABLE_USER, DISABLE_USER,
/// ROTATE_CREDENTIALS and APPLY_NODE_REVISION applied twice under the same
/// operation id (`job.id`) must converge to one real vpn-admin invocation
/// and one recorded outcome, not two. (There is no DELETE_USER job type in
/// this agent's dispatch table today — `dispatch.rs`'s `run_job_once`
/// match only covers CREATE_USER, SET_EXPIRY, CLEAR_EXPIRY, ENABLE_USER,
/// DISABLE_USER, ROTATE_SUBSCRIPTION_TOKEN, ROTATE_CREDENTIALS and
/// APPLY_NODE_REVISION; DELETE_USER is not part of the current wire
/// contract, so it is not tested here — see the batch status doc.)
///
/// Exercises the real production path (`apply_job`, the function
/// `poll_once` calls after `client.claim()`), not a re-implementation of
/// it. `unix`-gated because it shells out to a real (fake) executable via
/// `tokio::process::Command`, matching the precedent already set by
/// `dispatch.rs`'s Phase 9 `timeout_kills_the_child_before_it_can_mutate_state`
/// test — verified by manual run on Linux-shaped CI; this Windows dev
/// machine cannot execute a `#!/bin/sh` script directly via `Command::new`,
/// which is exactly why that existing test is gated the same way.
#[cfg(all(test, unix))]
mod op_dedup_integration_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use worker_client::Job;

    fn test_config(
        base_url: &str,
        vpn_admin_binary: &Path,
        vpn_admin_config: &Path,
    ) -> AgentConfig {
        AgentConfig {
            worker_url: base_url.to_string(),
            node_id: "node-1".to_string(),
            agent_api_key: "test-key".to_string(),
            poll_interval_secs: 1,
            vpn_admin_binary: vpn_admin_binary.to_string_lossy().to_string(),
            vpn_admin_config: vpn_admin_config.to_string_lossy().to_string(),
            clash_api_url: None,
            clash_api_secret: None,
            clash_probe_outbound: None,
            lease_pool_size: 0,
            lease_slot_lifetime_secs: 1800,
            lease_state_file: "/tmp/unused-lease-state.json".to_string(),
            report_queue_file: "/tmp/unused-report-queue.json".to_string(),
            op_dedup_file: "/tmp/unused-op-dedup.json".to_string(),
            external_authorization_state_file: "/tmp/unused-external-auth.json".to_string(),
            rotation_batch_interval_secs: 60,
            heartbeat_interval_secs: 60,
            protocol_probe: None,
        }
    }

    /// A fake `vpn-admin` that only knows enough of the real CLI surface
    /// to answer the subcommands `dispatch.rs` issues for the verbs under
    /// test. Every invocation appends one line to `calls_log` (the ground
    /// truth this module checks "applied exactly once" against, since it
    /// can only grow when the *real* subprocess actually runs — a dedup
    /// hit in `apply_job` never spawns it at all). `user create` and
    /// `user rotate-credentials`-shaped output is deliberately
    /// counter-driven (a fresh id per real invocation) so a double-mutate
    /// bug would also be visible as a *changed* id/result, not just an
    /// extra log line.
    fn write_fake_vpn_admin(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("fake-vpn-admin.sh");
        let calls_log = dir.join("calls.log");
        let counter = dir.join("counter");
        std::fs::write(&counter, "0").unwrap();
        let script = format!(
            r#"#!/bin/sh
set -eu
calls_log="{calls_log}"
counter_file="{counter}"
n=$(cat "$counter_file")
n=$((n + 1))
echo "$n" > "$counter_file"

case " $* " in
  *" user create "*)
    echo "create $*" >> "$calls_log"
    printf '{{"id":"user-%s","subscription_url":"https://example.test/sub/%s","provisioning_url":"https://example.test/prov/%s"}}\n' "$n" "$n" "$n"
    ;;
  *" rotate-credentials "*)
    echo "rotate $*" >> "$calls_log"
    ;;
  *" enable "*)
    echo "enable $*" >> "$calls_log"
    ;;
  *" disable "*)
    echo "disable $*" >> "$calls_log"
    ;;
  *" apply-revision "*)
    echo "apply-revision $*" >> "$calls_log"
    for a in "$@"; do last="$a"; done
    cp "$last" "$(dirname "$calls_log")/last-revision.json"
    ;;
  *)
    echo "unknown invocation: $*" >&2
    exit 1
    ;;
esac
exit 0
"#,
            calls_log = calls_log.display(),
            counter = counter.display(),
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn count_calls_matching(dir: &Path, needle: &str) -> usize {
        let log = dir.join("calls.log");
        match std::fs::read_to_string(&log) {
            Ok(s) => s.lines().filter(|l| l.contains(needle)).count(),
            Err(_) => 0,
        }
    }

    async fn setup(
        dir: &Path,
    ) -> (
        AgentConfig,
        WorkerClient,
        wiremock::MockServer,
        std::path::PathBuf,
    ) {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path_regex(
                r"^/api/agent/jobs/\d+/(complete|fail)$",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let vpn_admin = write_fake_vpn_admin(dir);
        let vpn_config = dir.join("deployment.toml");
        std::fs::write(&vpn_config, "").unwrap();
        let cfg = test_config(&server.uri(), &vpn_admin, &vpn_config);
        let client = WorkerClient::new(&cfg);
        (cfg, client, server, dir.to_path_buf())
    }

    /// Runs `apply_job` for `job` twice, with the op-dedup log *reopened*
    /// (dropped and reconstructed at the same path) between the two calls
    /// — standing in for the agent process restarting, per the batch's
    /// explicit requirement to prove dedup survives that, not merely
    /// that it works within one process's lifetime.
    async fn apply_twice_across_a_simulated_restart(
        cfg: &AgentConfig,
        client: &WorkerClient,
        report_queue_path: &Path,
        dedup_path: &Path,
        job: &Job,
    ) {
        {
            let report_queue = ReportQueue::spawn(report_queue_path.to_path_buf(), client.clone());
            let op_dedup = OpDedupLog::open(dedup_path.to_path_buf());
            apply_job(cfg, client, &report_queue, &op_dedup, job)
                .await
                .expect("first apply_job must succeed");
        } // report_queue/op_dedup dropped here — nothing in-process survives.

        {
            let report_queue = ReportQueue::spawn(report_queue_path.to_path_buf(), client.clone());
            let op_dedup = OpDedupLog::open(dedup_path.to_path_buf()); // reopened, not shared
            apply_job(cfg, client, &report_queue, &op_dedup, job)
                .await
                .expect(
                    "second apply_job (post-restart, same job id) must also succeed, not error",
                );
        }
    }

    #[tokio::test]
    async fn create_user_twice_across_a_restart_does_not_mint_a_second_credential() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, client, _server, dir) = setup(dir.path()).await;
        let job = Job {
            id: 101,
            job_type: "CREATE_USER".to_string(),
            payload: serde_json::json!({"user_id": "alice"}),
        };
        apply_twice_across_a_simulated_restart(
            &cfg,
            &client,
            &dir.join("report-queue.json"),
            &dir.join("op-dedup.json"),
            &job,
        )
        .await;

        assert_eq!(
            count_calls_matching(&dir, "create "),
            1,
            "vpn-admin user create must run exactly once for one operation id, even across a restart"
        );
    }

    #[tokio::test]
    async fn rotate_credentials_twice_across_a_restart_does_not_rotate_twice() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, client, _server, dir) = setup(dir.path()).await;
        let job = Job {
            id: 102,
            job_type: "ROTATE_CREDENTIALS".to_string(),
            payload: serde_json::json!({"vpn_user_id": "user-1"}),
        };
        apply_twice_across_a_simulated_restart(
            &cfg,
            &client,
            &dir.join("report-queue.json"),
            &dir.join("op-dedup.json"),
            &job,
        )
        .await;

        assert_eq!(
            count_calls_matching(&dir, "rotate "),
            1,
            "a repeated ROTATE_CREDENTIALS under the same operation id must not generate a second credential"
        );
    }

    #[tokio::test]
    async fn enable_user_twice_across_a_restart_applies_once() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, client, _server, dir) = setup(dir.path()).await;
        let job = Job {
            id: 103,
            job_type: "ENABLE_USER".to_string(),
            payload: serde_json::json!({"vpn_user_id": "user-1"}),
        };
        apply_twice_across_a_simulated_restart(
            &cfg,
            &client,
            &dir.join("report-queue.json"),
            &dir.join("op-dedup.json"),
            &job,
        )
        .await;

        assert_eq!(count_calls_matching(&dir, "enable "), 1);
    }

    #[tokio::test]
    async fn disable_user_twice_across_a_restart_applies_once() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, client, _server, dir) = setup(dir.path()).await;
        let job = Job {
            id: 104,
            job_type: "DISABLE_USER".to_string(),
            payload: serde_json::json!({"vpn_user_id": "user-1"}),
        };
        apply_twice_across_a_simulated_restart(
            &cfg,
            &client,
            &dir.join("report-queue.json"),
            &dir.join("op-dedup.json"),
            &job,
        )
        .await;

        assert_eq!(count_calls_matching(&dir, "disable "), 1);
    }

    #[tokio::test]
    async fn apply_node_revision_twice_across_a_restart_fetches_and_applies_once() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, client, server, dir) = setup(dir.path()).await;
        // APPLY_NODE_REVISION additionally fetches the revision document
        // from the Worker before shelling out; a dedup hit on the second
        // application must skip that fetch too, not just the vpn-admin
        // invocation.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/agent/revision/7"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"config": {"schema_version": 1, "users": []}}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let job = Job {
            id: 105,
            job_type: "APPLY_NODE_REVISION".to_string(),
            payload: serde_json::json!({"revision": 7}),
        };
        apply_twice_across_a_simulated_restart(
            &cfg,
            &client,
            &dir.join("report-queue.json"),
            &dir.join("op-dedup.json"),
            &job,
        )
        .await;

        assert_eq!(count_calls_matching(&dir, "apply-revision "), 1);
        // `.expect(1)` above is checked on drop; explicitly verifying it
        // here too makes the assertion's presence non-optional in a
        // panic-swallowing test harness.
        server.verify().await;
    }

    /// Cross-repo contract: a static-config revision document served by
    /// vpn-web's `GET /api/agent/revision/:revision` (`config` =
    /// `{"revision_schema":1,"static_config":{..}}`) reaches
    /// `vpn-admin apply-revision --input` verbatim — the agent neither
    /// interprets, filters nor re-shapes it; all validation happens in
    /// `vpn-admin` (`compat_config::static_revision`).
    #[tokio::test]
    async fn apply_node_revision_passes_a_static_revision_document_through_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, client, server, dir) = setup(dir.path()).await;
        let static_doc = serde_json::json!({
            "revision_schema": 1,
            "static_config": {
                "hysteria2": {"up_mbps": 200, "down_mbps": 200},
                "udp_probe": {"ipv4_resolvers": ["9.9.9.9"]}
            }
        });
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/agent/revision/12"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"revision": 12, "config": static_doc})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let job = Job {
            id: 106,
            job_type: "APPLY_NODE_REVISION".to_string(),
            payload: serde_json::json!({"revision": 12}),
        };
        let report_queue = ReportQueue::spawn(dir.join("report-queue.json"), client.clone());
        let op_dedup = OpDedupLog::open(dir.join("op-dedup.json"));
        apply_job(&cfg, &client, &report_queue, &op_dedup, &job)
            .await
            .expect("apply_job must succeed");

        let delivered: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("last-revision.json")).unwrap())
                .unwrap();
        assert_eq!(delivered, static_doc);
        assert_eq!(count_calls_matching(&dir, "--revision 12 "), 1);
        server.verify().await;
    }
}
