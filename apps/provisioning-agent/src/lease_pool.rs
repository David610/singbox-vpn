//! ADR-0003 ephemeral managed authorization: the node-side lease pool.
//!
//! The node keeps a bounded pool of pseudonymous credential slots (VLESS
//! uuid + Hysteria2 password), persisted locally (0600) so it survives
//! agent restarts. Each slot *generation* has a hard end, `valid_until`,
//! chosen here and enforced here:
//!
//! * `vpn-admin lease-pool sync` renders each slot user with
//!   `expires_at = valid_until`, so any render after that instant drops it;
//! * this sweeper rotates (new secret, new generation) every slot that has
//!   expired, that the control plane reports `revoked`, or that can no
//!   longer be leased and is confirmed never leased — whether or not the
//!   control plane is reachable.
//!
//! A new generation is reported to the control plane only after vpn-admin
//! applied it to the live sing-box (fail-closed apply: lock, render,
//! `sing-box check`, atomic rename, reload, verify), so any credential the
//! control plane can lease already works. Secrets never reach a log line.

use crate::config::AgentConfig;
use crate::dispatch::{parse_json_output, vpn_admin_command};
use crate::worker_client::WorkerClient;
use anyhow::{anyhow, Context, Result};
use compat_config::credentials;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const MAX_POOL_SIZE: usize = 1024;
/// valid_until is rounded up to this grid so slots minted together expire
/// together: one apply (one sing-box reload) per batch, not per slot.
pub const EPOCH_SECS: i64 = 60;
/// Used until the control plane tells us its own value.
pub const DEFAULT_MIN_REMAINING_SECS: i64 = 600;
const VPN_ADMIN_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Slot {
    pub slot: u32,
    pub generation: u64,
    pub valid_until: i64,
    pub vless_uuid: String,
    pub hysteria2_password: String,
    /// The control plane has stored this generation's secret.
    #[serde(default)]
    pub reported: bool,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("slot", &self.slot)
            .field("generation", &self.generation)
            .field("valid_until", &self.valid_until)
            .field("reported", &self.reported)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct LeaseState {
    pub slots: Vec<Slot>,
    /// The current `slots` are what the live sing-box config holds.
    #[serde(default)]
    pub applied: bool,
}

/// What the control plane last said about each slot.
#[derive(Clone, Debug, Default)]
pub struct RemoteView {
    /// Server time of the snapshot (unix seconds).
    pub as_of: i64,
    pub min_remaining: i64,
    /// slot -> (generation, state)
    pub slots: HashMap<u32, (u64, String)>,
}

pub fn round_up_to_epoch(t: i64) -> i64 {
    (t + EPOCH_SECS - 1).div_euclid(EPOCH_SECS) * EPOCH_SECS
}

pub fn clamp_lifetime(secs: u64) -> i64 {
    secs.clamp(900, 7200) as i64
}

fn mint(slot: u32, generation: u64, now: i64, lifetime: i64) -> Slot {
    Slot {
        slot,
        generation,
        valid_until: round_up_to_epoch(now + lifetime),
        vless_uuid: credentials::generate_uuid_v4(),
        hysteria2_password: credentials::generate_hysteria2_password(),
        reported: false,
    }
}

/// Pure decision: which slots must get a fresh generation now.
pub fn slots_to_rotate(state: &LeaseState, remote: Option<&RemoteView>, now: i64) -> Vec<u32> {
    let min_remaining = remote
        .map(|r| r.min_remaining)
        .unwrap_or(DEFAULT_MIN_REMAINING_SECS);
    state
        .slots
        .iter()
        .filter(|s| {
            if s.valid_until <= now {
                return true; // expired: the hard bound, no control plane needed
            }
            let leasable_until = s.valid_until - min_remaining;
            if !s.reported && now >= leasable_until {
                return true; // never leasable anywhere, nobody holds it
            }
            let Some(r) = remote else { return false };
            match r.slots.get(&s.slot) {
                Some((generation, st)) if *generation == s.generation => {
                    st == "revoked" || (st == "active" && r.as_of >= leasable_until)
                }
                _ => false,
            }
        })
        .map(|s| s.slot)
        .collect()
}

/// Pure: resize to `size` and rotate `rotate`. Returns whether anything changed.
pub fn plan(state: &mut LeaseState, size: usize, rotate: &[u32], now: i64, lifetime: i64) -> bool {
    let size = size.min(MAX_POOL_SIZE);
    let mut changed = false;
    let before = state.slots.len();
    state.slots.retain(|s| (s.slot as usize) < size);
    changed |= state.slots.len() != before;
    for slot in 0..size as u32 {
        match state.slots.iter_mut().find(|s| s.slot == slot) {
            Some(existing) if rotate.contains(&slot) => {
                *existing = mint(slot, existing.generation + 1, now, lifetime);
                changed = true;
            }
            Some(_) => {}
            None => {
                state.slots.push(mint(slot, 1, now, lifetime));
                changed = true;
            }
        }
    }
    state.slots.sort_by_key(|s| s.slot);
    if changed {
        state.applied = false;
    }
    changed
}

pub fn load_state(path: &Path) -> Result<LeaseState> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing lease state {path:?} (contents not shown)")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LeaseState::default()),
        Err(e) => Err(e).with_context(|| format!("reading lease state {path:?}")),
    }
}

/// Atomic, 0600, fsynced: write temp in the same dir, rename over.
pub fn save_state(path: &Path, state: &LeaseState) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("lease state path has no parent"))?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut tmp = tempfile::NamedTempFile::new_in(dir).context("creating lease state temp file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("restricting lease state temp file")?;
    }
    use std::io::Write;
    tmp.write_all(&serde_json::to_vec(state)?)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| anyhow!("installing lease state: {}", e.error))?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn vpn_admin_input(state: &LeaseState) -> Value {
    json!({
        "slots": state.slots.iter().map(|s| json!({
            "slot": s.slot,
            "vless_uuid": s.vless_uuid,
            "hysteria2_password": s.hysteria2_password,
            "expires_at": s.valid_until,
        })).collect::<Vec<_>>()
    })
}

fn rfc3339(t: i64) -> Result<String> {
    Ok(OffsetDateTime::from_unix_timestamp(t)?.format(&Rfc3339)?)
}

/// Body for `/api/agent/leases/sync`: every slot, secrets only for
/// generations the control plane has not stored yet.
pub fn sync_body(state: &LeaseState, obfs_password: Option<&str>) -> Result<Value> {
    let mut slots = Vec::with_capacity(state.slots.len());
    for s in &state.slots {
        let mut item = json!({
            "slot": s.slot,
            "generation": s.generation,
            "valid_until": rfc3339(s.valid_until)?,
        });
        if !s.reported {
            item["vless_uuid"] = json!(s.vless_uuid);
            item["hysteria2_password"] = json!(s.hysteria2_password);
        }
        slots.push(item);
    }
    let mut body = json!({ "slots": slots });
    if let Some(p) = obfs_password {
        body["hysteria2_obfs_password"] = json!(p);
    }
    Ok(body)
}

/// Applies the sync response to local state. Returns the remote view.
pub fn absorb_sync_response(state: &mut LeaseState, resp: &Value) -> Result<RemoteView> {
    let as_of = resp
        .get("as_of")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("sync response missing as_of"))?;
    let as_of = OffsetDateTime::parse(as_of, &Rfc3339)
        .context("parsing sync as_of")?
        .unix_timestamp();
    let min_remaining = resp
        .get("min_remaining_seconds")
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_MIN_REMAINING_SECS);
    let mut view = RemoteView {
        as_of,
        min_remaining,
        slots: HashMap::new(),
    };
    for item in resp
        .get("slots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(slot), Some(generation), Some(st)) = (
            item.get("slot").and_then(Value::as_u64),
            item.get("generation").and_then(Value::as_u64),
            item.get("state").and_then(Value::as_str),
        ) else {
            continue;
        };
        view.slots.insert(slot as u32, (generation, st.to_string()));
    }
    for s in &mut state.slots {
        s.reported = matches!(view.slots.get(&s.slot), Some((g, _)) if *g == s.generation);
    }
    Ok(view)
}

pub struct LeasePool {
    path: PathBuf,
    state: LeaseState,
    remote: Option<RemoteView>,
    obfs_password: Option<String>,
    obfs_reported: bool,
    /// Force one apply per process start so the live config is re-proven
    /// (and the obfs password re-read) after a restart.
    applied_this_process: bool,
}

impl LeasePool {
    pub fn open(cfg: &AgentConfig) -> Result<Self> {
        let path = PathBuf::from(&cfg.lease_state_file);
        let state = load_state(&path)?;
        Ok(Self {
            path,
            state,
            remote: None,
            obfs_password: None,
            obfs_reported: false,
            applied_this_process: false,
        })
    }

    pub async fn tick(&mut self, cfg: &AgentConfig, client: &WorkerClient) -> Result<()> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let lifetime = clamp_lifetime(cfg.lease_slot_lifetime_secs);
        let rotate = slots_to_rotate(&self.state, self.remote.as_ref(), now);
        if plan(&mut self.state, cfg.lease_pool_size, &rotate, now, lifetime) {
            // Persist BEFORE applying: a crash mid-apply re-applies this
            // exact state on restart instead of forgetting a live secret.
            save_state(&self.path, &self.state)?;
            if !rotate.is_empty() {
                tracing::info!(rotated = rotate.len(), "lease pool: rotating slots");
            }
        }
        if !self.state.applied || !self.applied_this_process {
            self.apply(cfg).await?;
        }
        if cfg.lease_pool_size == 0 && self.state.slots.is_empty() {
            return Ok(());
        }
        let obfs = (!self.obfs_reported)
            .then_some(self.obfs_password.as_deref())
            .flatten();
        let body = sync_body(&self.state, obfs)?;
        let resp = client.sync_leases(&body).await?;
        let before = self.state.clone();
        let view = absorb_sync_response(&mut self.state, &resp)?;
        if resp.get("obfs_stored").and_then(Value::as_bool) == Some(true)
            || self.obfs_password.is_none()
        {
            self.obfs_reported = true;
        }
        self.remote = Some(view);
        if self.state != before {
            save_state(&self.path, &self.state)?;
        }
        Ok(())
    }

    async fn apply(&mut self, cfg: &AgentConfig) -> Result<()> {
        let dir = tempfile::Builder::new()
            .prefix("vpn-lease-pool-")
            .tempdir()
            .context("creating lease-pool temp dir")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let input = dir.path().join("lease-pool.json");
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            let mut f = opts.open(&input).context("creating lease-pool input")?;
            f.write_all(&serde_json::to_vec(&vpn_admin_input(&self.state))?)?;
        }
        let output = tokio::time::timeout(
            VPN_ADMIN_TIMEOUT,
            vpn_admin_command(cfg)
                .args(["lease-pool", "sync", "--input"])
                .arg(&input)
                .output(),
        )
        .await
        .context("vpn-admin lease-pool sync timed out after 60s")?
        .context("spawning vpn-admin lease-pool sync")?;
        let parsed = parse_json_output(&output, "lease-pool sync")?;
        if parsed.get("live").and_then(Value::as_bool) != Some(true) {
            // Fail closed: never report a generation the running sing-box
            // has not provably loaded.
            return Err(anyhow!(
                "vpn-admin lease-pool sync did not reach a live, reloaded sing-box"
            ));
        }
        let obfs = parsed
            .get("hysteria2_obfs_password")
            .and_then(Value::as_str)
            .map(str::to_string);
        if obfs != self.obfs_password {
            self.obfs_reported = false;
        }
        self.obfs_password = obfs;
        self.state.applied = true;
        self.applied_this_process = true;
        save_state(&self.path, &self.state)?;
        tracing::info!(slots = self.state.slots.len(), "lease pool applied live");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIFE: i64 = 1800;

    fn fresh(size: usize, now: i64) -> LeaseState {
        let mut st = LeaseState::default();
        plan(&mut st, size, &[], now, LIFE);
        st
    }

    fn view(as_of: i64, entries: &[(u32, u64, &str)]) -> RemoteView {
        RemoteView {
            as_of,
            min_remaining: 600,
            slots: entries
                .iter()
                .map(|(s, g, st)| (*s, (*g, st.to_string())))
                .collect(),
        }
    }

    #[test]
    fn plan_mints_a_bounded_pool_of_distinct_random_slots() {
        let st = fresh(32, 1_000);
        assert_eq!(st.slots.len(), 32);
        assert!(!st.applied);
        let uuids: std::collections::HashSet<_> = st.slots.iter().map(|s| &s.vless_uuid).collect();
        assert_eq!(uuids.len(), 32);
        assert!(st.slots.iter().all(|s| s.valid_until % EPOCH_SECS == 0));
        assert!(st.slots.iter().all(|s| s.valid_until >= 1_000 + LIFE));
        let capped = fresh(5000, 1_000);
        assert_eq!(capped.slots.len(), MAX_POOL_SIZE);
    }

    #[test]
    fn expired_slots_rotate_without_any_control_plane() {
        let st = fresh(2, 0);
        let end = st.slots[0].valid_until;
        let mut reported = st.clone();
        reported.slots.iter_mut().for_each(|s| s.reported = true);
        assert!(slots_to_rotate(&reported, None, end - 1).is_empty());
        assert_eq!(slots_to_rotate(&reported, None, end), vec![0, 1]);
    }

    #[test]
    fn revoked_slots_rotate_immediately_but_leased_ones_wait_for_expiry() {
        let mut st = fresh(3, 0);
        st.slots.iter_mut().for_each(|s| s.reported = true);
        let v = view(10, &[(0, 1, "revoked"), (1, 1, "leased"), (2, 1, "active")]);
        assert_eq!(slots_to_rotate(&st, Some(&v), 10), vec![0]);
    }

    #[test]
    fn unleased_slot_rotates_only_once_confirmed_unleased_after_its_window() {
        let mut st = fresh(1, 0);
        st.slots[0].reported = true;
        let leasable_until = st.slots[0].valid_until - 600;
        // Snapshot from before the window closed: it might have been leased since.
        let stale = view(leasable_until - 1, &[(0, 1, "active")]);
        assert!(slots_to_rotate(&st, Some(&stale), leasable_until + 5).is_empty());
        let confirmed = view(leasable_until, &[(0, 1, "active")]);
        assert_eq!(
            slots_to_rotate(&st, Some(&confirmed), leasable_until + 5),
            vec![0]
        );
        // A remote row for an older generation says nothing about this one.
        let old_gen = view(leasable_until, &[(0, 0, "revoked")]);
        assert!(slots_to_rotate(&st, Some(&old_gen), 10).is_empty());
    }

    #[test]
    fn unreported_slot_past_its_window_rotates() {
        let st = fresh(1, 0);
        let leasable_until = st.slots[0].valid_until - DEFAULT_MIN_REMAINING_SECS;
        assert!(slots_to_rotate(&st, None, leasable_until - 1).is_empty());
        assert_eq!(slots_to_rotate(&st, None, leasable_until), vec![0]);
    }

    #[test]
    fn rotation_bumps_generation_changes_secret_and_marks_unapplied() {
        let mut st = fresh(2, 0);
        st.applied = true;
        st.slots.iter_mut().for_each(|s| s.reported = true);
        let old = st.slots[1].clone();
        assert!(plan(&mut st, 2, &[1], 100, LIFE));
        assert!(!st.applied);
        assert_eq!(st.slots[1].generation, old.generation + 1);
        assert_ne!(st.slots[1].vless_uuid, old.vless_uuid);
        assert_ne!(st.slots[1].hysteria2_password, old.hysteria2_password);
        assert!(!st.slots[1].reported);
        assert!(st.slots[0].reported, "untouched slot keeps its state");
        assert!(
            !plan(&mut st, 2, &[], 100, LIFE),
            "no-op plan changes nothing"
        );
    }

    #[test]
    fn shrinking_to_zero_empties_the_pool() {
        let mut st = fresh(4, 0);
        assert!(plan(&mut st, 0, &[], 0, LIFE));
        assert!(st.slots.is_empty());
    }

    #[test]
    fn state_persists_atomically_with_0600_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("lease-pool.json");
        let st = fresh(3, 0);
        save_state(&path, &st).unwrap();
        assert_eq!(load_state(&path).unwrap(), st);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(
            load_state(&dir.path().join("missing.json")).unwrap(),
            LeaseState::default()
        );
    }

    #[test]
    fn sync_body_sends_secrets_only_for_unreported_generations() {
        let mut st = fresh(2, 0);
        st.slots[0].reported = true;
        let body = sync_body(&st, None).unwrap();
        let slots = body["slots"].as_array().unwrap();
        assert!(slots[0].get("vless_uuid").is_none());
        assert!(slots[0].get("hysteria2_password").is_none());
        assert_eq!(slots[1]["vless_uuid"], json!(st.slots[1].vless_uuid));
        assert!(body.get("hysteria2_obfs_password").is_none());
        assert!(sync_body(&st, Some("obfs")).unwrap()["hysteria2_obfs_password"] == json!("obfs"));
    }

    #[test]
    fn absorb_marks_reported_by_generation_and_reads_server_policy() {
        let mut st = fresh(2, 0);
        let resp = json!({
            "as_of": "2026-09-26T00:00:00Z",
            "min_remaining_seconds": 300,
            "need_secret": [1],
            "slots": [{"slot": 0, "generation": 1, "state": "active"}]
        });
        let v = absorb_sync_response(&mut st, &resp).unwrap();
        assert!(st.slots[0].reported);
        assert!(!st.slots[1].reported);
        assert_eq!(v.min_remaining, 300);
    }

    #[test]
    fn debug_never_prints_secrets() {
        let st = fresh(1, 0);
        let dbg = format!("{:?}", st);
        assert!(!dbg.contains(&st.slots[0].vless_uuid));
        assert!(!dbg.contains(&st.slots[0].hysteria2_password));
    }
}
