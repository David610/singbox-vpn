//! Compatibility-domain types. Deliberately separate from
//! `config::EndpointDescriptor` (the native, signed-bundle relay
//! descriptor) — see spec §5 and
//! `docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §6. A `CompatEndpoint`
//! describes a third-party-client-facing listener (VLESS+REALITY or
//! Hysteria2) on a sing-box (or future backend) data plane; it is never
//! signed into a native `RelayBundle` and native code never parses it.

use crate::secret::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The Google/YouTube consumer domain set used by both
/// `render::youtube_direct_rule` (client-side, opt-in `compat=youtube-direct`)
/// and `server::render_server_config_for_deployment`'s relay hairpin
/// exception (`CompatUser::google_egress_hairpin`). One shared list so the
/// two mechanisms — routing this traffic to the client's own line, or
/// hairpinning it through a better-peered relay — can never silently drift
/// apart on what counts as "Google/YouTube traffic". See
/// `docs/YOUTUBE_FINAL_ROOT_CAUSE.md` §15/§16 for why the set is this broad:
/// auth, DRM/licensing, the player API, static assets and media each
/// resolve to a different Google domain, and a hole in any of them breaks
/// the feature both mechanisms exist to restore.
pub const GOOGLE_EGRESS_DOMAINS: &[&str] = &[
    "youtube.com",
    "youtubekids.com",
    "youtu.be",
    "youtube-nocookie.com",
    "googlevideo.com",
    "ytimg.com",
    "ggpht.com",
    "googleusercontent.com",
    "youtubei.googleapis.com",
    "google.com",
    "googleapis.com",
    "gstatic.com",
];

/// Default name of the reserved, non-customer protocol-probe user each
/// node creates for peer health probing (`provisioning-agent`'s
/// `protocol_probe`). The server renderer confines any user with this name
/// to [`PROBE_ALLOWED_IP_CIDRS`] / [`PROBE_ALLOWED_DOMAINS`] on
/// [`PROBE_ALLOWED_PORT`] and rejects everything else for it, so the
/// credential (handed to peer agents) is never an open proxy.
pub const PROBE_USER_NAME: &str = "arcana-probe";

/// The second reserved, node-local internal principal (spec C-16
/// "Exceptions"): created directly by `vpn-admin` on install/update when
/// no fleet `arcana-probe` user exists yet, so the local REALITY
/// self-test (install/update/rotate gates) always has a loopback
/// principal to use. Random credential, never published in any
/// subscription/provisioning document, never reported to the control
/// plane, excluded from heartbeat counts, and preserved across
/// whole-store replaces. Like `PROBE_USER_NAME`, this is only a
/// human-readable label — the actual privilege gate is the structural
/// [`CompatUser::is_reserved_probe`] flag, never this string.
pub const RESERVED_SELFTEST_USER_NAME: &str = "arcana-selftest";

/// The exact URLs the protocol prober fetches through the tunnel. Single
/// source of truth for both the prober and the server-side allowlist
/// below (a unit test pins every URL's host into that allowlist).
///
/// IP literal, no DNS needed: `https_ipv4`, `egress_ipv4`, latency.
pub const PROBE_TRACE_V4_URL: &str = "https://1.1.1.1/cdn-cgi/trace";
/// Resolved by the server (socks5h): the `dns` dimension.
pub const PROBE_DNS_URL: &str = "https://www.gstatic.com/generate_204";
/// AAAA-only host resolved by the server (socks5h): the `ipv6` dimension.
pub const PROBE_V6_ONLY_URL: &str = "https://ipv6.icanhazip.com";

/// C-16 data-plane egress policy (cross-repo remediation plan, invariant
/// 3): IPv4 ranges an authenticated tunnel user must never reach through
/// an Arcana exit — loopback, link-local/metadata, RFC1918/CGNAT,
/// documentation/benchmark ranges, and multicast/reserved. Applied by
/// `apply_c16_egress_policy` (in `crate::server`) ahead of the exit's
/// otherwise-unconditional `direct` outbound. Kept as one array (rather
/// than split reject/allow-with-exception lists) so every call site pins
/// the exact same set — see that function's doc comment for why DNS
/// resolution happens before these rules are evaluated.
pub const C16_DENY_IPV4_CIDRS: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

/// C-16 IPv6 deny set — loopback, IPv4-mapped/NAT64 embeddings, ULA,
/// link-local, documentation, multicast, and the well-known EC2 IMDS
/// link-local address expressed as a full /128 (the /10 `fe80::/10`
/// entry already covers it, this entry is defense-in-depth in case that
/// range is ever narrowed).
pub const C16_DENY_IPV6_CIDRS: &[&str] = &[
    "::/128",
    "::1/128",
    "::ffff:0:0/96",
    "64:ff9b::/96",
    "100::/64",
    "2001:db8::/32",
    "fc00::/7",
    "fe80::/10",
    "ff00::/8",
    "fd00:ec2::254/128",
];

/// TCP destination port C-16 rejects for every user on every node
/// regardless of destination (SMTP; spam-relay abuse mitigation, not a
/// pivot-protection control).
pub const C16_DENY_TCP_PORT: u16 = 25;

/// Destinations the probe user may reach (IP-literal URLs above).
pub const PROBE_ALLOWED_IP_CIDRS: &[&str] = &["1.1.1.1/32"];
/// Destinations the probe user may reach (exact host names, not suffixes).
pub const PROBE_ALLOWED_DOMAINS: &[&str] = &["www.gstatic.com", "ipv6.icanhazip.com"];
/// The only destination port the probe user may reach (all URLs are HTTPS).
pub const PROBE_ALLOWED_PORT: u16 = 443;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatTransport {
    VlessReality,
    Hysteria2,
}

impl CompatTransport {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompatTransport::VlessReality => "vless-reality",
            CompatTransport::Hysteria2 => "hysteria2",
        }
    }
}

/// Public (client-safe) parameters for a compatibility endpoint. Never
/// includes server-private material (REALITY private key, TLS private
/// key) — those live in `RealityServerParams`/`Hysteria2ServerParams`
/// (server-side only, see `store.rs`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicParameters {
    Reality {
        public_key_hex: String,
        short_id: String,
        fingerprint: String,
    },
    Hysteria2 {
        /// Salamander obfuscation password, if enabled. Shared with
        /// clients by design (it is not a per-user secret, it's a
        /// protocol-obfuscation shared value) but still not logged.
        obfs_password: Option<String>,
    },
}

/// Where an endpoint physically lives, and therefore whose credential a
/// user authenticates to it with.
///
/// [`EndpointOrigin::Local`] endpoints run on THIS deployment and use the
/// user's own `vless_uuid`/`hysteria2_password`.
/// [`EndpointOrigin::Peer`] endpoints run on a server this deployment does
/// not control; the credential comes from
/// [`CompatUser::peer_credentials`] and was created independently on that
/// server by its own operator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointOrigin {
    #[default]
    Local,
    Peer,
}

impl EndpointOrigin {
    /// Used by `skip_serializing_if` so a local endpoint serializes
    /// exactly as it did before this field existed. That matters beyond
    /// tidiness: `render::endpoints_fingerprint` hashes this
    /// serialization, and a running `vpn-subscription` compares its
    /// fingerprint against one `vpn-admin` computes. Emitting a new key
    /// for local endpoints would make every zero-peer deployment report a
    /// spurious mismatch across the upgrade.
    pub fn is_local(&self) -> bool {
        matches!(self, EndpointOrigin::Local)
    }
}

/// A single client-facing compatibility listener.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompatEndpoint {
    pub id: String,
    pub transport: CompatTransport,
    pub host: String,
    pub port: u16,
    pub server_name: Option<String>,
    pub label: String,
    pub public_parameters: PublicParameters,

    /// Local unless explicitly declared as a peer. Skipped when local so
    /// a zero-peer deployment's serialization — and therefore its
    /// endpoints fingerprint — is byte-identical to before.
    #[serde(default, skip_serializing_if = "EndpointOrigin::is_local")]
    pub origin: EndpointOrigin,

    /// Operator-declared shared-fate identifier. Local endpoints leave
    /// this absent: they all share this deployment's `public_host`, and
    /// the client derives the domain from the host, which is exactly
    /// right for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_domain: Option<String>,

    /// Opaque operator labels, passed through to the client untouched.
    /// Nothing on the server reads them, and no lookup ever populates
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Optional peer-endpoint id whose per-user credential authenticates
    /// this route. This lets a direct exit and a via-relay alias share the
    /// one credential issued by that exit instead of duplicating secrets.
    /// Local endpoints never use this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

/// A user's credential for ONE peer endpoint.
///
/// The value was created on the peer server by that server's own
/// `vpn-admin` and typed in here by the operator who runs both. This
/// server never generates one: it does not control the peer, so a
/// generated credential could not authenticate there and would produce a
/// confidently-wrong provisioning document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum PeerCredential {
    VlessReality { uuid: String },
    Hysteria2 { password: SecretString },
}

impl PeerCredential {
    /// The transport this credential is valid for. A credential is never
    /// coerced across transports — a `--password` handed to a
    /// `vless-reality` peer is an operator error, not something to
    /// silently reshape.
    pub fn transport(&self) -> CompatTransport {
        match self {
            PeerCredential::VlessReality { .. } => CompatTransport::VlessReality,
            PeerCredential::Hysteria2 { .. } => CompatTransport::Hysteria2,
        }
    }
}

/// Which credential policy a [`CredentialGrant`] is minted under.
///
/// The two classes are the same *shape* with different lifetimes, because
/// they differ only in how often the holder can be expected to come back
/// for a new one — not in what the credential proves. Everything else about
/// them (opaque principal, explicit validity window, immediate revocation,
/// bounded overlap) is identical, so that a mistake in one cannot silently
/// weaken the other.
///
/// See `credential_policy` for the policy each class enforces and for why
/// the numbers are what they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialClass {
    /// Arcana's own client. Silently renewable, short authorization
    /// window, never requires a reconnect to renew.
    Native,
    /// A third-party VPN client (Hiddify, Shadowrocket, INCY, generic
    /// sing-box/Xray importers). Cannot implement renewal logic, so its
    /// window is long and operator-configurable within policy bounds.
    ///
    /// Named `compatibility` in JSON; the client-facing opaque principal
    /// prefix is `ext_` (see [`crate::credentials::generate_principal_id`]).
    Compatibility,
}

impl CredentialClass {
    /// Stable lowercase wire name. Used in policy errors and in the
    /// client capability contract.
    pub fn as_str(&self) -> &'static str {
        match self {
            CredentialClass::Native => "native",
            CredentialClass::Compatibility => "compatibility",
        }
    }

    /// The opaque principal-id prefix for this class. A principal id is
    /// self-describing about its *policy* (so an operator can tell at a
    /// glance in `users.json` which lifetime rule applies) while carrying
    /// no customer identity whatsoever.
    pub fn principal_prefix(&self) -> &'static str {
        match self {
            CredentialClass::Native => "native_",
            CredentialClass::Compatibility => "ext_",
        }
    }
}

/// One time-bounded credential grant for a [`CompatUser`].
///
/// A grant is the unit of authorization: it carries its own opaque
/// principal, its own credential material, and an explicit half-open
/// validity window `[valid_from, valid_until)`.
///
/// # Why this is separate from the account
///
/// A `CompatUser` is an *account* — it may outlive any individual
/// credential by months. Before this type existed, the account doubled as
/// its own single credential, which made the account-level `expires_at`
/// serve two incompatible purposes at once (see `credential_policy`). One
/// account can now hold several grants: one native, one or more
/// compatibility credentials for distinct external devices, and briefly
/// two of either during a controlled rotation overlap.
///
/// # Material stability
///
/// `principal_id`, `vless_uuid` and `hysteria2_password` are fixed for the
/// life of the grant. **Renewal must not change any of them** — only
/// `valid_until`. sing-box has no per-user expiry field, so material is the
/// only thing that can change the rendered document, and changing it forces
/// a sing-box restart that disconnects every client on the node. Holding
/// material constant across renewals is what makes silent renewal free.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialGrant {
    /// Opaque authorization principal, e.g. `native_ab18…` or
    /// `ext_f921…`.
    ///
    /// This is what the data plane authenticates, routes, expires and
    /// troubleshoots against. It deliberately contains **no** customer
    /// email, name, Stripe customer id, Supabase account id, billing
    /// record or subscription token — an operator reading `users.json` or a
    /// `vpn-admin` listing learns only that some principal holds a
    /// credential of a given class, which is everything required to
    /// authenticate it, revoke it, apply route policy to it, expire it, or
    /// diagnose it.
    ///
    /// Distinct from `CompatUser::id`, which identifies the account. Where
    /// an account has exactly one legacy credential the two coincide by
    /// construction (see [`CompatUser::effective_grants`]).
    pub principal_id: String,
    pub class: CredentialClass,
    pub vless_uuid: String,
    pub hysteria2_password: SecretString,
    /// Unix seconds; inclusive. A grant is not authorized before this.
    pub valid_from: i64,
    /// Unix seconds; **exclusive**. A grant is dead at exactly this
    /// instant, so no credential survives past its authorized validity.
    ///
    /// Always present and always finite for an explicitly stored grant.
    /// The "never expires" case is expressed by the legacy account fields,
    /// not by an absent `valid_until`, so that "explicit valid_until" is
    /// true of every stored grant and cannot be forgotten.
    pub valid_until: i64,
    /// Unix seconds at which this grant was revoked, if it was. Absent
    /// means never revoked.
    ///
    /// Revocation is recorded rather than expressed by deletion so that
    /// revocation is *auditable* — an operator can answer "was this
    /// credential ever issued to this principal, and when did it die?" —
    /// and so that a replayed or restored state file cannot resurrect it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<i64>,
    /// Monotonic counter, incremented on every rotation of this (principal,
    /// class). Ordering only; it is not a secret and not an identity.
    ///
    /// Starts at 1 for a grant that has never been rotated, so that
    /// generation 0 can mean "not present" without a sentinel.
    #[serde(default = "first_generation")]
    pub generation: u64,
}

fn first_generation() -> u64 {
    1
}

impl CredentialGrant {
    /// Mint a new grant with freshly generated credential material.
    pub fn new(
        principal_id: String,
        class: CredentialClass,
        vless_uuid: &str,
        valid_from: i64,
        valid_until: i64,
    ) -> Self {
        Self {
            principal_id,
            class,
            vless_uuid: vless_uuid.to_string(),
            hysteria2_password: SecretString::new(crate::credentials::generate_hysteria2_password()),
            valid_from,
            valid_until,
            revoked_at: None,
            generation: 1,
        }
    }

    /// Whether this grant was revoked at or before `now_unix`.
    ///
    /// Revocation is treated as already in force at the recorded instant,
    /// not one second later: an operator who revokes at T means "not
    /// usable at T", and a revoke recorded in the future is treated as
    /// revoked now rather than as a scheduled revocation nobody asked for.
    pub fn is_revoked_at(&self, now_unix: i64) -> bool {
        self.revoked_at.is_some_and(|at| at <= now_unix)
    }

    /// Revoke this grant at `now_unix`. Idempotent — re-revoking keeps the
    /// earlier instant, so the recorded revocation time is the true time
    /// authorization actually ended.
    pub fn revoke(&mut self, now_unix: i64) {
        self.revoked_at = Some(
            self.revoked_at
                .map_or(now_unix, |existing| existing.min(now_unix)),
        );
    }

    /// Extend this grant's authorization to `new_valid_until`.
    ///
    /// Returns `Err` rather than shortening: `valid_until` may only ever
    /// move forward within a grant's life. Shortening is what rotation
    /// does, and it does it by minting a *new* grant and clipping this one,
    /// which keeps the audit trail ("this credential was valid until X")
    /// intact.
    ///
    /// Refuses to extend a grant that has already expired or been revoked.
    /// Renewing an expired credential is precisely the resurrection this
    /// design forbids: it would resurrect a secret that a customer stopped
    /// using, or that leaked, back into a live window.
    pub fn renew_until(&mut self, new_valid_until: i64, now_unix: i64) -> Result<(), String> {
        if self.is_revoked_at(now_unix) {
            return Err(format!(
                "credential {} was revoked and cannot be renewed; mint a new one",
                self.principal_id
            ));
        }
        if now_unix >= self.valid_until {
            return Err(format!(
                "credential {} expired at {} (now {now_unix}) and cannot be renewed; \
                 mint a new one",
                self.principal_id, self.valid_until
            ));
        }
        if new_valid_until < self.valid_until {
            return Err(format!(
                "credential {} cannot be shortened from {} to {new_valid_until}; \
                 mint a superseding credential instead",
                self.principal_id, self.valid_until
            ));
        }
        self.valid_until = new_valid_until;
        Ok(())
    }

    /// Structural validation of a single grant.
    pub fn validate(&self) -> Result<(), String> {
        crate::credentials::validate_principal_id(&self.principal_id, self.class)?;
        if !crate::credentials::is_uuid_v4_shaped(&self.vless_uuid) {
            return Err(format!(
                "credential {} has a VLESS UUID that is not 8-4-4-4-12 hex",
                self.principal_id
            ));
        }
        if self.hysteria2_password.expose().is_empty() {
            return Err(format!(
                "credential {} has an empty Hysteria2 password",
                self.principal_id
            ));
        }
        if self.valid_until <= self.valid_from {
            return Err(format!(
                "credential {} has valid_until {} at or before valid_from {}; every credential \
                 must have a non-empty validity window",
                self.principal_id, self.valid_until, self.valid_from
            ));
        }
        if self.valid_until <= 0 || self.valid_from < 0 {
            return Err(format!(
                "credential {} has a non-positive validity window ({}, {})",
                self.principal_id, self.valid_from, self.valid_until
            ));
        }
        if let Some(revoked_at) = self.revoked_at {
            if revoked_at < self.valid_from {
                return Err(format!(
                    "credential {} was revoked at {revoked_at}, before its window opened at {}",
                    self.principal_id, self.valid_from
                ));
            }
        }
        Ok(())
    }
}

/// A compatibility (third-party-client) user. Persisted in
/// `/etc/vpn/compat/users/users.json`; never mixed into the native
/// `config`/`rendezvous` trust chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompatUser {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub vless_uuid: String,
    pub hysteria2_password: SecretString,
    /// SHA-256 hex digest of the subscription token. The raw token is
    /// never persisted (spec §14) — only shown once at creation/rotation.
    pub subscription_token_hash_hex: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    /// EXPERIMENTAL, opt-in, per-user, default `false`: render THIS
    /// user's VLESS inbound entry with an EMPTY flow instead of
    /// `xtls-rprx-vision` (see `server::render_singbox_server_config`).
    ///
    /// This exists only for the "Vision-off" experiment of
    /// `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.5, and it is
    /// required rather than optional: sing-box's VLESS server validates
    /// the client's requested flow against the configured per-user flow
    /// with a plain inequality (sing-vmess `vless/service.go`:
    /// `else if request.Flow != userFlow { return E.New("flow mismatch:
    /// ...") }`), so a client that omits the flow CANNOT connect to a
    /// user provisioned with `xtls-rprx-vision`. There is no way to
    /// accept both flows for one UUID — the inbound's user map is keyed
    /// by UUID, one flow per entry — so this is deliberately a per-user
    /// toggle: while it is on, that ONE user connects with the
    /// Vision-off client profile (`?compat=vision-off`) and their normal
    /// Vision profile stops working; every other user is untouched.
    ///
    /// Skipped during serialization when `false`, so `users.json` for a
    /// deployment that never runs this experiment stays byte-identical
    /// to what it was before this field existed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision_off_experiment: bool,

    /// Relay-role only, default `false`: this ONE user's traffic to the
    /// Google/YouTube domain set (see `server::GOOGLE_EGRESS_DOMAINS`) is
    /// allowed straight to this relay's own `direct` outbound instead of
    /// being restricted to the declared exit-forwarding + reject-all
    /// policy every other user gets (`server::render_server_config_for_deployment`,
    /// `NodeRole::Relay` branch).
    ///
    /// This is not a client-facing feature: it exists so an EXIT can hold
    /// this one user's credential and hairpin real end-user Google/YouTube
    /// traffic through a relay with materially better network peering to
    /// Google's CDN than the exit's own network — see
    /// `docs/YOUTUBE_FINAL_ROOT_CAUSE.md` §16. A real end user is never
    /// expected to hold this credential.
    ///
    /// Every other user, and every other destination for this one, keeps
    /// the relay's existing fail-closed guarantee unchanged: this flag
    /// only ever ADDS one narrowly-scoped allow rule ahead of the
    /// existing policy, for one user id and one fixed domain set.
    ///
    /// Skipped during serialization when `false`, same treatment as
    /// `vision_off_experiment`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub google_egress_hairpin: bool,

    /// This user's credentials for peer endpoints, keyed by peer endpoint
    /// id (ADR-0009 Option A: per-user, per-endpoint).
    ///
    /// Per-user rather than one shared credential per peer, because a
    /// shared one breaks per-user revocation — disabling a local user
    /// would not revoke their access to the peer — and widens blast
    /// radius: one device compromise would expose a credential that works
    /// for every other user.
    ///
    /// An endpoint absent from this map is one the user cannot
    /// authenticate to, and is omitted from their provisioning document
    /// entirely rather than served as a broken option.
    ///
    /// Skipped when empty, so `users.json` for a deployment that has no
    /// peers stays byte-identical to what it was before this field
    /// existed — the same treatment `vision_off_experiment` gets.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub peer_credentials: BTreeMap<String, PeerCredential>,

    /// STRUCTURAL marker for a reserved, node-local internal probe
    /// principal (spec C-16 "Exceptions"; see
    /// `server::apply_probe_user_confinement`). This is never set by any
    /// customer-facing or operator-facing user-creation path
    /// (`vpn-admin user create` always constructs `false` here) — the
    /// ONLY way to get `true` is `vpn-admin user create-probe`
    /// (`cmd_user_create_probe` in `apps/admin/src/main.rs`), which is
    /// not reachable from customer input.
    ///
    /// Confinement in `server.rs` keys off THIS field, never off
    /// `name == PROBE_USER_NAME`. A customer who happens to name their
    /// account `"arcana-probe"` therefore gets an ordinary customer
    /// record with `is_reserved_probe: false` — no probe confinement,
    /// no probe privilege, no special treatment of any kind from the
    /// name alone. A pre-existing customer of that name is never
    /// silently upgraded by this field appearing: it defaults to
    /// `false` for every already-persisted record (`serde(default)`)
    /// and nothing ever flips it after creation.
    ///
    /// Skipped during serialization when `false`, same treatment as
    /// `vision_off_experiment`/`google_egress_hairpin` — a deployment
    /// that has never provisioned a probe user has a byte-identical
    /// `users.json` to before this field existed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_reserved_probe: bool,

    /// This account's time-bounded credential grants.
    ///
    /// Empty on any deployment created before credential grants existed, in
    /// which case [`CompatUser::effective_grants`] synthesizes a single
    /// grant from the legacy `vless_uuid` / `hysteria2_password` /
    /// `expires_at` fields. That fallback is what makes this field
    /// additive: an existing `users.json` loads unchanged, renders
    /// byte-identically, and needs no migration step to keep working.
    ///
    /// When this is non-empty it is **authoritative**: rendering uses these
    /// grants and ignores the legacy fields. The legacy fields are left in
    /// place rather than removed so that the fleet lease-pool code path,
    /// which writes them directly, keeps composing with this model instead
    /// of having to be rewritten.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<CredentialGrant>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl CompatUser {
    /// Account-level authorization. A disabled or expired account is
    /// excluded from the rendered config entirely.
    ///
    /// Note this is an **account** check, not a credential check: it is the
    /// coarse switch that kills every credential a user holds at once. The
    /// per-credential window is [`CredentialGrant`]'s job, and a user can
    /// be active here while holding no live credential (all lapsed), in
    /// which case they are authorized for nothing.
    pub fn is_active(&self, now_unix: i64) -> bool {
        self.enabled && self.expires_at.map(|exp| now_unix < exp).unwrap_or(true)
    }

    /// This account's credential grants, with the pre-grant model
    /// synthesized where needed.
    ///
    /// Returns `Cow` rather than a reference slice because the legacy path
    /// has to *construct* a grant: before grants existed, the account's own
    /// `vless_uuid` / `hysteria2_password` / `expires_at` were the
    /// credential, so an account with no explicit `credentials` is treated
    /// as holding exactly one grant whose principal is the account's own
    /// `id`.
    ///
    /// Three properties of that fallback are deliberate and load-bearing:
    ///
    /// * The synthesized `principal_id` is the account `id`, so every
    ///   existing `auth_user` route rule, probe confinement rule and
    ///   hairpin exception keeps matching the same identity string it
    ///   matched before.
    /// * `valid_from` is 0 and `valid_until` mirrors `expires_at`, so
    ///   authorization is bit-for-bit what `is_active` already decided.
    ///   In particular `created_at` is deliberately **not** used as
    ///   `valid_from` — it has never been enforced, so honouring it now
    ///   could lock out an existing account whose stored `created_at` is
    ///   wrong or in the future.
    /// * The class is [`CredentialClass::Compatibility`]: a bare legacy
    ///   credential has third-party refresh semantics, which is what it has
    ///   always had.
    ///
    /// The result is that a deployment with no `credentials` field renders
    /// an identical document to one built before this type existed.
    pub fn effective_grants(&self) -> Vec<std::borrow::Cow<'_, CredentialGrant>> {
        if !self.credentials.is_empty() {
            return self
                .credentials
                .iter()
                .map(std::borrow::Cow::Borrowed)
                .collect();
        }
        vec![std::borrow::Cow::Owned(CredentialGrant {
            principal_id: self.id.clone(),
            class: CredentialClass::Compatibility,
            vless_uuid: self.vless_uuid.clone(),
            hysteria2_password: self.hysteria2_password.clone(),
            valid_from: 0,
            // `i64::MAX` rather than `None`: the stored-grant invariant is
            // "valid_until is explicit and finite", and a legacy account
            // that never expires is exactly the account-level `None` case.
            valid_until: self.expires_at.unwrap_or(i64::MAX),
            revoked_at: None,
            generation: 1,
        })]
    }

    /// The principals of this account's credentials that are authorized at
    /// `now_unix`.
    ///
    /// This is the set every `auth_user` route rule, probe confinement
    /// rule and hairpin exception must be scoped to. It is a *set* rather
    /// than a single id because an account may hold several live grants
    /// during an overlap — a rule scoped to only the newest one would stop
    /// matching the older credential mid-rotation and cut a working client
    /// off.
    pub fn active_principal_ids(&self, now_unix: i64) -> Vec<String> {
        if !self.is_active(now_unix) {
            return Vec::new();
        }
        self.effective_grants()
            .into_iter()
            .filter(|g| crate::credential_policy::grant_is_authorized(g, now_unix))
            .map(|g| g.principal_id.clone())
            .collect()
    }

    /// This user's credential for `endpoint_id`, if the operator has set
    /// one. Deliberately returns `None` rather than falling back to the
    /// user's LOCAL credential: the local `vless_uuid` is meaningless on a
    /// server this deployment does not control, and offering it would
    /// produce an endpoint that cannot authenticate.
    pub fn peer_credential(&self, endpoint_id: &str) -> Option<&PeerCredential> {
        self.peer_credentials.get(endpoint_id)
    }

    /// Validate this account's grant set against the overlap policy.
    ///
    /// Called on every store load and every write, so an out-of-policy set
    /// can never reach the render path — which is what makes "a restart
    /// cannot resurrect an expired credential" true by construction.
    pub fn validate_grants(&self) -> Result<(), String> {
        crate::credential_policy::validate_grant_set(&self.credentials)
    }
}

/// Server-side REALITY parameters. `private_key_hex` must never be
/// logged, serialized into a subscription response, or rendered into a
/// client-facing config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealityServerParams {
    pub private_key_hex: SecretString,
    pub public_key_hex: String,
    pub short_ids: Vec<String>,
    /// Disguise/decoy target dialed for the real REALITY handshake
    /// (sing-box `tls.reality.handshake.server`/`server_port`).
    pub handshake_server: String,
    pub handshake_port: u16,

    /// This exit's own credential for the relay's
    /// `google_egress_hairpin` user (see that field's doc comment and
    /// `docs/YOUTUBE_FINAL_ROOT_CAUSE.md` §16) — the UUID an EXIT dials
    /// the relay with so Google/YouTube traffic can egress from the
    /// relay's network instead of the exit's own. `None` (the default)
    /// on every deployment that has not opted into this: the exit then
    /// renders exactly as it always has, with no new outbound and no new
    /// route rule. Grouped with `RealityServerParams` purely so it reaches
    /// `render_server_config_for_deployment` through an existing
    /// parameter instead of widening that function's signature — it is
    /// this exit's own outbound credential, not this exit's own REALITY
    /// server identity, and is never mixed into `private_key_hex`/
    /// `public_key_hex`/`short_ids` above.
    #[serde(default)]
    pub google_egress_hairpin_uuid: Option<SecretString>,
}

/// Server-side Hysteria2 TLS/obfuscation parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hysteria2ServerParams {
    pub tls_cert_path: String,
    pub tls_key_path: String,
    pub obfs_password: Option<SecretString>,
    /// Directory served back to unauthenticated/invalid Hysteria2
    /// connections (sing-box `masquerade` of type `file`), so failed
    /// probes see plausible HTTP content instead of a distinctive
    /// auth-reject signature. `None` disables masquerade (e.g. in tests
    /// that don't care about this behavior).
    pub masquerade_dir_path: Option<String>,
    /// Explicit fixed-rate (Brutal) bandwidth, Mbps, set only when the
    /// operator has measured the VPS's real sustained throughput (`vpn
    /// benchmark`) — see docs/PERFORMANCE_OPTIMIZATION_PLAN.md. `None`
    /// (the default) leaves sing-box's adaptive BBR-based congestion
    /// control in place, which is the only safe default when the real
    /// bandwidth is unknown: an inflated fixed value causes sustained
    /// self-induced congestion instead of the loss-adaptive backoff BBR
    /// provides (see `render_hysteria2_inbound`'s doc comment for the
    /// mechanism). Both fields are set together or not at all — see
    /// `Hysteria2Section::bandwidth` in `deployment.rs`.
    pub up_mbps: Option<u32>,
    pub down_mbps: Option<u32>,
}

impl Default for PublicParameters {
    fn default() -> Self {
        PublicParameters::Reality {
            public_key_hex: String::new(),
            short_id: String::new(),
            fingerprint: String::new(),
        }
    }
}

/// Exists so construction sites can spell only the fields they care
/// about (`..Default::default()`), which keeps adding an optional
/// metadata field from rippling through every call site and test.
impl Default for CompatEndpoint {
    fn default() -> Self {
        CompatEndpoint {
            id: String::new(),
            transport: CompatTransport::VlessReality,
            host: String::new(),
            port: 0,
            server_name: None,
            label: String::new(),
            public_parameters: PublicParameters::default(),
            origin: EndpointOrigin::Local,
            failure_domain: None,
            region: None,
            provider: None,
            asn: None,
            path: None,
            credential_ref: None,
        }
    }
}
