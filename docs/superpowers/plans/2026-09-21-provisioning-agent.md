# VPS Provisioning Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A small Rust binary (`apps/provisioning-agent`, new crate) that runs on the VPN VPS, polls the `vpn-web` Worker API (implemented in the sibling `2026-09-21-provisioning-worker-api.md` plan) over outbound HTTPS only, and drives the existing `vpn-admin` CLI to actually create/update/disable VPN accounts in response to Stripe-driven `provisioning_jobs`.

**Architecture:** A thin async dispatcher, not a reimplementation of `vpn-admin`'s logic — the agent shells out to the already-existing `vpn-admin` binary (`std::process::Command`, via `tokio::process::Command` since the agent's poll loop is async) for every job type, so `vpn-admin`'s own validate/apply/reload discipline stays the single place that logic lives. The agent's own code is: poll → dispatch → report.

**Tech Stack:** New crate `apps/provisioning-agent`, sibling to `apps/admin`. `tokio` (already a workspace dependency) for the async runtime, `reqwest` (new, rustls-backed, no native-tls) for the Worker API client, `serde`/`serde_json`/`toml` (all already workspace dependencies) for config and job payloads, `time` (new, lightweight) for RFC3339 ↔ unix-seconds conversion (job payloads from the webhook carry ISO8601 timestamps; `vpn-admin` takes unix seconds).

**Spec:** `docs/superpowers/specs/2026-09-21-vpn-provisioning-agent-design.md` — §6 (agent architecture, dispatch table), §7 (the `rotate-token --json` prerequisite this plan's Task 0 adds).

## Global Constraints

- **The agent never opens an inbound port and never receives a Supabase credential.** Its only two secrets are its own `agent_api_key` (the raw per-node key from `scripts/register-node.mjs` in the `vpn-web` repo) and whatever `vpn-admin` itself needs (nothing extra — the agent invokes it exactly as an operator would from the shell).
- **Every `vpn-admin` invocation goes through the existing binary as a subprocess**, never by linking `apps/admin`'s internals directly. This keeps `vpn-admin`'s locking/atomic-render/validate/reload discipline as the only place that logic lives, and means this plan cannot introduce a second, divergent code path for mutating `deployment.toml`/`users.json`.
- **A failed `vpn-admin` invocation is retried up to 3 times with a short fixed backoff (2s) before being reported as failed** — handles transient issues (disk hiccup, brief lock contention) without unbounded retry. After 3 failures, `POST .../fail` and move on to the next poll; a failed job is not retried automatically by the agent itself.
- **The agent's own crash or restart must not double-process a job.** Since `claim_next_job` on the Worker side already marks a job `claimed` atomically, and the agent only proceeds to `complete`/`fail` after a claim succeeds, a crash between claim and report simply leaves that job `claimed` forever (visible for manual investigation) rather than silently re-claiming and double-running it — the agent does not implement its own claimed-job recovery/timeout in this plan (flagged as a known limitation, not fixed here; consistent with this project's "soft, manual-review" posture elsewhere).
- **`rotate-token`'s new `--json` output never includes server private keys** — same explicit constraint already documented on `create --json`.
- Poll interval: 15 seconds, configurable.

---

## Task 0: Add `--json` output to `user rotate-token`

**Files:**
- Modify: `apps/admin/src/main.rs`

**Interfaces:**
- Produces: `vpn-admin user rotate-token <user_id> --json` → `{"id": string, "subscription_url": string}` on stdout, nothing else. The provisioning agent (Task 3) consumes this exact shape.

- [ ] **Step 1: Add the `--json` flag to the `RotateToken` clap variant**

In `apps/admin/src/main.rs`, find the `RotateToken` variant inside `enum UserCommands` (around line 376):

```rust
    RotateToken {
        user_id: String,
        /// Print a terminal QR code of the new subscription URL.
        #[arg(long)]
        qr: bool,
    },
```

Change it to:

```rust
    RotateToken {
        user_id: String,
        /// Print a terminal QR code of the new subscription URL.
        #[arg(long)]
        qr: bool,
        /// Print `{"id","subscription_url"}` as JSON instead of the
        /// human-readable form. Never includes server private keys —
        /// same constraint as `create --json`.
        #[arg(long)]
        json: bool,
    },
```

- [ ] **Step 2: Update the call site and `cmd_user_rotate_token`'s signature**

Find where `Commands::User(UserCommands::RotateToken { user_id, qr })` is matched (around line 559) and add `json` to the destructure and the call:

```rust
        Commands::User(UserCommands::RotateToken { user_id, qr, json }) => {
            cmd_user_rotate_token(&cfg, &user_id, qr, json)
        }
```

(Match whatever the existing call looks like exactly — read the surrounding lines first, since other arms in this `match` follow a consistent style to mirror.)

Update `cmd_user_rotate_token`'s signature (currently at line 2856):

```rust
fn cmd_user_rotate_token(cfg: &DeploymentConfig, id: &str, qr: bool, json: bool) -> Result<()> {
```

- [ ] **Step 3: Add the JSON output path**

Inside `cmd_user_rotate_token`, mirroring `cmd_user_create`'s pattern exactly (diverting human output to stderr when JSON is requested, so pipelines never see mixed output):

```rust
fn cmd_user_rotate_token(cfg: &DeploymentConfig, id: &str, qr: bool, json: bool) -> Result<()> {
    let machine_stdout = if json {
        Some(MachineStdout::divert_human_output_to_stderr()?)
    } else {
        None
    };
    let mut users = store::load_users(&cfg.users_file())?;
    let token = credentials::generate_subscription_token();
    let hash = credentials::hash_token(&token);
    find_user_mut(&mut users, id)?.subscription_token_hash_hex = hash;
    store::save_users_atomic(&cfg.users_file(), &users)?;
    // Token rotation does not change VLESS/Hysteria2 credentials, so the
    // sing-box config is unaffected — no re-render needed.
    let url = subscription_url(cfg, &token);

    if let Some(machine_stdout) = machine_stdout {
        let out = serde_json::json!({
            "id": id,
            "subscription_url": url,
        });
        return machine_stdout.write_document(&out);
    }

    if suppress_onboarding_secrets() {
        println!(
            "SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS is set — the new subscription URL and \
             QR code are suppressed below because they ARE the credential (this is an \
             automated/CI run, not an interactive session; re-run without this variable to \
             see the real value)."
        );
        println!("credential generated: yes");
        println!("subscription URL generated: yes");
        println!("QR generated: suppressed");
        return Ok(());
    }
    println!("New Hiddify subscription URL for {id}:");
    println!("  {url}");
    println!();
    println!("The previous subscription URL now 404s — it can no longer be used to FETCH or");
    println!("REFRESH this user's config. This does NOT change the VLESS UUID or Hysteria2");
    println!("password: an already-imported REALITY/Hysteria2 profile keeps connecting exactly");
    println!("as before, since those transport credentials are unchanged. Re-importing this new");
    println!("URL is only needed so the client can refresh in the future (some clients also drop");
    println!("saved servers on a failed refresh — re-import here avoids depending on that).");
    println!("Only `vpn-admin hysteria-obfs-rotate` changes the Hysteria2 obfuscation secret");
    println!("itself, which DOES require re-importing to keep the Hysteria2 profile working.");
    if qr {
        println!();
        println!("Scan this QR code in Hiddify (Add profile -> Scan QR code):");
        print_qr(&url)?;
    }
    Ok(())
}
```

- [ ] **Step 4: Build and exercise manually**

Run: `cargo build -p admin --bin vpn-admin` (matching this repo's Windows-dev-environment test-command substitutions already documented elsewhere in this repo's plans — use whatever exact local build command this environment's other admin-CLI plans have already established works here).

Then, against a local test deployment (reuse whatever `deployment.toml`/`users.json` fixture this repo's existing admin CLI tests already set up — check `apps/admin/tests/` for the established local-test-fixture pattern before creating a new one):

```bash
vpn-admin user create --name json-rotate-test --json
```

Note the printed `id`, then:

```bash
vpn-admin user rotate-token <id> --json
```

Expected: valid JSON on stdout, exactly `{"id": "<id>", "subscription_url": "https://..."}`, no extra keys, no human-readable text mixed in. Run without `--json` on the same user afterward and confirm the existing human-readable output is unchanged (same text as before this change, since Steps 1-3 only added a new branch, never modified the existing one).

- [ ] **Step 5: Run the existing admin CLI test suite**

Run: `cargo test -p admin` (or this repo's established scoped-test substitution for Windows, per this repo's own documented baseline test-command notes — do not use a bare `cargo test --workspace`, which this repo's other plans have already flagged as hitting unrelated pre-existing Windows failures).

Expected: passes, no regressions (this change is additive — a new flag on an existing command, no existing behavior altered).

- [ ] **Step 6: Commit**

```bash
git add apps/admin/src/main.rs
git commit -m "Add --json output to vpn-admin user rotate-token

Mirrors user create --json's pattern (MachineStdout diverts human
output to stderr so JSON stdout stays clean). Needed by the VPS
provisioning agent (see docs/superpowers/specs/2026-09-21-vpn-
provisioning-agent-design.md §6): ROTATE_SUBSCRIPTION_TOKEN jobs need
a machine-readable subscription_url, which rotate-token previously
had no way to produce. Never includes server private keys, same
constraint as create --json.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 1: Scaffold the `apps/provisioning-agent` crate

**Files:**
- Create: `apps/provisioning-agent/Cargo.toml`
- Create: `apps/provisioning-agent/src/config.rs`
- Create: `apps/provisioning-agent/src/main.rs`
- Modify: `Cargo.toml` (add the new crate to `[workspace] members`)

**Interfaces:**
- Produces: `AgentConfig` struct (`worker_url: String`, `node_id: String`, `agent_api_key: String`, `poll_interval_secs: u64`, `vpn_admin_binary: String`, `vpn_admin_config: String`), loaded from a TOML file via `AgentConfig::load(path: &Path) -> anyhow::Result<Self>`. Tasks 2-4 consume this.

- [ ] **Step 1: Add the crate to the workspace**

In the root `Cargo.toml`, add `"apps/provisioning-agent"` to the `members` list (after `"apps/admin"`):

```toml
members = [
    "crates/common",
    "crates/compat-config",
    "crates/provisioning-contract",
    "apps/admin",
    "apps/provisioning-agent",
    "services/subscription",
]
```

- [ ] **Step 2: Write `apps/provisioning-agent/Cargo.toml`**

```toml
[package]
name = "provisioning-agent"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[[bin]]
name = "vpn-provisioning-agent"
path = "src/main.rs"

[dependencies]
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
toml.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap.workspace = true
anyhow = "1"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
time = { version = "0.3", features = ["parsing", "formatting"] }

[dev-dependencies]
tempfile = "3"
```

`rustls-tls`, not the default `native-tls` feature, matches this project's general preference for not depending on the host's OpenSSL install (and keeps the VPS deployment self-contained — no system TLS library version to track).

- [ ] **Step 3: Write `apps/provisioning-agent/src/config.rs`**

```rust
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Agent configuration, loaded from a TOML file (default
/// `/etc/vpn/provisioning-agent.toml` on a real VPS, an arbitrary path in
/// tests). Deliberately separate from `vpn-admin`'s own
/// `deployment.toml` — this agent's concerns (which Worker to poll, with
/// which credential) are unrelated to VPN deployment topology, and
/// keeping them in separate files means neither can accidentally corrupt
/// the other.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Base URL of the vpn-web Worker API, e.g. "https://example.com" —
    /// no trailing slash.
    pub worker_url: String,
    /// This agent's node_id, must match a row in vpn-web's `nodes` table.
    pub node_id: String,
    /// The raw per-node API key `scripts/register-node.mjs` printed when
    /// this node was registered. Sent as `Authorization: Bearer
    /// <agent_api_key>` on every Worker API call.
    pub agent_api_key: String,
    /// How often to poll for a new job, in seconds.
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Path to the `vpn-admin` (or `vpn`) binary this agent shells out to.
    pub vpn_admin_binary: String,
    /// Path to the `deployment.toml` this agent's `vpn-admin` invocations
    /// should use — passed as `vpn-admin --config <this>`.
    pub vpn_admin_config: String,
}

fn default_poll_interval_secs() -> u64 {
    15
}

impl AgentConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading agent config from {path:?}"))?;
        let cfg: AgentConfig = toml::from_str(&text)
            .with_context(|| format!("parsing agent config from {path:?}"))?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_parses_a_minimal_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provisioning-agent.toml");
        std::fs::write(
            &path,
            r#"
worker_url = "http://127.0.0.1:8788"
node_id = "node-1"
agent_api_key = "test-key"
vpn_admin_binary = "/usr/local/bin/vpn-admin"
vpn_admin_config = "/etc/vpn/deployment.toml"
"#,
        )
        .unwrap();

        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.worker_url, "http://127.0.0.1:8788");
        assert_eq!(cfg.node_id, "node-1");
        assert_eq!(cfg.poll_interval_secs, 15, "default should apply when omitted");
    }

    #[test]
    fn load_respects_an_explicit_poll_interval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provisioning-agent.toml");
        std::fs::write(
            &path,
            r#"
worker_url = "http://127.0.0.1:8788"
node_id = "node-1"
agent_api_key = "test-key"
poll_interval_secs = 5
vpn_admin_binary = "/usr/local/bin/vpn-admin"
vpn_admin_config = "/etc/vpn/deployment.toml"
"#,
        )
        .unwrap();

        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.poll_interval_secs, 5);
    }

    #[test]
    fn load_fails_on_missing_file() {
        let result = AgentConfig::load(Path::new("/nonexistent/provisioning-agent.toml"));
        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Write a minimal `apps/provisioning-agent/src/main.rs`** (just enough to compile and load config; the poll loop itself is Task 4)

```rust
mod config;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "/etc/vpn/provisioning-agent.toml")]
    config: PathBuf,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cfg = AgentConfig::load(&cli.config)
        .with_context(|| format!("loading agent config from {:?}", cli.config))?;
    tracing::info!(node_id = %cfg.node_id, worker_url = %cfg.worker_url, "provisioning agent starting");
    Ok(())
}
```

- [ ] **Step 5: Build and test**

Run: `cargo build -p provisioning-agent` then `cargo test -p provisioning-agent`.

Expected: builds clean, all 3 config tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml apps/provisioning-agent/Cargo.toml apps/provisioning-agent/src/config.rs apps/provisioning-agent/src/main.rs
git commit -m "Scaffold apps/provisioning-agent crate with config loading

New binary vpn-provisioning-agent, sibling to vpn-admin. Config is a
separate TOML file from deployment.toml (unrelated concerns — which
Worker to poll vs VPN deployment topology). rustls-tls, not
native-tls, to avoid depending on the host's OpenSSL install.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: Worker API client

**Files:**
- Create: `apps/provisioning-agent/src/worker_client.rs`
- Modify: `apps/provisioning-agent/src/main.rs` (add `mod worker_client;`)

**Interfaces:**
- Consumes: `AgentConfig` (Task 1).
- Produces: `WorkerClient::new(cfg: &AgentConfig) -> Self`, `async fn claim(&self) -> Result<Option<Job>>`, `async fn complete(&self, job_id: i64, result: serde_json::Value) -> Result<()>`, `async fn fail(&self, job_id: i64, error: &str) -> Result<()>`, and `struct Job { id: i64, job_type: String, payload: serde_json::Value }`. Task 4 (poll loop) consumes all of these.

- [ ] **Step 1: Write `apps/provisioning-agent/src/worker_client.rs`**

```rust
use crate::config::AgentConfig;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

/// A job claimed from the Worker's /api/agent/claim endpoint. Field names
/// match that endpoint's response body exactly (see the vpn-web
/// provisioning-worker-api plan, Task 3) — job_type is one of
/// "CREATE_USER" | "SET_EXPIRY" | "ENABLE_USER" | "DISABLE_USER" |
/// "ROTATE_SUBSCRIPTION_TOKEN".
#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: i64,
    pub job_type: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
struct ClaimResponse {
    job: Option<Job>,
}

pub struct WorkerClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl WorkerClient {
    pub fn new(cfg: &AgentConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: cfg.worker_url.clone(),
            api_key: cfg.agent_api_key.clone(),
        }
    }

    /// Claims the oldest pending job for this agent's node, or None if
    /// nothing is pending right now.
    pub async fn claim(&self) -> Result<Option<Job>> {
        let res = self
            .http
            .post(format!("{}/api/agent/claim", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("POST /api/agent/claim request failed")?;

        if !res.status().is_success() {
            bail!("POST /api/agent/claim returned {}", res.status());
        }

        let parsed: ClaimResponse = res
            .json()
            .await
            .context("parsing /api/agent/claim response body")?;
        Ok(parsed.job)
    }

    /// Reports a job as successfully completed. `result` is whatever
    /// job-type-specific payload the Worker's /complete endpoint expects
    /// (see the vpn-web plan's Task 4) — e.g. for CREATE_USER,
    /// `{"vpn_user_id": ..., "subscription_url": ...}`.
    pub async fn complete(&self, job_id: i64, result: Value) -> Result<()> {
        let res = self
            .http
            .post(format!("{}/api/agent/jobs/{job_id}/complete", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "result": result }))
            .send()
            .await
            .context("POST /api/agent/jobs/:id/complete request failed")?;

        if !res.status().is_success() {
            bail!(
                "POST /api/agent/jobs/{job_id}/complete returned {}",
                res.status()
            );
        }
        Ok(())
    }

    /// Reports a job as failed. The Worker marks it `failed` and sends
    /// the operator alert — this agent does not retry beyond its own
    /// local 3-attempt backoff (see the poll loop in main.rs).
    pub async fn fail(&self, job_id: i64, error: &str) -> Result<()> {
        let res = self
            .http
            .post(format!("{}/api/agent/jobs/{job_id}/fail", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "error": error }))
            .send()
            .await
            .context("POST /api/agent/jobs/:id/fail request failed")?;

        if !res.status().is_success() {
            bail!(
                "POST /api/agent/jobs/{job_id}/fail returned {}",
                res.status()
            );
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Add `mod worker_client;` to `main.rs`**

Add near the top of `apps/provisioning-agent/src/main.rs`, alongside `mod config;`:

```rust
mod config;
mod worker_client;
```

- [ ] **Step 3: Build**

Run: `cargo build -p provisioning-agent`. Expected: builds clean (this task has no unit tests of its own — `WorkerClient` needs a real or mocked HTTP endpoint, which Task 5's integration test against the real local Worker API covers; a mocked-HTTP unit test here would just re-verify `reqwest`'s own request-building, not this crate's logic).

- [ ] **Step 4: Commit**

```bash
git add apps/provisioning-agent/src/worker_client.rs apps/provisioning-agent/src/main.rs
git commit -m "Add Worker API client (claim/complete/fail)

Thin reqwest wrapper over the three provisioning endpoints from the
vpn-web provisioning-worker-api plan. Bearer-authenticates with this
node's agent_api_key on every call — the agent's only credential.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: `vpn-admin` dispatcher

**Files:**
- Create: `apps/provisioning-agent/src/dispatch.rs`
- Modify: `apps/provisioning-agent/src/main.rs` (add `mod dispatch;`)

**Interfaces:**
- Consumes: `Job` (Task 2), `AgentConfig` (Task 1).
- Produces: `async fn run_job(cfg: &AgentConfig, job: &Job) -> Result<serde_json::Value>` — returns the `result` value to report via `WorkerClient::complete`, or an `Err` (with the failure's display text becoming the `/fail` error message) after this function's own internal 3-attempt retry is exhausted. Task 4 (poll loop) consumes this.

- [ ] **Step 1: Write `apps/provisioning-agent/src/dispatch.rs`**

```rust
use crate::config::AgentConfig;
use crate::worker_client::Job;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::process::Command;

const MAX_ATTEMPTS: u32 = 3;
const RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// Runs the vpn-admin subcommand for one job, retrying transient failures
/// up to MAX_ATTEMPTS times before giving up. Returns the result payload
/// to report to the Worker's /complete endpoint.
pub async fn run_job(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let mut last_err = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match run_job_once(cfg, job).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                tracing::warn!(job_id = job.id, attempt, error = %err, "vpn-admin invocation failed");
                last_err = Some(err);
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(RETRY_BACKOFF).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("job failed with no recorded error")))
}

async fn run_job_once(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    match job.job_type.as_str() {
        "CREATE_USER" => create_user(cfg, job).await,
        "SET_EXPIRY" => set_expiry(cfg, job).await,
        "ENABLE_USER" => enable_or_disable(cfg, job, "enable").await,
        "DISABLE_USER" => enable_or_disable(cfg, job, "disable").await,
        "ROTATE_SUBSCRIPTION_TOKEN" => rotate_token(cfg, job).await,
        other => bail!("unknown job_type {other:?} (job {})", job.id),
    }
}

/// Every vpn-admin invocation goes through this one helper, so the
/// binary path / config path / working-directory setup happens in
/// exactly one place.
fn vpn_admin_command(cfg: &AgentConfig) -> Command {
    let mut cmd = Command::new(&cfg.vpn_admin_binary);
    cmd.arg("--config").arg(&cfg.vpn_admin_config);
    cmd
}

fn payload_str<'a>(job: &'a Job, key: &str) -> Result<&'a str> {
    job.payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("job {} payload missing string field {key:?}", job.id))
}

/// Job payloads carry RFC3339 timestamps (e.g. "2027-01-01T00:00:00Z",
/// as written by the Stripe webhook's `new Date(...).toISOString()`);
/// vpn-admin's --expires-at takes unix seconds.
fn payload_expires_at_unix(job: &Job) -> Result<i64> {
    let raw = payload_str(job, "expires_at")?;
    let parsed = OffsetDateTime::parse(raw, &Rfc3339)
        .with_context(|| format!("parsing expires_at {raw:?} as RFC3339 (job {})", job.id))?;
    Ok(parsed.unix_timestamp())
}

async fn create_user(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let user_id = payload_str(job, "user_id")?;
    let expires_at = payload_expires_at_unix(job)?;

    let output = vpn_admin_command(cfg)
        .args([
            "user",
            "create",
            "--name",
            user_id,
            "--expires-at",
            &expires_at.to_string(),
            "--json",
        ])
        .output()
        .await
        .context("spawning vpn-admin user create")?;
    let parsed = parse_json_output(&output, "user create")?;

    let vpn_user_id = parsed
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing id"))?;
    let subscription_url = parsed
        .get("subscription_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing subscription_url"))?;

    Ok(serde_json::json!({
        "vpn_user_id": vpn_user_id,
        "subscription_url": subscription_url,
    }))
}

async fn set_expiry(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;
    let expires_at = payload_expires_at_unix(job)?;

    let output = vpn_admin_command(cfg)
        .args([
            "user",
            "set-expiry",
            vpn_user_id,
            "--expires-at",
            &expires_at.to_string(),
        ])
        .output()
        .await
        .context("spawning vpn-admin user set-expiry")?;
    require_success(&output, "user set-expiry")?;
    Ok(serde_json::json!({}))
}

async fn enable_or_disable(cfg: &AgentConfig, job: &Job, subcommand: &str) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let output = vpn_admin_command(cfg)
        .args(["user", subcommand, vpn_user_id])
        .output()
        .await
        .with_context(|| format!("spawning vpn-admin user {subcommand}"))?;
    require_success(&output, &format!("user {subcommand}"))?;
    Ok(serde_json::json!({}))
}

async fn rotate_token(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let output = vpn_admin_command(cfg)
        .args(["user", "rotate-token", vpn_user_id, "--json"])
        .output()
        .await
        .context("spawning vpn-admin user rotate-token")?;
    let parsed = parse_json_output(&output, "user rotate-token")?;

    let subscription_url = parsed
        .get("subscription_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user rotate-token --json output missing subscription_url"))?;

    Ok(serde_json::json!({ "subscription_url": subscription_url }))
}

fn require_success(output: &std::process::Output, what: &str) -> Result<()> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("vpn-admin {what} exited with {}: {stderr}", output.status);
    }
    Ok(())
}

fn parse_json_output(output: &std::process::Output, what: &str) -> Result<Value> {
    require_success(output, what)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .with_context(|| format!("parsing vpn-admin {what} --json output: {stdout:?}"))
}
```

- [ ] **Step 2: Add `mod dispatch;` to `main.rs`**

```rust
mod config;
mod dispatch;
mod worker_client;
```

- [ ] **Step 3: Write unit tests for the pure logic (payload parsing), using a fake job — not a real vpn-admin invocation**

Add to the bottom of `apps/provisioning-agent/src/dispatch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker_client::Job;

    fn job(job_type: &str, payload: Value) -> Job {
        Job {
            id: 1,
            job_type: job_type.to_string(),
            payload,
        }
    }

    #[test]
    fn payload_str_reads_a_present_string_field() {
        let j = job("SET_EXPIRY", serde_json::json!({"vpn_user_id": "abc123"}));
        assert_eq!(payload_str(&j, "vpn_user_id").unwrap(), "abc123");
    }

    #[test]
    fn payload_str_errors_on_missing_field() {
        let j = job("SET_EXPIRY", serde_json::json!({}));
        assert!(payload_str(&j, "vpn_user_id").is_err());
    }

    #[test]
    fn payload_expires_at_unix_parses_rfc3339() {
        let j = job(
            "SET_EXPIRY",
            serde_json::json!({"expires_at": "2027-01-01T00:00:00Z"}),
        );
        // 2027-01-01T00:00:00Z is a fixed, known unix timestamp.
        assert_eq!(payload_expires_at_unix(&j).unwrap(), 1_798_761_600);
    }

    #[test]
    fn payload_expires_at_unix_errors_on_malformed_timestamp() {
        let j = job("SET_EXPIRY", serde_json::json!({"expires_at": "not-a-date"}));
        assert!(payload_expires_at_unix(&j).is_err());
    }

    #[test]
    fn parse_json_output_errors_on_nonzero_exit() {
        let output = std::process::Output {
            status: std::process::ExitStatusExt::from_raw(1i32.wrapping_shl(8)),
            stdout: vec![],
            stderr: b"boom".to_vec(),
        };
        // ExitStatusExt is unix-only; this test module compiles only
        // where that's available (matches this repo's existing pattern
        // of #[cfg(unix)] for exit-status-construction tests elsewhere).
        assert!(parse_json_output(&output, "test").is_err());
    }
}
```

Note: `std::process::ExitStatusExt` is unix-only (from `std::os::unix::process::ExitStatusExt`) — add the correct import (`use std::os::unix::process::ExitStatusExt;`) at the top of the test module, and gate that one test with `#[cfg(unix)]` since this repo's CI/dev environment is Windows for parts of the codebase (consistent with this repo's already-documented pattern of `#[cfg(unix)]`-gating OS-specific tests, e.g. in `apps/admin`). The other four tests are OS-independent and need no gate.

- [ ] **Step 4: Run tests**

Run: `cargo test -p provisioning-agent`.

Expected: all tests pass (the `#[cfg(unix)]`-gated one only runs on Windows if cross-compiled/WSL-tested — on a plain Windows `cargo test` run it's simply skipped, which is expected and not a failure, matching this repo's other documented Windows-environment test caveats).

- [ ] **Step 5: Commit**

```bash
git add apps/provisioning-agent/src/dispatch.rs apps/provisioning-agent/src/main.rs
git commit -m "Add vpn-admin dispatcher with per-job-type subcommand mapping

Every job_type maps to exactly one vpn-admin subcommand invocation via
tokio::process::Command — CREATE_USER/ROTATE_SUBSCRIPTION_TOKEN parse
--json output for the subscription_url the Worker needs to encrypt;
SET_EXPIRY/ENABLE_USER/DISABLE_USER only check the exit code. Up to 3
attempts with a 2s backoff per job before giving up. Unit-tested the
pure payload-parsing logic (RFC3339 timestamp conversion, missing-
field errors) without invoking a real vpn-admin process.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: Poll loop, systemd unit, and end-to-end local verification

**Files:**
- Modify: `apps/provisioning-agent/src/main.rs`
- Create: `apps/provisioning-agent/vpn-provisioning-agent.service` (systemd unit, reference/documentation — not installed by this plan, since no real VPS exists yet)

**Interfaces:**
- Consumes: `WorkerClient` (Task 2), `run_job` (Task 3), `AgentConfig` (Task 1).
- Produces: the assembled `vpn-provisioning-agent` binary's actual runtime behavior — nothing further consumes this within the codebase; it's the end of the chain.

- [ ] **Step 1: Replace `apps/provisioning-agent/src/main.rs` with the full poll loop**

```rust
mod config;
mod dispatch;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use std::path::PathBuf;
use std::time::Duration;
use worker_client::WorkerClient;

#[derive(Parser)]
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
    let poll_interval = Duration::from_secs(cfg.poll_interval_secs);

    loop {
        if let Err(err) = poll_once(&cfg, &client).await {
            // A poll-loop-level error (Worker unreachable, auth failure,
            // etc) is logged and the loop continues — this agent has no
            // "give up" state, since the alternative (crashing) just
            // means systemd restarts it into the same situation. Job-
            // level failures are handled inside poll_once itself, via
            // client.fail(...), and never reach this branch.
            tracing::error!(error = %err, "poll iteration failed");
        }
        tokio::time::sleep(poll_interval).await;
    }
}

async fn poll_once(cfg: &AgentConfig, client: &WorkerClient) -> Result<()> {
    let Some(job) = client.claim().await.context("claiming a job")? else {
        return Ok(());
    };
    tracing::info!(job_id = job.id, job_type = %job.job_type, "claimed job");

    match dispatch::run_job(cfg, &job).await {
        Ok(result) => {
            client
                .complete(job.id, result)
                .await
                .context("reporting job completion")?;
            tracing::info!(job_id = job.id, "job completed");
        }
        Err(err) => {
            let message = err.to_string();
            tracing::error!(job_id = job.id, error = %message, "job failed after retries");
            client
                .fail(job.id, &message)
                .await
                .context("reporting job failure")?;
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Write the reference systemd unit**

Create `apps/provisioning-agent/vpn-provisioning-agent.service`:

```ini
[Unit]
Description=Arcana VPN provisioning agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/vpn-provisioning-agent --config /etc/vpn/provisioning-agent.toml
Restart=on-failure
RestartSec=5
# No inbound network access needed — outbound HTTPS to the Worker only.
# Runs as a dedicated non-root user with access to vpn-admin's
# deployment.toml/users.json, same as any other vpn-admin operator
# invocation on this box — this file documents the expected shape but
# does not attempt to fully re-derive this repo's existing deployment
# user/permissions setup, which is out of scope for this plan.
User=vpn-admin-agent

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 3: Build**

Run: `cargo build -p provisioning-agent`. Expected: builds clean.

- [ ] **Step 4: End-to-end local verification against the real local Worker API**

Requires: the `vpn-web` provisioning-worker-api plan already implemented and its local stack running (`npx supabase start`, `npm run build && npx wrangler pages dev out --port 8788`, `.dev.vars` populated, a node registered via that plan's `scripts/register-node.mjs node-1` with the printed raw key). Also requires a local `vpn-admin` test deployment (reuse whatever fixture `apps/admin`'s own tests use — check `apps/admin/tests/` for the established pattern — with a real `deployment.toml`/`users.json` this agent can safely mutate).

Write a local (not committed to git — this is an ad hoc config file for manual testing, standard practice this project already uses for scratch verification) `provisioning-agent.toml`:

```toml
worker_url = "http://127.0.0.1:8788"
node_id = "node-1"
agent_api_key = "<paste the raw key scripts/register-node.mjs printed>"
poll_interval_secs = 3
vpn_admin_binary = "<path to the locally built vpn-admin binary, e.g. target/debug/vpn-admin>"
vpn_admin_config = "<path to a local test deployment.toml>"
```

Run the agent in one terminal: `cargo run -p provisioning-agent -- --config /path/to/provisioning-agent.toml` (or the built binary directly).

In another terminal, insert a synthetic `CREATE_USER` job directly via `psql` (same local Supabase instance the Worker is using):

```sql
insert into public.provisioning_jobs (idempotency_key, node_id, job_type, payload)
values ('e2e-create-1', 'node-1', 'CREATE_USER', '{"user_id":"e2e-test-user","expires_at":"2027-01-01T00:00:00Z"}');
```

Watch the agent's log output. Expected within one poll interval: "claimed job" then "job completed" for job 1. Verify via `psql`:
- `provisioning_jobs.status = 'done'` for that row, `result` contains `vpn_user_id` and `subscription_url`.
- A `vpn_accounts` row exists for `user_id = 'e2e-test-user'`.
- A `vpn_secrets` row exists for that account, `ciphertext`/`nonce` non-null.
- Directly against the local `vpn-admin` test deployment: `vpn-admin --config <path> user list` shows a user named `e2e-test-user`, `enabled = true`.

Then insert a `SET_EXPIRY` job referencing that same account (using the `vpn_user_id` the previous step's `vpn_accounts` row now has):

```sql
insert into public.provisioning_jobs (idempotency_key, node_id, job_type, vpn_account_id, payload)
select 'e2e-set-expiry-1', 'node-1', 'SET_EXPIRY', id, jsonb_build_object('vpn_user_id', vpn_user_id, 'expires_at', '2027-06-01T00:00:00Z')
from public.vpn_accounts where user_id = 'e2e-test-user';
```

Expected: agent claims and completes it within one poll interval; `vpn-admin --config <path> user list --json` (or equivalent inspection) shows that user's expiry updated to the new timestamp.

Then insert a job with a deliberately broken payload to exercise the failure path:

```sql
insert into public.provisioning_jobs (idempotency_key, node_id, job_type, vpn_account_id, payload)
select 'e2e-fail-1', 'node-1', 'DISABLE_USER', id, '{}'::jsonb
from public.vpn_accounts where user_id = 'e2e-test-user';
```

(Empty payload — no `vpn_user_id` — deliberately triggers `payload_str`'s error path.) Expected: agent logs "job failed after retries" after 3 attempts (~6s of backoff, per the `RETRY_BACKOFF`/`MAX_ATTEMPTS` constants), then reports `/fail`; `psql` confirms `provisioning_jobs.status = 'failed'` with the payload-missing error message in `result`, and the Worker's terminal output (or Resend, if a real key is configured) shows the failure-alert attempt.

Report the actual agent log output and SQL query results for all three cases — not a paraphrase. Stop the agent, `wrangler pages dev`, and `npx supabase stop` when finished.

- [ ] **Step 5: Commit**

```bash
git add apps/provisioning-agent/src/main.rs apps/provisioning-agent/vpn-provisioning-agent.service
git commit -m "Assemble the poll loop and add a reference systemd unit

15s poll interval (configurable), claim -> dispatch -> complete/fail
on every iteration; a poll-loop-level error (Worker unreachable) logs
and continues rather than crashing, since systemd restarting into the
same failure gains nothing. Verified end to end against the real
local Worker API and a local vpn-admin test deployment: CREATE_USER,
SET_EXPIRY, and a deliberate failure path (missing payload field,
retried 3 times, then reported failed) all behave as designed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Explicitly not in this plan

- Deploying this binary + systemd unit to a real VPS, or running `scripts/register-node.mjs` against a production `nodes` table — operator actions once a real VPS exists (spec §9 prerequisite), not part of this plan's testing.
- Claimed-job recovery/timeout if the agent crashes mid-job — flagged as a known limitation in Global Constraints, not fixed here.
- §7 misuse-detection sampling — confirmed out of scope during brainstorming.
- Dashboard UI changes to call `GET /api/vpn/config` — separate small follow-up, depends on both this plan and the `vpn-web` provisioning-worker-api plan being done.
