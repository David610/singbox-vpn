use crate::worker_client::WorkerClient;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// Bound on persisted pending reports (Phase 8: "bound the
/// retention/storage of operation IDs" — this queue is the completion-
/// report analogue). Far more than a single node should ever accumulate;
/// a wedged Worker endpoint can't grow this file without limit.
const MAX_PENDING: usize = 4096;
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Outcome {
    Complete(Value),
    Fail(String),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct Entry {
    job_id: i64,
    outcome: Outcome,
    #[serde(default)]
    attempts: u32,
}

/// Independent, disk-backed completion/failure report queue.
///
/// This is the fix for Phase 8's central reliability rule: a failed
/// `/complete` or `/fail` call to the Worker must never block the
/// agent's other periodic work (heartbeat, health probe, next-job poll,
/// lease expiry enforcement, traffic/stat reporting). The previous code
/// retried these calls inline — in `poll_once`, the same call chain the
/// main loop awaits before it can do anything else — with backoff that
/// grew unbounded in wall-clock terms; a wedged or slow Worker endpoint
/// therefore stalled the whole agent, including lease expiry enforcement
/// and heartbeats that have nothing to do with the failing report.
///
/// `enqueue_complete`/`enqueue_fail` persist the report and return
/// immediately; a background task (spawned by `spawn`) owns actually
/// delivering it, on its own schedule. The queue file survives an agent
/// restart, so a report queued right before a crash/restart is retried
/// rather than silently lost, and delivering it twice is safe because
/// the Worker's /complete and /fail endpoints are idempotent per job id
/// (job already terminal => no-op; see vpn-web's job-contract).
pub struct ReportQueue {
    path: PathBuf,
    wake: mpsc::UnboundedSender<()>,
}

impl ReportQueue {
    /// Starts the background delivery task (which first replays whatever
    /// was persisted from a previous run) and returns the handle callers
    /// enqueue through.
    pub fn spawn(path: PathBuf, client: WorkerClient) -> Self {
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(path.clone(), client, wake_rx));
        Self {
            path,
            wake: wake_tx,
        }
    }

    pub fn enqueue_complete(&self, job_id: i64, result: Value) -> Result<()> {
        self.enqueue(Entry {
            job_id,
            outcome: Outcome::Complete(result),
            attempts: 0,
        })
    }

    pub fn enqueue_fail(&self, job_id: i64, message: &str) -> Result<()> {
        self.enqueue(Entry {
            job_id,
            outcome: Outcome::Fail(message.to_string()),
            attempts: 0,
        })
    }

    fn enqueue(&self, entry: Entry) -> Result<()> {
        let mut entries = load(&self.path)?;
        if entries.len() >= MAX_PENDING {
            tracing::error!(
                path = %self.path.display(),
                max = MAX_PENDING,
                "report queue at capacity; dropping oldest pending report"
            );
            entries.remove(0);
        }
        entries.push(entry);
        save(&self.path, &entries)?;
        // Best-effort nudge; the background task also polls periodically,
        // so a dropped/lagging wake is not a correctness problem.
        let _ = self.wake.send(());
        Ok(())
    }
}

fn load(path: &Path) -> Result<Vec<Entry>> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => Ok(Vec::new()),
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("parsing report queue {path:?}"))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err).with_context(|| format!("reading report queue {path:?}")),
    }
}

fn save(path: &Path, entries: &[Entry]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Contains job results, which may carry subscription/provisioning
    // URLs with embedded tokens - never left world/group readable.
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec(entries).context("serializing report queue")?;
    std::fs::write(&tmp, &bytes).with_context(|| format!("writing {tmp:?}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).ok();
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming {tmp:?} to {path:?}"))
}

async fn run(path: PathBuf, client: WorkerClient, mut wake: mpsc::UnboundedReceiver<()>) {
    loop {
        let entries = match load(&path) {
            Ok(e) => e,
            Err(err) => {
                tracing::error!(error = %err, "report queue unreadable; backing off");
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            }
        };

        if entries.is_empty() {
            // Sleep until woken by a new enqueue, or fall back to a
            // periodic poll (paranoia against a missed/dropped wake).
            let _ = tokio::time::timeout(IDLE_POLL_INTERVAL, wake.recv()).await;
            continue;
        }

        let mut remaining = Vec::with_capacity(entries.len());
        let mut max_attempts_after = 0u32;
        for mut entry in entries {
            let result = match &entry.outcome {
                Outcome::Complete(v) => client.complete(entry.job_id, v.clone()).await,
                Outcome::Fail(msg) => client.fail(entry.job_id, msg).await,
            };
            match result {
                Ok(()) => {
                    tracing::info!(
                        job_id = entry.job_id,
                        attempts = entry.attempts,
                        "report queue: delivered"
                    );
                }
                Err(err) => {
                    entry.attempts = entry.attempts.saturating_add(1);
                    tracing::warn!(
                        job_id = entry.job_id,
                        attempts = entry.attempts,
                        error = %err,
                        "report queue: delivery failed, will retry independently of the main loop"
                    );
                    max_attempts_after = max_attempts_after.max(entry.attempts);
                    remaining.push(entry);
                }
            }
        }
        if let Err(err) = save(&path, &remaining) {
            tracing::error!(error = %err, "report queue: failed to persist after a delivery attempt");
        }

        if !remaining.is_empty() {
            let shift = max_attempts_after.min(5);
            let backoff = Duration::from_secs((1_u64 << shift).min(MAX_BACKOFF.as_secs()));
            tokio::time::sleep(backoff).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(base_url: &str) -> WorkerClient {
        let cfg = AgentConfig {
            worker_url: base_url.to_string(),
            node_id: "node-1".to_string(),
            agent_api_key: "test-key".to_string(),
            vpn_admin_binary: "/usr/local/bin/vpn-admin".to_string(),
            vpn_admin_config: "/etc/vpn/deployment.toml".to_string(),
            poll_interval_secs: 1,
            clash_api_url: None,
            clash_api_secret: None,
            clash_probe_outbound: None,
            lease_pool_size: 0,
            lease_slot_lifetime_secs: 1800,
            lease_state_file: "/tmp/unused-lease-state.json".to_string(),
            report_queue_file: "/tmp/unused-report-queue.json".to_string(),
            op_dedup_file: "/tmp/unused-op-dedup.json".to_string(),
            rotation_batch_interval_secs: 60,
            heartbeat_interval_secs: 60,
            protocol_probe: None,
        };
        WorkerClient::new(&cfg)
    }

    #[test]
    fn load_on_a_missing_file_is_an_empty_queue_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn save_then_load_round_trips_and_is_0600_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let entries = vec![Entry {
            job_id: 42,
            outcome: Outcome::Complete(serde_json::json!({"vpn_user_id": "u1"})),
            attempts: 2,
        }];
        save(&path, &entries).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].job_id, 42);
        assert_eq!(loaded[0].attempts, 2);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[tokio::test]
    async fn enqueue_persists_immediately_even_before_the_background_task_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let (wake_tx, _wake_rx) = mpsc::unbounded_channel();
        let queue = ReportQueue {
            path: path.clone(),
            wake: wake_tx,
        };
        queue
            .enqueue_complete(7, serde_json::json!({"ok": true}))
            .unwrap();
        let entries = load(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].job_id, 7);
    }

    /// Phase 8's core property: a completion report that fails repeatedly
    /// eventually gets delivered once the endpoint recovers, without the
    /// caller ever blocking on it (enqueue returns immediately regardless
    /// of the endpoint's state).
    #[tokio::test]
    async fn queued_completion_survives_repeated_500s_and_delivers_once_the_endpoint_recovers() {
        let server = MockServer::start().await;
        // First two attempts fail, third succeeds.
        Mock::given(method("POST"))
            .and(path_regex(r"^/api/agent/jobs/\d+/complete$"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/api/agent/jobs/\d+/complete$"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let client = client_for(&server.uri());
        let queue = ReportQueue::spawn(path.clone(), client);
        queue
            .enqueue_complete(99, serde_json::json!({"vpn_user_id": "u99"}))
            .unwrap();
        // enqueue() returned immediately above — that's the fix under
        // test. Now poll the on-disk queue until the background task has
        // drained it (bounded so a real regression fails the test instead
        // of hanging forever).
        for _ in 0..100 {
            if load(&path).unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("report queue never delivered the completion after the endpoint recovered");
    }

    #[tokio::test]
    async fn enqueue_never_blocks_regardless_of_endpoint_health() {
        // No mock mounted at all -> every delivery attempt fails with a
        // connection error. enqueue() must still return promptly.
        let server = MockServer::start().await;
        drop(server); // server dropped immediately: connections now fail.

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let client = client_for("http://127.0.0.1:1"); // nothing listens here
        let queue = ReportQueue::spawn(path, client);

        let started = std::time::Instant::now();
        queue.enqueue_complete(1, serde_json::json!({})).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "enqueue must return immediately, independent of the Worker's reachability"
        );
    }
}
