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
//! sing-box 1.14.1 cannot change inbound users at runtime, and every apply
//! that changes the rendered config restarts it (SIGHUP reload was measured
//! to cut open connections too), dropping every open connection on the
//! node. So disruption is bounded two ways:
//!
//! * **Renewal extends, it does not rotate.** A leased slot whose lease the
//!   control plane renewed carries `extend_to`; the node adopts it (floored
//!   to the batch grid, capped at `now + lifetime`) by changing only the
//!   user's `expires_at` in vpn-admin's store. The rendered sing-box config
//!   is identical, so vpn-admin does not restart sing-box.
//! * **Batched rotation.** Every `valid_until` sits on a grid of
//!   `rotation_batch_interval_secs`, so expiries coincide with batch
//!   boundaries. Non-urgent rotations (non-urgent revocations, unleased
//!   slots whose leasable window closed) are deferred to the first poll
//!   after the next boundary; expired and urgently revoked slots rotate
//!   immediately and take every pending rotation with them. Result: at
//!   most one rotation apply per batch window plus one per urgent
//!   revocation, and an expired credential stops working at its
//!   `valid_until` + one poll + apply time.
//!
//! A new generation is reported to the control plane only after vpn-admin
//! applied it to the live sing-box (fail-closed apply: lock, render,
//! `sing-box check`, atomic rename, reload, verify), so any credential the
//! control plane can lease already works. Secrets never reach a log line.

use crate::config::AgentConfig;
use crate::dispatch::{parse_json_output, vpn_admin_command};
use crate::worker_client::WorkerClient;
use anyhow::{anyhow, Context, Result};
use compat_config::authorization::{Authorization, AuthorizationSet, CredentialClass};
use compat_config::credentials;
use compat_config::secret::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const MAX_POOL_SIZE: usize = 1024;
/// Default rotation batch window (`rotation_batch_interval_secs`). Every
/// valid_until lies on this grid, so slots expire together, at a boundary.
pub const DEFAULT_BATCH_SECS: i64 = 600;
/// Used until the control plane tells us its own value.
pub const DEFAULT_MIN_REMAINING_SECS: i64 = 600;
const VPN_ADMIN_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Slot {
    pub slot: u32,
    pub generation: u64,
    pub valid_until: i64,
    /// Stable for the logical native lease slot across credential rotation.
    #[serde(default)]
    pub principal_id: String,
    /// Unique to this credential generation; rotates with both protocol secrets.
    #[serde(default)]
    pub credential_id: String,
    #[serde(default = "native_class")]
    pub class: CredentialClass,
    #[serde(default)]
    pub valid_from: i64,
    #[serde(default)]
    pub revoked: bool,
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
            .field("principal_id", &self.principal_id)
            .field("credential_id", &self.credential_id)
            .field("class", &self.class)
            .field("valid_from", &self.valid_from)
            .field("revoked", &self.revoked)
            .field("reported", &self.reported)
            .finish_non_exhaustive()
    }
}

fn native_class() -> CredentialClass {
    CredentialClass::Native
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct LeaseState {
    pub slots: Vec<Slot>,
    /// The current `slots` are what the live sing-box config holds.
    #[serde(default)]
    pub applied: bool,
    /// Unix time of the last rotation (for batching; survives restarts).
    #[serde(default)]
    pub last_rotation_at: i64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ExternalProtocol {
    vless_uuid: String,
    hysteria2_password: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalAuthorizationWire {
    principal_id: String,
    credential_id: String,
    class: CredentialClass,
    logical_route_id: String,
    valid_from: String,
    valid_until: String,
    revoked: bool,
    protocol: ExternalProtocol,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalAuthorizationResponse {
    authorizations: Vec<ExternalAuthorizationWire>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ExternalAuthorization {
    principal_id: String,
    credential_id: String,
    class: CredentialClass,
    logical_route_id: String,
    valid_from: i64,
    valid_until: i64,
    revoked: bool,
    vless_uuid: String,
    hysteria2_password: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ExternalAuthorizationState {
    authorizations: Vec<ExternalAuthorization>,
}

/// The control plane's view of one slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSlot {
    pub generation: u64,
    pub state: String,
    /// Revocation that must not wait for the next batch (abuse/admin).
    pub urgent: bool,
    /// Renewal: the lease holding this generation was extended to this
    /// time (unix seconds); the node may adopt it.
    pub extend_to: Option<i64>,
}

/// What the control plane last said about each slot.
#[derive(Clone, Debug, Default)]
pub struct RemoteView {
    /// Server time of the snapshot (unix seconds).
    pub as_of: i64,
    pub min_remaining: i64,
    pub slots: HashMap<u32, RemoteSlot>,
}

pub fn round_down_to_grid(t: i64, grid: i64) -> i64 {
    t.div_euclid(grid) * grid
}

pub fn clamp_batch_interval(secs: u64) -> i64 {
    secs.clamp(60, DEFAULT_BATCH_SECS as u64) as i64
}

/// Slot lifetime, clamped so that valid_until never exceeds the native 30-minute horizon
/// (the control plane's limit) and (b) a freshly minted slot, floored to
/// the grid, still has a leasable window of at least 60 s.
pub fn clamp_lifetime(secs: u64, grid: i64) -> i64 {
    let min = (DEFAULT_MIN_REMAINING_SECS + grid + 60).max(900);
    (secs as i64).clamp(min, compat_config::authorization::NATIVE_LEASE_SECS)
}

fn mint(slot: u32, generation: u64, now: i64, lifetime: i64, grid: i64) -> Slot {
    Slot {
        slot,
        generation,
        // Floored (never later than now + lifetime) onto the batch grid.
        valid_until: round_down_to_grid(now + lifetime, grid),
        principal_id: credentials::generate_native_principal_id(),
        credential_id: credentials::generate_credential_id(),
        class: CredentialClass::Native,
        valid_from: now,
        revoked: false,
        vless_uuid: credentials::generate_uuid_v4(),
        hysteria2_password: credentials::generate_hysteria2_password(),
        reported: false,
    }
}

/// Why a slot needs a new generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// valid_until passed: the hard bound, no control plane needed.
    Expired,
    /// Revoked with the urgent flag (abuse/admin).
    Urgent,
    /// Non-urgent revocation, or an unleased slot whose window closed:
    /// waits for the next batch boundary.
    Deferrable,
}

/// Pure: every slot that needs a fresh generation, and why.
pub fn rotation_candidates(
    state: &LeaseState,
    remote: Option<&RemoteView>,
    now: i64,
) -> Vec<(u32, Why)> {
    let min_remaining = remote
        .map(|r| r.min_remaining)
        .unwrap_or(DEFAULT_MIN_REMAINING_SECS);
    state
        .slots
        .iter()
        .filter_map(|s| {
            if s.valid_until <= now {
                return Some((s.slot, Why::Expired));
            }
            let leasable_until = s.valid_until - min_remaining;
            if !s.reported && now >= leasable_until {
                return Some((s.slot, Why::Deferrable)); // never leasable, nobody holds it
            }
            let r = remote?;
            match r.slots.get(&s.slot) {
                Some(rs) if rs.generation == s.generation => {
                    if rs.state == "revoked" {
                        Some((
                            s.slot,
                            if rs.urgent {
                                Why::Urgent
                            } else {
                                Why::Deferrable
                            },
                        ))
                    } else if rs.state == "active" && r.as_of >= leasable_until {
                        Some((s.slot, Why::Deferrable))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        })
        .collect()
}

/// Pure batching decision: which slots rotate on this tick.
///
/// Expired or urgently revoked slots force a rotation now, and every other
/// pending candidate rides along (the restart is paid anyway). Otherwise
/// deferrable candidates rotate only once a grid boundary has passed since
/// the last rotation, so non-urgent rotations cost at most one apply per
/// batch window.
pub fn slots_to_rotate(
    state: &LeaseState,
    remote: Option<&RemoteView>,
    now: i64,
    grid: i64,
) -> Vec<u32> {
    let candidates = rotation_candidates(state, remote, now);
    let forced = candidates.iter().any(|(_, why)| *why != Why::Deferrable);
    let boundary_passed = now.div_euclid(grid) > state.last_rotation_at.div_euclid(grid);
    if forced || (!candidates.is_empty() && boundary_passed) {
        candidates.into_iter().map(|(slot, _)| slot).collect()
    } else {
        Vec::new()
    }
}

/// Pure: adopt control-plane renewals. A leased, still-valid generation
/// whose lease was extended moves its valid_until forward to `extend_to`,
/// floored to the grid and capped at `now + lifetime`; never backwards,
/// never for a generation that is expired, revoked, active or superseded.
/// Only `expires_at` in vpn-admin's store changes, so the rendered sing-box
/// config — and the running process — stay the same. Returns how many
/// slots were extended.
pub fn adopt_extensions(
    state: &mut LeaseState,
    remote: &RemoteView,
    now: i64,
    lifetime: i64,
    grid: i64,
) -> usize {
    let cap = round_down_to_grid(now + lifetime, grid);
    let mut n = 0;
    for s in &mut state.slots {
        let Some(rs) = remote.slots.get(&s.slot) else {
            continue;
        };
        if rs.generation != s.generation || rs.state != "leased" || s.valid_until <= now {
            continue;
        }
        if let Some(t) = rs.extend_to {
            let next = round_down_to_grid(t, grid).min(cap);
            if next > s.valid_until {
                s.valid_from = now;
                s.valid_until = next;
                s.revoked = false;
                n += 1;
            }
        }
    }
    if n > 0 {
        state.applied = false;
    }
    n
}

/// Pure: resize to `size` and rotate `rotate`. Returns whether anything changed.
pub fn plan(
    state: &mut LeaseState,
    size: usize,
    rotate: &[u32],
    now: i64,
    lifetime: i64,
    grid: i64,
) -> bool {
    let size = size.min(MAX_POOL_SIZE);
    let mut changed = false;
    let before = state.slots.len();
    state.slots.retain(|s| (s.slot as usize) < size);
    changed |= state.slots.len() != before;
    for slot in 0..size as u32 {
        match state.slots.iter_mut().find(|s| s.slot == slot) {
            Some(existing) if rotate.contains(&slot) => {
                let principal_id = existing.principal_id.clone();
                *existing = mint(slot, existing.generation + 1, now, lifetime, grid);
                existing.principal_id = principal_id;
                changed = true;
            }
            Some(_) => {}
            None => {
                state.slots.push(mint(slot, 1, now, lifetime, grid));
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
        Ok(bytes) => {
            let mut state: LeaseState = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing lease state {path:?} (contents not shown)"))?;
            // One-time, secret-preserving legacy migration. Empty serde-defaulted ids can only
            // come from the pre-authorization schema. They are minted locally, then persisted
            // before apply; vpn-admin replaces all lease-* users with this exact cred_* set.
            for slot in &mut state.slots {
                if slot.principal_id.is_empty() {
                    slot.principal_id = credentials::generate_native_principal_id();
                }
                if slot.credential_id.is_empty() {
                    slot.credential_id = credentials::generate_credential_id();
                }
                if slot.valid_from == 0 {
                    slot.valid_from =
                        slot.valid_until - compat_config::authorization::NATIVE_LEASE_SECS;
                }
            }
            Ok(state)
        }
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

const MAX_EXTERNAL_AUTHORIZATIONS: usize = 4096;

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn valid_password(value: &str) -> bool {
    (16..=128).contains(&value.len()) && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

fn valid_route_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("route_") else {
        return false;
    };
    (3..=60).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn external_as_authorization(item: &ExternalAuthorization) -> Authorization {
    Authorization {
        principal_id: item.principal_id.clone(),
        credential_id: item.credential_id.clone(),
        class: item.class,
        valid_from: item.valid_from,
        valid_until: item.valid_until,
        revoked: item.revoked,
        vless_uuid: Some(SecretString::new(item.vless_uuid.clone())),
        hysteria2_password: Some(SecretString::new(item.hysteria2_password.clone())),
    }
}

fn validate_external_state(state: &ExternalAuthorizationState) -> Result<()> {
    if state.authorizations.len() > MAX_EXTERNAL_AUTHORIZATIONS {
        return Err(anyhow!(
            "external authorization snapshot exceeds {MAX_EXTERNAL_AUTHORIZATIONS} records"
        ));
    }
    let mut routes = BTreeSet::new();
    let mut auths = Vec::with_capacity(state.authorizations.len());
    for item in &state.authorizations {
        if item.class != CredentialClass::Compatibility {
            return Err(anyhow!(
                "external authorization class must be compatibility"
            ));
        }
        if !valid_route_id(&item.logical_route_id) {
            return Err(anyhow!(
                "external authorization has invalid logical_route_id"
            ));
        }
        if !valid_uuid(&item.vless_uuid) || !valid_password(&item.hysteria2_password) {
            return Err(anyhow!(
                "external authorization has malformed protocol secret"
            ));
        }
        // Bound memory used by attacker-controlled route ids even though the
        // node does not otherwise interpret them operationally.
        routes.insert(item.logical_route_id.as_str());
        auths.push(external_as_authorization(item));
    }
    AuthorizationSet {
        authorizations: auths,
    }
    .validate()
    .map_err(anyhow::Error::msg)
}

fn decode_external_response(value: Value) -> Result<ExternalAuthorizationState> {
    let response: ExternalAuthorizationResponse = serde_json::from_value(value)
        .context("validating /api/agent/authorizations response shape")?;
    let mut authorizations = Vec::with_capacity(response.authorizations.len());
    for item in response.authorizations {
        let valid_from = OffsetDateTime::parse(&item.valid_from, &Rfc3339)
            .context("parsing external valid_from")?
            .unix_timestamp();
        let valid_until = OffsetDateTime::parse(&item.valid_until, &Rfc3339)
            .context("parsing external valid_until")?
            .unix_timestamp();
        authorizations.push(ExternalAuthorization {
            principal_id: item.principal_id,
            credential_id: item.credential_id,
            class: item.class,
            logical_route_id: item.logical_route_id,
            valid_from,
            valid_until,
            revoked: item.revoked,
            vless_uuid: item.protocol.vless_uuid,
            hysteria2_password: item.protocol.hysteria2_password,
        });
    }
    authorizations.sort_by(|a, b| {
        (&a.principal_id, &a.credential_id).cmp(&(&b.principal_id, &b.credential_id))
    });
    let state = ExternalAuthorizationState { authorizations };
    validate_external_state(&state)?;
    Ok(state)
}

fn active_external(state: &ExternalAuthorizationState, now: i64) -> Vec<ExternalAuthorization> {
    state
        .authorizations
        .iter()
        .filter(|a| !a.revoked && a.valid_from <= now && now < a.valid_until)
        .cloned()
        .collect()
}

fn load_external_state(path: &Path) -> Result<Option<ExternalAuthorizationState>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let state: ExternalAuthorizationState =
                serde_json::from_slice(&bytes).with_context(|| {
                    format!("parsing external authorization state {path:?} (contents not shown)")
                })?;
            validate_external_state(&state)?;
            Ok(Some(state))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading external authorization state {path:?}")),
    }
}

fn save_external_state(path: &Path, state: &ExternalAuthorizationState) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("external authorization state path has no parent"))?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .context("creating external authorization state temp file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("restricting external authorization state temp file")?;
    }
    use std::io::Write;
    tmp.write_all(&serde_json::to_vec(state)?)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| anyhow!("installing external authorization state: {}", e.error))?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn vpn_admin_input(state: &LeaseState, external: Option<&ExternalAuthorizationState>) -> Value {
    let mut document = json!({
        "slots": state.slots.iter().map(|s| json!({
            "slot": s.slot,
            "principal_id": s.principal_id,
            "credential_id": s.credential_id,
            "class": s.class,
            "valid_from": s.valid_from,
            "revoked": s.revoked,
            "vless_uuid": s.vless_uuid,
            "hysteria2_password": s.hysteria2_password,
            "expires_at": s.valid_until,
        })).collect::<Vec<_>>()
    });
    if let Some(external) = external {
        document["compatibility_authorizations"] = json!(external
            .authorizations
            .iter()
            .map(|a| json!({
                "principal_id": a.principal_id,
                "credential_id": a.credential_id,
                "class": a.class,
                "logical_route_id": a.logical_route_id,
                "valid_from": a.valid_from,
                "valid_until": a.valid_until,
                "revoked": a.revoked,
                "vless_uuid": a.vless_uuid,
                "hysteria2_password": a.hysteria2_password,
            }))
            .collect::<Vec<_>>());
    }
    document
}

fn rfc3339(t: i64) -> Result<String> {
    Ok(OffsetDateTime::from_unix_timestamp(t)?.format(&Rfc3339)?)
}

/// Body for `/api/agent/leases/sync`: every slot, secrets only for
/// generations the control plane has not stored yet.
pub fn sync_body(
    state: &LeaseState,
    obfs_password: Option<&str>,
    lifetime: i64,
    grid: i64,
) -> Result<Value> {
    let mut slots = Vec::with_capacity(state.slots.len());
    for s in &state.slots {
        let mut item = json!({
            "slot": s.slot,
            "generation": s.generation,
            "principal_id": s.principal_id,
            "credential_id": s.credential_id,
            "class": s.class,
            "valid_from": rfc3339(s.valid_from)?,
            "revoked": s.revoked,
            "valid_until": rfc3339(s.valid_until)?,
        });
        if !s.reported {
            item["vless_uuid"] = json!(s.vless_uuid);
            item["hysteria2_password"] = json!(s.hysteria2_password);
        }
        slots.push(item);
    }
    // The control plane computes renewal targets on this node's grid and
    // never past its lifetime, so what it returns is what the node enforces.
    let mut body = json!({
        "slots": slots,
        "policy": {
            "rotation_batch_interval_secs": grid,
            "slot_lifetime_secs": lifetime,
        },
    });
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
        let extend_to = match item.get("extend_to").and_then(Value::as_str) {
            Some(t) => Some(
                OffsetDateTime::parse(t, &Rfc3339)
                    .context("parsing sync extend_to")?
                    .unix_timestamp(),
            ),
            None => None,
        };
        view.slots.insert(
            slot as u32,
            RemoteSlot {
                generation,
                state: st.to_string(),
                urgent: item.get("urgent").and_then(Value::as_bool) == Some(true),
                extend_to,
            },
        );
    }
    for s in &mut state.slots {
        s.reported = matches!(view.slots.get(&s.slot), Some(r) if r.generation == s.generation);
        s.revoked = matches!(view.slots.get(&s.slot), Some(r) if r.generation == s.generation && r.state == "revoked");
    }
    Ok(view)
}

pub struct LeasePool {
    path: PathBuf,
    state: LeaseState,
    external_path: PathBuf,
    /// None means this node has never obtained a valid external snapshot.
    /// In that state vpn-admin receives no external field and preserves any
    /// already-managed ext_* users during a staged agent rollout.
    external: Option<ExternalAuthorizationState>,
    /// Exact active external projection last proven live by vpn-admin in this
    /// process. Time-boundary changes make this differ without new control
    /// plane input, which is how expiry remains enforced during outages.
    applied_external: Option<Vec<ExternalAuthorization>>,
    /// Credential ids last acknowledged to the control plane after a proven
    /// live apply. None forces a re-ack after every agent restart.
    acked_external: Option<Vec<String>>,
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
        let external_path = PathBuf::from(&cfg.external_authorization_state_file);
        let external = match load_external_state(&external_path) {
            Ok(state) => state,
            Err(err) => {
                // Keep native authorization alive if only the external cache
                // is corrupt. Existing ext_* users stay untouched until a
                // fresh validated snapshot repairs the cache.
                tracing::error!(
                    error = %err,
                    "external authorization cache unreadable; preserving live external users until resync"
                );
                None
            }
        };
        Ok(Self {
            path,
            state,
            external_path,
            external,
            applied_external: None,
            acked_external: None,
            remote: None,
            obfs_password: None,
            obfs_reported: false,
            applied_this_process: false,
        })
    }

    pub async fn tick(&mut self, cfg: &AgentConfig, client: &WorkerClient) -> Result<()> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let grid = clamp_batch_interval(cfg.rotation_batch_interval_secs);
        let lifetime = clamp_lifetime(cfg.lease_slot_lifetime_secs, grid);
        let rotate = slots_to_rotate(&self.state, self.remote.as_ref(), now, grid);
        if !rotate.is_empty() {
            self.state.last_rotation_at = now;
        }
        if plan(
            &mut self.state,
            cfg.lease_pool_size,
            &rotate,
            now,
            lifetime,
            grid,
        ) {
            // Persist BEFORE applying: a crash mid-apply re-applies this
            // exact state on restart instead of forgetting a live secret.
            save_state(&self.path, &self.state)?;
            if !rotate.is_empty() {
                tracing::info!(rotated = rotate.len(), "lease pool: rotating slots");
            }
        }
        // External desired state is independent of the native lease pool. A
        // malformed or unavailable control plane can never replace the last
        // good local snapshot; it only delays new changes. Secrets are never
        // included in these error messages.
        match client.fetch_authorizations().await {
            Ok(raw) => match decode_external_response(raw) {
                Ok(next) => {
                    if self.external.as_ref() != Some(&next) {
                        // Persist before live apply so a crash cannot forget a
                        // credential that may already have reached sing-box.
                        save_external_state(&self.external_path, &next)?;
                        self.external = Some(next);
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "rejecting malformed external authorization snapshot"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "external authorization fetch failed; enforcing last known local snapshot"
                );
            }
        }

        let active_now = self
            .external
            .as_ref()
            .map(|state| active_external(state, now));
        let external_needs_apply = match (&active_now, &self.applied_external) {
            (Some(active), Some(applied)) => active != applied,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if !self.state.applied || !self.applied_this_process || external_needs_apply {
            self.apply(cfg, now).await?;
        }

        if let Some(applied) = &self.applied_external {
            let mut ids: Vec<String> = applied
                .iter()
                .map(|authorization| authorization.credential_id.clone())
                .collect();
            ids.sort();
            ids.dedup();
            if self.acked_external.as_ref() != Some(&ids) {
                match client.ack_authorizations(&ids).await {
                    Ok(()) => self.acked_external = Some(ids),
                    Err(err) => tracing::warn!(
                        error = %err,
                        "external authorization live acknowledgement failed; publication remains gated"
                    ),
                }
            }
        }

        if cfg.lease_pool_size == 0 && self.state.slots.is_empty() {
            return Ok(());
        }
        let obfs = (!self.obfs_reported)
            .then_some(self.obfs_password.as_deref())
            .flatten();
        let body = sync_body(&self.state, obfs, lifetime, grid)?;
        let resp = client.sync_leases(&body).await?;
        let before = self.state.clone();
        let view = absorb_sync_response(&mut self.state, &resp)?;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let extended = adopt_extensions(&mut self.state, &view, now, lifetime, grid);
        if extended > 0 {
            // Persist first; the next tick applies it (store-only change,
            // no sing-box restart).
            tracing::info!(extended, "lease pool: renewals adopted");
        }
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

    async fn apply(&mut self, cfg: &AgentConfig, now: i64) -> Result<()> {
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
            f.write_all(&serde_json::to_vec(&vpn_admin_input(
                &self.state,
                self.external.as_ref(),
            ))?)?;
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
        self.applied_external = self
            .external
            .as_ref()
            .map(|state| active_external(state, now));
        save_state(&self.path, &self.state)?;
        tracing::info!(
            slots = self.state.slots.len(),
            external_active = self.applied_external.as_ref().map_or(0, Vec::len),
            "managed authorization projection applied live"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIFE: i64 = 1800;
    const GRID: i64 = 600;

    fn fresh(size: usize, now: i64) -> LeaseState {
        let mut st = LeaseState::default();
        plan(&mut st, size, &[], now, LIFE, GRID);
        st
    }

    fn rs(generation: u64, state: &str) -> RemoteSlot {
        RemoteSlot {
            generation,
            state: state.to_string(),
            urgent: false,
            extend_to: None,
        }
    }

    fn view(as_of: i64, entries: &[(u32, RemoteSlot)]) -> RemoteView {
        RemoteView {
            as_of,
            min_remaining: 600,
            slots: entries.iter().cloned().collect(),
        }
    }

    fn reported(mut st: LeaseState) -> LeaseState {
        st.slots.iter_mut().for_each(|s| s.reported = true);
        st
    }

    #[test]
    fn plan_mints_a_bounded_pool_of_distinct_random_slots_on_the_grid() {
        let st = fresh(32, 1_000);
        assert_eq!(st.slots.len(), 32);
        assert!(!st.applied);
        let uuids: std::collections::HashSet<_> = st.slots.iter().map(|s| &s.vless_uuid).collect();
        assert_eq!(uuids.len(), 32);
        assert!(st.slots.iter().all(|s| s.valid_until % GRID == 0));
        assert!(st
            .slots
            .iter()
            .all(|s| s.valid_until <= 1_000 + LIFE && s.valid_until > 1_000 + LIFE - GRID));
        let capped = fresh(5000, 1_000);
        assert_eq!(capped.slots.len(), MAX_POOL_SIZE);
    }

    #[test]
    fn lifetime_clamp_keeps_a_leasable_window_and_native_30_minute_cap() {
        assert_eq!(clamp_lifetime(60, 600), 1260);
        assert_eq!(clamp_lifetime(1800, 600), 1800);
        assert_eq!(clamp_lifetime(99_999, 600), 1800);
        assert_eq!(clamp_lifetime(900, 60), 900);
        assert_eq!(clamp_batch_interval(1), 60);
        assert_eq!(clamp_batch_interval(99_999), 600);
        for grid in [60, 300, 600] {
            let life = clamp_lifetime(0, grid);
            for now in [0, 1, grid - 1, 12_345] {
                let vu = round_down_to_grid(now + life, grid);
                assert!(vu <= now + 1800);
                assert!(
                    vu - DEFAULT_MIN_REMAINING_SECS - now >= 60,
                    "grid {grid} now {now}"
                );
            }
        }
    }

    #[test]
    fn expired_slots_rotate_without_any_control_plane() {
        let st = reported(fresh(2, 0));
        let end = st.slots[0].valid_until;
        assert!(slots_to_rotate(&st, None, end - 1, GRID).is_empty());
        assert_eq!(slots_to_rotate(&st, None, end, GRID), vec![0, 1]);
    }

    #[test]
    fn expiry_is_never_deferred_even_right_after_a_rotation() {
        let mut st = reported(fresh(1, 0));
        let end = st.slots[0].valid_until;
        st.last_rotation_at = end;
        assert_eq!(slots_to_rotate(&st, None, end, GRID), vec![0]);
    }

    #[test]
    fn non_urgent_revocation_waits_for_the_next_batch_boundary() {
        let mut st = reported(fresh(3, 0));
        st.slots.iter_mut().for_each(|s| s.valid_until = 6_000);
        st.last_rotation_at = 1_210; // rotated in window [1200, 1800)
        let v = view(
            1_300,
            &[
                (0, rs(1, "revoked")),
                (1, rs(1, "leased")),
                (2, rs(1, "active")),
            ],
        );
        assert_eq!(
            rotation_candidates(&st, Some(&v), 1_300),
            vec![(0, Why::Deferrable)]
        );
        assert!(slots_to_rotate(&st, Some(&v), 1_300, GRID).is_empty());
        assert!(slots_to_rotate(&st, Some(&v), 1_799, GRID).is_empty());
        assert_eq!(slots_to_rotate(&st, Some(&v), 1_800, GRID), vec![0]);
    }

    #[test]
    fn urgent_revocation_rotates_now_and_takes_pending_rotations_along() {
        let mut st = reported(fresh(3, 0));
        st.last_rotation_at = 1_210;
        let mut urgent = rs(1, "revoked");
        urgent.urgent = true;
        let v = view(
            1_300,
            &[(0, rs(1, "revoked")), (1, urgent), (2, rs(1, "leased"))],
        );
        assert_eq!(slots_to_rotate(&st, Some(&v), 1_300, GRID), vec![0, 1]);
    }

    #[test]
    fn batching_bounds_non_urgent_rotations_to_one_per_window() {
        // Simulate one revocation every 10 s for an hour: rotations happen
        // only at boundaries.
        let mut st = reported(fresh(64, 0));
        st.last_rotation_at = 0;
        let mut rotations = 0;
        let mut revoked: Vec<u32> = Vec::new();
        for (i, now) in (1..360).map(|k| k * 10).enumerate() {
            revoked.push((i % 64) as u32);
            let entries: Vec<_> = st
                .slots
                .iter()
                .map(|s| {
                    let state = if revoked.contains(&s.slot) {
                        "revoked"
                    } else {
                        "leased"
                    };
                    (s.slot, rs(s.generation, state))
                })
                .collect();
            let v = view(now, &entries);
            let rot = slots_to_rotate(&st, Some(&v), now, GRID);
            if !rot.is_empty() {
                rotations += 1;
                st.last_rotation_at = now;
                // Rotated slots come back as fresh, reported, leased ones
                // that don't expire within the test.
                for s in &mut st.slots {
                    if rot.contains(&s.slot) {
                        s.generation += 1;
                        s.valid_until = 100_000;
                    }
                }
                revoked.retain(|r| !rot.contains(r));
            }
        }
        assert!(rotations <= 3600 / GRID as usize, "rotations = {rotations}");
        assert!(rotations >= 5);
    }

    #[test]
    fn unleased_slot_rotates_only_once_confirmed_unleased_after_its_window() {
        let mut st = reported(fresh(1, 0));
        let leasable_until = st.slots[0].valid_until - 600;
        let stale = view(leasable_until - 1, &[(0, rs(1, "active"))]);
        assert!(rotation_candidates(&st, Some(&stale), leasable_until + 5).is_empty());
        let confirmed = view(leasable_until, &[(0, rs(1, "active"))]);
        st.last_rotation_at = leasable_until;
        assert!(
            slots_to_rotate(&st, Some(&confirmed), leasable_until + 5, GRID).is_empty(),
            "deferred: a rotation already happened in this window"
        );
        st.last_rotation_at = 0;
        assert_eq!(
            slots_to_rotate(&st, Some(&confirmed), leasable_until + 5, GRID),
            vec![0]
        );
        let old_gen = view(leasable_until, &[(0, rs(0, "revoked"))]);
        assert!(rotation_candidates(&st, Some(&old_gen), 10).is_empty());
    }

    #[test]
    fn unreported_slot_past_its_window_rotates() {
        let st = fresh(1, 0);
        let leasable_until = st.slots[0].valid_until - DEFAULT_MIN_REMAINING_SECS;
        assert!(slots_to_rotate(&st, None, leasable_until - 1, GRID).is_empty());
        assert_eq!(slots_to_rotate(&st, None, leasable_until, GRID), vec![0]);
    }

    #[test]
    fn renewal_extends_in_place_floored_capped_and_only_for_live_leased_slots() {
        let mut st = reported(fresh(5, 0));
        st.applied = true;
        let vu = st.slots[0].valid_until; // 1800
        let before = st.clone();
        let before_admin = vpn_admin_input(&st, None);
        let ext = |g: u64, state: &str, t: i64| RemoteSlot {
            extend_to: Some(t),
            ..rs(g, state)
        };
        let v = view(
            1_000,
            &[
                (0, ext(1, "leased", 2_999)),  // floored to 2400
                (1, ext(1, "leased", 99_999)), // capped at now+LIFE floored = 2400
                (2, ext(1, "active", 2_999)),  // never leased: no
                (3, ext(1, "revoked", 2_999)), // revoked: no
                (4, ext(2, "leased", 2_999)),  // other generation: no
            ],
        );
        assert_eq!(adopt_extensions(&mut st, &v, 1_000, LIFE, GRID), 2);
        assert_eq!(st.slots[0].valid_until, 2_400);
        assert_eq!(st.slots[1].valid_until, 2_400);
        for i in 2..5 {
            assert_eq!(st.slots[i].valid_until, vu);
        }
        assert!(!st.applied, "store must be re-applied");
        let after_admin = vpn_admin_input(&st, None);
        for i in 0..5 {
            assert_eq!(
                st.slots[i].vless_uuid, before.slots[i].vless_uuid,
                "same credential"
            );
            assert_eq!(
                st.slots[i].generation, before.slots[i].generation,
                "same slot generation"
            );
            assert_eq!(st.slots[i].credential_id, before.slots[i].credential_id);
            assert_eq!(st.slots[i].principal_id, before.slots[i].principal_id);
        }
        assert_eq!(
            before_admin["slots"][0]["vless_uuid"],
            after_admin["slots"][0]["vless_uuid"]
        );
        assert_eq!(
            before_admin["slots"][0]["hysteria2_password"],
            after_admin["slots"][0]["hysteria2_password"]
        );
        assert_eq!(
            before_admin["slots"][0]["credential_id"],
            after_admin["slots"][0]["credential_id"]
        );
        // Never backwards, never resurrects an expired generation.
        let back = view(1_000, &[(0, ext(1, "leased", 1_200))]);
        assert_eq!(adopt_extensions(&mut st, &back, 1_000, LIFE, GRID), 0);
        let mut expired = reported(fresh(1, 0));
        let late = view(1_800, &[(0, ext(1, "leased", 3_000))]);
        assert_eq!(adopt_extensions(&mut expired, &late, 1_800, LIFE, GRID), 0);
        // An extended slot is not rotated at its old valid_until.
        assert!(!rotation_candidates(&st, None, vu)
            .iter()
            .any(|(s, _)| *s == 0));
    }

    #[test]
    fn rotation_bumps_generation_changes_secret_and_marks_unapplied() {
        let mut st = reported(fresh(2, 0));
        st.applied = true;
        let old = st.slots[1].clone();
        assert!(plan(&mut st, 2, &[1], 100, LIFE, GRID));
        assert!(!st.applied);
        assert_eq!(st.slots[1].generation, old.generation + 1);
        assert_ne!(st.slots[1].vless_uuid, old.vless_uuid);
        assert_ne!(st.slots[1].hysteria2_password, old.hysteria2_password);
        assert!(!st.slots[1].reported);
        assert!(st.slots[0].reported, "untouched slot keeps its state");
        assert!(
            !plan(&mut st, 2, &[], 100, LIFE, GRID),
            "no-op plan changes nothing"
        );
    }

    #[test]
    fn shrinking_to_zero_empties_the_pool() {
        let mut st = fresh(4, 0);
        assert!(plan(&mut st, 0, &[], 0, LIFE, GRID));
        assert!(st.slots.is_empty());
    }

    #[test]
    fn state_persists_atomically_with_0600_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("lease-pool.json");
        let mut st = fresh(3, 0);
        st.last_rotation_at = 1234;
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
        // A table written before batching existed still loads.
        std::fs::write(&path, br#"{"slots":[],"applied":true}"#).unwrap();
        assert_eq!(load_state(&path).unwrap().last_rotation_at, 0);
    }

    #[test]
    fn legacy_state_migrates_ids_without_rotating_protocol_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.json");
        let uuid = "11111111-1111-4111-8111-111111111111";
        let password = "legacy-password-secret";
        std::fs::write(&path, format!(r#"{{"slots":[{{"slot":0,"generation":7,"valid_until":1800,"vless_uuid":"{uuid}","hysteria2_password":"{password}","reported":true}}],"applied":true}}"#)).unwrap();
        let migrated = load_state(&path).unwrap();
        let slot = &migrated.slots[0];
        assert!(slot.principal_id.starts_with("native_"));
        assert!(slot.credential_id.starts_with("cred_"));
        assert_eq!(slot.valid_from, 0);
        assert_eq!(slot.vless_uuid, uuid);
        assert_eq!(slot.hysteria2_password, password);
        assert_eq!(slot.generation, 7);
        let input = vpn_admin_input(&migrated, None);
        assert_eq!(input["slots"][0]["principal_id"], slot.principal_id);
        assert_eq!(input["slots"][0]["credential_id"], slot.credential_id);
        assert_eq!(input["slots"][0]["class"], "native");
    }

    #[test]
    fn sync_body_sends_secrets_only_for_unreported_generations_and_policy() {
        let mut st = fresh(2, 0);
        st.slots[0].reported = true;
        let body = sync_body(&st, None, LIFE, GRID).unwrap();
        let slots = body["slots"].as_array().unwrap();
        assert_eq!(slots[0]["principal_id"], st.slots[0].principal_id);
        assert_eq!(slots[0]["credential_id"], st.slots[0].credential_id);
        assert_eq!(slots[0]["class"], "native");
        assert!(slots[0].get("vless_uuid").is_none());
        assert!(slots[0].get("hysteria2_password").is_none());
        assert_eq!(slots[1]["vless_uuid"], json!(st.slots[1].vless_uuid));
        assert!(body.get("hysteria2_obfs_password").is_none());
        assert_eq!(body["policy"]["rotation_batch_interval_secs"], json!(GRID));
        assert_eq!(body["policy"]["slot_lifetime_secs"], json!(LIFE));
        assert!(
            sync_body(&st, Some("obfs"), LIFE, GRID).unwrap()["hysteria2_obfs_password"]
                == json!("obfs")
        );
    }

    #[test]
    fn absorb_marks_reported_by_generation_and_reads_urgency_and_extensions() {
        let mut st = fresh(3, 0);
        let resp = json!({
            "as_of": "2026-09-26T00:00:00Z",
            "min_remaining_seconds": 300,
            "need_secret": [1],
            "slots": [
                {"slot": 0, "generation": 1, "state": "leased", "extend_to": "2026-09-26T00:30:00Z"},
                {"slot": 2, "generation": 1, "state": "revoked", "urgent": true}
            ]
        });
        let v = absorb_sync_response(&mut st, &resp).unwrap();
        assert!(st.slots[0].reported);
        assert!(!st.slots[1].reported);
        assert_eq!(v.min_remaining, 300);
        assert_eq!(v.slots[&0].extend_to, Some(1_790_382_600));
        assert!(!v.slots[&0].urgent);
        assert!(v.slots[&2].urgent);
        assert_eq!(v.slots[&2].extend_to, None);
    }

    fn external_wire(extra: Value) -> Value {
        let mut item = json!({
            "principal_id": "ext_opaque0001",
            "credential_id": "cred_external001",
            "class": "compatibility",
            "logical_route_id": "route_de_fast",
            "valid_from": "2026-09-26T00:00:00Z",
            "valid_until": "2026-10-03T00:00:00Z",
            "revoked": false,
            "protocol": {
                "vless_uuid": "11111111-1111-4111-8111-111111111111",
                "hysteria2_password": "external-password-0001"
            }
        });
        if let Some(map) = extra.as_object() {
            for (key, value) in map {
                item[key] = value.clone();
            }
        }
        json!({ "authorizations": [item] })
    }

    #[test]
    fn external_snapshot_is_strict_persistent_and_expires_locally() {
        let state = decode_external_response(external_wire(json!({}))).unwrap();
        assert_eq!(state.authorizations.len(), 1);
        let start = state.authorizations[0].valid_from;
        let end = state.authorizations[0].valid_until;
        assert_eq!(active_external(&state, start).len(), 1);
        assert_eq!(active_external(&state, end - 1).len(), 1);
        assert!(active_external(&state, end).is_empty());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("external-authorizations.json");
        save_external_state(&path, &state).unwrap();
        assert!(load_external_state(&path).unwrap() == Some(state));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn external_snapshot_rejects_unknown_fields_and_bad_security_contract() {
        assert!(decode_external_response(external_wire(json!({
            "account_id": "must-never-reach-node"
        })))
        .is_err());
        assert!(decode_external_response(external_wire(json!({
            "class": "native"
        })))
        .is_err());
        assert!(decode_external_response(external_wire(json!({
            "principal_id": "user@example.com"
        })))
        .is_err());
        assert!(decode_external_response(external_wire(json!({
            "protocol": {
                "vless_uuid": "bad",
                "hysteria2_password": "short"
            }
        })))
        .is_err());
    }

    #[test]
    fn debug_never_prints_secrets() {
        let st = fresh(1, 0);
        let dbg = format!("{:?}", st);
        assert!(!dbg.contains(&st.slots[0].vless_uuid));
        assert!(!dbg.contains(&st.slots[0].hysteria2_password));
    }
}