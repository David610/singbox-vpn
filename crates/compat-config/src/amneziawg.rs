//! AmneziaWG transport: keys, obfuscation parameters, address allocation,
//! and rendering of the server `awg setconf` file and client `awg-quick`
//! profiles.
//!
//! Upstream authority (pinned in `deploy/lib/versions.env`):
//! `amneziawg-go` v3.1.20260828 (`device/uapi.go`, `device/obf.go`,
//! README) and `amneziawg-tools` v3.1.20260812 (`src/config.c`). The
//! validation rules below restate what those parsers enforce, plus a few
//! stricter safety rules that are marked as ours. No cryptography is
//! implemented here: keys are X25519 scalars/points produced with the
//! existing `curve25519-dalek` dependency, exactly as `awg genkey` /
//! `awg pubkey` do, and every handshake is performed by upstream code.
//!
//! Parameter ownership, per upstream README:
//!
//! * must match on server and client: `S1`–`S4`, `H1`–`H4`,
//!   `HeaderProtectionKey`;
//! * client-side (may differ): `Jc`/`Jmin`/`Jmax`, `I1`–`I5`;
//! * `ContentPaddingAddition`: upstream recommends setting it on both.

use crate::secret::SecretString;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use curve25519_dalek::montgomery::MontgomeryPoint;
use platform_core::transport::{
    InstallPlan, L4Protocol, PinnedArtifact, PortRule, ProbeSpec, RevocationEffect,
    TransportCapabilities, TransportDescriptor, TransportError, TransportKind, TransportProvider,
};
use platform_core::health::HealthLayer;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// Upstream pins. Kept in sync with `deploy/lib/versions.env` by
/// `install_plan_is_pinned_and_matches_versions_env`.
pub const AMNEZIAWG_GO_VERSION: &str = "v3.1.20260828";
pub const AMNEZIAWG_GO_COMMIT: &str = "b5928efb6ca19f0153958460c3d141f04abc5c2e";
pub const AMNEZIAWG_TOOLS_VERSION: &str = "v3.1.20260812";
pub const AMNEZIAWG_TOOLS_COMMIT: &str = "ee0f0a9aa34ff0a0da4b3433b9512781cfe02843";

/// `HeaderCipherNonceSize` in amneziawg-go: header protection needs every
/// `S1`–`S4` padding to be at least this large.
pub const HEADER_PROTECTION_MIN_PADDING: u16 = 12;
/// Standard WireGuard message types. Our generator keeps header ranges
/// clear of them so obfuscated traffic never carries plain WireGuard
/// type values (a stricter-than-upstream rule).
pub const WIREGUARD_MESSAGE_TYPES: std::ops::RangeInclusive<u32> = 1..=4;
/// Largest junk/signature packet we accept. Upstream warns that packets
/// larger than the path MTU fragment, which is itself a fingerprint; the
/// IPv6 minimum MTU is the conservative bound (ours).
pub const MAX_OBFUSCATION_PACKET: u32 = 1280;

// ---------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------

fn clamp(mut k: [u8; 32]) -> [u8; 32] {
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    k
}

/// A WireGuard-format key: standard base64 of 32 bytes (44 characters).
pub fn validate_key(key: &str) -> Result<[u8; 32], String> {
    let bytes = STANDARD
        .decode(key.trim())
        .map_err(|e| format!("not valid base64: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("decodes to {} bytes; expected 32", v.len()))
}

/// Same derivation as `awg pubkey`.
pub fn derive_public_key(private_key: &str) -> Result<String, String> {
    let private = validate_key(private_key)?;
    Ok(STANDARD.encode(MontgomeryPoint::mul_base_clamped(private).to_bytes()))
}

/// Same as `awg genkey`: 32 CSPRNG bytes, clamped.
pub fn generate_private_key() -> SecretString {
    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    SecretString::new(STANDARD.encode(clamp(k)))
}

/// Same as `awg genpsk` (and the header protection key upstream tells
/// operators to create with `awg genkey`): 32 CSPRNG bytes.
pub fn generate_symmetric_key() -> SecretString {
    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    SecretString::new(STANDARD.encode(k))
}

// ---------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------

/// `a` or `a-b` with `a <= b` (upstream `UintRange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct U32Range {
    pub lo: u32,
    pub hi: u32,
}

impl U32Range {
    pub fn overlaps(&self, other: &U32Range) -> bool {
        self.lo <= other.hi && other.lo <= self.hi
    }
}

impl fmt::Display for U32Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.lo == self.hi {
            write!(f, "{}", self.lo)
        } else {
            write!(f, "{}-{}", self.lo, self.hi)
        }
    }
}

impl FromStr for U32Range {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.trim().split('-').collect();
        let parse = |p: &str| p.parse::<u32>().map_err(|e| format!("{s:?}: {e}"));
        let (lo, hi) = match parts.as_slice() {
            [a] => (parse(a)?, parse(a)?),
            [a, b] => (parse(a)?, parse(b)?),
            _ => return Err(format!("{s:?} is not `a` or `a-b`")),
        };
        if hi < lo {
            return Err(format!("{s:?}: upper bound below lower bound"));
        }
        Ok(U32Range { lo, hi })
    }
}

impl Serialize for U32Range {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for U32Range {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------
// Obfuscation parameters
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AwgProfile {
    /// AmneziaWG 3.x: header protection and content padding enabled.
    /// Handshake-verified against the pinned upstream build.
    #[serde(rename = "awg-3")]
    Awg3,
    /// No AWG-3-only parameters, for clients still on AmneziaWG 2.x.
    /// Interop with a 2.x build is UNVERIFIED.
    #[serde(rename = "awg-2-compatible")]
    Awg2Compatible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwgParams {
    pub profile: AwgProfile,
    pub jc: u16,
    pub jmin: u16,
    pub jmax: u16,
    pub s1: u16,
    pub s2: u16,
    pub s3: u16,
    pub s4: u16,
    pub h1: U32Range,
    pub h2: U32Range,
    pub h3: U32Range,
    pub h4: U32Range,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature_packets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_protection_key: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_padding_addition: Option<U32Range>,
}

impl AwgParams {
    /// Fresh random parameters for `profile`, satisfying [`validate`].
    pub fn generate(profile: AwgProfile) -> Self {
        let mut rng = OsRng;
        let jc = rng.gen_range(4..=8);
        let jmin = rng.gen_range(40..=80);
        let jmax = jmin + rng.gen_range(200..=600);
        let s1 = rng.gen_range(15..=150);
        let mut s2 = rng.gen_range(15..=150);
        // AWG 1.x could only tell init from response by size; keep them
        // distinct even though 3.x no longer relies on it.
        if s1 + 56 == s2 {
            s2 += 1;
        }
        let s3 = rng.gen_range(15..=60);
        let s4 = rng.gen_range(12..=24);

        // Eight distinct boundaries well above the WireGuard type values,
        // sorted and paired into four disjoint ranges, then shuffled so the
        // ordering of H1..H4 leaks nothing.
        let mut bounds = std::collections::BTreeSet::new();
        while bounds.len() < 8 {
            bounds.insert(rng.gen_range(0x0001_0000u32..=0xFFFF_FFF0));
        }
        let b: Vec<u32> = bounds.into_iter().collect();
        let mut ranges: Vec<U32Range> =
            b.chunks(2).map(|p| U32Range { lo: p[0], hi: p[1] }).collect();
        for i in (1..ranges.len()).rev() {
            let j = rng.gen_range(0..=i);
            ranges.swap(i, j);
        }

        let (header_protection_key, content_padding_addition) = match profile {
            AwgProfile::Awg3 => (
                Some(generate_symmetric_key()),
                Some(U32Range { lo: 0, hi: rng.gen_range(8..=32) }),
            ),
            AwgProfile::Awg2Compatible => (None, None),
        };

        AwgParams {
            profile,
            jc,
            jmin,
            jmax,
            s1,
            s2,
            s3,
            s4,
            h1: ranges[0],
            h2: ranges[1],
            h3: ranges[2],
            h4: ranges[3],
            signature_packets: Vec::new(),
            header_protection_key,
            content_padding_addition,
        }
    }

    pub fn headers(&self) -> [U32Range; 4] {
        [self.h1, self.h2, self.h3, self.h4]
    }

    pub fn paddings(&self) -> [u16; 4] {
        [self.s1, self.s2, self.s3, self.s4]
    }

    pub fn validate(&self) -> Result<(), TransportError> {
        let bad = |m: String| Err(TransportError::InvalidConfig(format!("amneziawg: {m}")));
        if self.jc > 128 {
            return bad(format!("Jc {} exceeds 128", self.jc));
        }
        if self.jmin > self.jmax {
            return bad("Jmin must be <= Jmax".into());
        }
        if u32::from(self.jmax) > MAX_OBFUSCATION_PACKET {
            return bad(format!("Jmax must be <= {MAX_OBFUSCATION_PACKET} to avoid fragmentation"));
        }
        let headers = self.headers();
        for i in 0..4 {
            for j in i + 1..4 {
                if headers[i].overlaps(&headers[j]) {
                    return bad(format!("H{} and H{} overlap (upstream: headers must not overlap)", i + 1, j + 1));
                }
            }
            let wireguard_types = U32Range { lo: *WIREGUARD_MESSAGE_TYPES.start(), hi: *WIREGUARD_MESSAGE_TYPES.end() };
            if headers[i].overlaps(&wireguard_types) {
                return bad(format!("H{} includes a plain WireGuard message type (1-4)", i + 1));
            }
        }
        match (self.profile, &self.header_protection_key) {
            (AwgProfile::Awg3, Some(key)) => {
                validate_key(key.expose())
                    .map_err(|e| TransportError::InvalidConfig(format!("amneziawg: HeaderProtectionKey {e}")))?;
                if self.paddings().iter().any(|s| *s < HEADER_PROTECTION_MIN_PADDING) {
                    return bad(format!(
                        "header protection requires S1-S4 >= {HEADER_PROTECTION_MIN_PADDING}"
                    ));
                }
            }
            (AwgProfile::Awg3, None) => return bad("the awg-3 profile requires a HeaderProtectionKey".into()),
            (AwgProfile::Awg2Compatible, Some(_)) => {
                return bad("HeaderProtectionKey is an AWG 3 parameter; use the awg-3 profile".into())
            }
            (AwgProfile::Awg2Compatible, None) => {}
        }
        if self.profile == AwgProfile::Awg2Compatible && self.content_padding_addition.is_some() {
            return bad("ContentPaddingAddition is an AWG 3 parameter".into());
        }
        if let Some(cpa) = self.content_padding_addition {
            if cpa.hi > u32::from(u16::MAX) {
                return bad("ContentPaddingAddition must fit in 16 bits (awg tools parse it as u16)".into());
            }
        }
        if self.signature_packets.len() > 5 {
            return bad("at most five signature packets (I1-I5)".into());
        }
        for (i, spec) in self.signature_packets.iter().enumerate() {
            let len = validate_signature_packet(spec)
                .map_err(|e| TransportError::InvalidConfig(format!("amneziawg: I{}: {e}", i + 1)))?;
            if len > MAX_OBFUSCATION_PACKET {
                return bad(format!("I{} is {len} bytes; must be <= {MAX_OBFUSCATION_PACKET}", i + 1));
            }
        }
        Ok(())
    }
}

/// Validate an `I1`–`I5` signature-packet spec and return its size in
/// bytes. Only the documented tags are accepted: `<b 0x..>`, `<r n>`,
/// `<rd n>`, `<rc n>`, `<t>` (upstream also parses internal data tags,
/// which have no meaning in a signature packet and are refused here).
pub fn validate_signature_packet(spec: &str) -> Result<u32, String> {
    if spec.contains('\n') || spec.contains('\r') {
        return Err("must be a single line".into());
    }
    let mut rest = spec;
    let mut total: u32 = 0;
    let mut tags = 0;
    while let Some(start) = rest.find('<') {
        if !rest[..start].trim().is_empty() {
            return Err(format!("unexpected text {:?} outside a tag", rest[..start].trim()));
        }
        let end = rest[start..].find('>').ok_or("missing closing >")? + start;
        let tag = &rest[start + 1..end];
        let mut parts = tag.split_whitespace();
        let key = parts.next().ok_or("empty tag")?;
        let arg = parts.next();
        if parts.next().is_some() {
            return Err(format!("tag <{tag}> has too many arguments"));
        }
        let len = match (key, arg) {
            ("b", Some(hex)) => {
                let hex = hex.strip_prefix("0x").unwrap_or(hex);
                if hex.is_empty() || hex.len() % 2 != 0 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("<b {hex}> needs a non-empty even-length hex sequence"));
                }
                (hex.len() / 2) as u32
            }
            ("r" | "rd" | "rc", Some(n)) => {
                let n: u32 = n.parse().map_err(|_| format!("<{key} {n}> needs a positive integer"))?;
                if n == 0 {
                    return Err(format!("<{key} 0> is empty"));
                }
                n
            }
            ("t", None) => 4,
            (other, _) => return Err(format!("unsupported tag <{other}>")),
        };
        total = total.saturating_add(len);
        tags += 1;
        rest = &rest[end + 1..];
    }
    if !rest.trim().is_empty() {
        return Err(format!("unexpected trailing text {:?}", rest.trim()));
    }
    if tags == 0 {
        return Err("no tags".into());
    }
    Ok(total)
}

// ---------------------------------------------------------------------
// Addressing
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv4Net {
    pub addr: Ipv4Addr,
    pub prefix: u8,
}

impl FromStr for Ipv4Net {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (a, p) = s.split_once('/').ok_or_else(|| format!("{s:?} is not a.b.c.d/prefix"))?;
        let addr: Ipv4Addr = a.parse().map_err(|e| format!("{s:?}: {e}"))?;
        let prefix: u8 = p.parse().map_err(|e| format!("{s:?}: {e}"))?;
        if !(8..=30).contains(&prefix) {
            return Err(format!("{s:?}: IPv4 prefix must be /8../30"));
        }
        let mask = u32::MAX << (32 - prefix);
        if u32::from(addr) & !mask != 0 {
            return Err(format!("{s:?}: host bits set"));
        }
        if !addr.is_private() {
            return Err(format!("{s:?}: tunnel subnet must be RFC 1918 private space"));
        }
        Ok(Ipv4Net { addr, prefix })
    }
}

impl fmt::Display for Ipv4Net {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl Ipv4Net {
    fn host(&self, index: u32) -> Option<Ipv4Addr> {
        let size = 1u64 << (32 - self.prefix);
        // Exclude network (0) and broadcast (size - 1).
        (u64::from(index) >= 1 && u64::from(index) < size - 1)
            .then(|| Ipv4Addr::from(u32::from(self.addr) + index))
    }

    fn index_of(&self, ip: Ipv4Addr) -> Option<u32> {
        let offset = u32::from(ip).checked_sub(u32::from(self.addr))?;
        (u64::from(offset) < 1u64 << (32 - self.prefix)).then_some(offset)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv6Net {
    pub addr: Ipv6Addr,
    pub prefix: u8,
}

impl FromStr for Ipv6Net {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (a, p) = s.split_once('/').ok_or_else(|| format!("{s:?} is not addr/prefix"))?;
        let addr: Ipv6Addr = a.parse().map_err(|e| format!("{s:?}: {e}"))?;
        let prefix: u8 = p.parse().map_err(|e| format!("{s:?}: {e}"))?;
        if !(48..=112).contains(&prefix) {
            return Err(format!("{s:?}: IPv6 prefix must be /48../112"));
        }
        if (addr.segments()[0] & 0xfe00) != 0xfc00 {
            return Err(format!("{s:?}: tunnel subnet must be unique-local (fc00::/7)"));
        }
        let bits = u128::from(addr);
        if bits & (u128::MAX >> prefix) != 0 {
            return Err(format!("{s:?}: host bits set"));
        }
        Ok(Ipv6Net { addr, prefix })
    }
}

impl fmt::Display for Ipv6Net {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl Ipv6Net {
    fn host(&self, index: u32) -> Ipv6Addr {
        Ipv6Addr::from(u128::from(self.addr) + u128::from(index))
    }
}

// ---------------------------------------------------------------------
// Node configuration and credentials
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwgNodeConfig {
    pub interface: String,
    pub public_host: String,
    pub listen_port: u16,
    pub subnet_v4: Ipv4Net,
    pub subnet_v6: Option<Ipv6Net>,
    pub mtu: u16,
    pub persistent_keepalive: u16,
    /// Resolvers written only into fallback `awg-quick` profiles, which
    /// otherwise use the device resolver outside the tunnel. First-party
    /// clients own DNS policy and never receive this.
    pub fallback_client_dns: Vec<String>,
    /// Present only in `vpn-admin` (root). The subscription service loads
    /// the node configuration without it, so it can render client profiles
    /// but never a server configuration.
    pub server_private_key: Option<SecretString>,
    pub server_public_key: String,
    pub params: AwgParams,
}

/// One user's AmneziaWG credential on one node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwgCredential {
    /// The client's private key. Generated server-side in self-hosted mode
    /// so a bearer-authenticated profile can be served; same trust class
    /// as a VLESS UUID.
    pub private_key: SecretString,
    pub public_key: String,
    pub preshared_key: SecretString,
    pub address_v4: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_v6: Option<Ipv6Addr>,
}

impl AwgNodeConfig {
    pub fn server_address_v4(&self) -> Ipv4Addr {
        self.subnet_v4.host(1).expect("validated prefix has host 1")
    }

    pub fn server_address_v6(&self) -> Option<Ipv6Addr> {
        self.subnet_v6.map(|n| n.host(1))
    }

    fn validate_config(&self) -> Result<(), TransportError> {
        let bad = |m: &str| Err(TransportError::InvalidConfig(format!("amneziawg: {m}")));
        if self.listen_port == 0 {
            return bad("listen_port must be non-zero");
        }
        if self.interface.is_empty()
            || self.interface.len() > 15
            || !self.interface.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return bad("interface must be 1-15 characters of [A-Za-z0-9_.-]");
        }
        if !(1280..=1500).contains(&self.mtu) {
            return bad("mtu must be 1280..=1500");
        }
        if self.public_host.trim().is_empty() || self.public_host.contains(char::is_whitespace) {
            return bad("public_host is not dialable");
        }
        validate_key(&self.server_public_key)
            .map_err(|e| TransportError::InvalidConfig(format!("amneziawg: server public key {e}")))?;
        if let Some(private) = &self.server_private_key {
            let derived = derive_public_key(private.expose())
                .map_err(|e| TransportError::InvalidConfig(format!("amneziawg: server private key {e}")))?;
            if derived != self.server_public_key {
                return bad("server public key does not match the private key");
            }
        }
        for dns in &self.fallback_client_dns {
            if dns.parse::<std::net::IpAddr>().is_err() {
                return bad("fallback_client_dns entries must be IP literals");
            }
        }
        self.params.validate()
    }
}

fn used_indexes(config: &AwgNodeConfig, existing: &[(&str, &AwgCredential)]) -> std::collections::BTreeSet<u32> {
    let mut used: std::collections::BTreeSet<u32> = existing
        .iter()
        .filter_map(|(_, c)| config.subnet_v4.index_of(c.address_v4))
        .collect();
    used.insert(1); // server
    used
}

fn check_no_reuse(existing: &[(&str, &AwgCredential)]) -> Result<(), TransportError> {
    let mut keys = std::collections::BTreeSet::new();
    let mut addrs = std::collections::BTreeSet::new();
    for (user, c) in existing {
        if !keys.insert(c.public_key.as_str()) {
            return Err(TransportError::CredentialReuse(format!("public key of {user} is already used")));
        }
        if !addrs.insert(c.address_v4) {
            return Err(TransportError::CredentialReuse(format!("address of {user} is already used")));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

fn push_obfuscation(out: &mut String, p: &AwgParams, client: bool) {
    out.push_str(&format!("Jc = {}\nJmin = {}\nJmax = {}\n", p.jc, p.jmin, p.jmax));
    out.push_str(&format!("S1 = {}\nS2 = {}\nS3 = {}\nS4 = {}\n", p.s1, p.s2, p.s3, p.s4));
    out.push_str(&format!("H1 = {}\nH2 = {}\nH3 = {}\nH4 = {}\n", p.h1, p.h2, p.h3, p.h4));
    if client {
        for (i, spec) in p.signature_packets.iter().enumerate() {
            out.push_str(&format!("I{} = {spec}\n", i + 1));
        }
    }
    if let Some(k) = &p.header_protection_key {
        out.push_str(&format!("HeaderProtectionKey = {}\n", k.expose()));
    }
    if let Some(c) = p.content_padding_addition {
        out.push_str(&format!("ContentPaddingAddition = {c}\n"));
    }
}

/// The server file consumed by `awg setconf <iface> <file>`. Contains the
/// server private key: written root-only, never logged, never served.
pub fn render_server_setconf(config: &AwgNodeConfig, active: &[(&str, &AwgCredential)]) -> Result<String, TransportError> {
    config.validate_config()?;
    check_no_reuse(active)?;
    let private = config.server_private_key.as_ref().ok_or_else(|| {
        TransportError::InvalidConfig(
            "amneziawg: the server private key is required to render the server configuration".into(),
        )
    })?;
    let mut out = String::from("[Interface]\n");
    out.push_str(&format!("PrivateKey = {}\n", private.expose()));
    out.push_str(&format!("ListenPort = {}\n", config.listen_port));
    push_obfuscation(&mut out, &config.params, false);
    for (user_id, c) in active {
        if config.subnet_v4.index_of(c.address_v4).is_none() {
            return Err(TransportError::InvalidCredential(format!("{user_id}: address outside the tunnel subnet")));
        }
        out.push_str(&format!("\n# {}\n[Peer]\n", sanitize_comment(user_id)));
        out.push_str(&format!("PublicKey = {}\n", c.public_key));
        out.push_str(&format!("PresharedKey = {}\n", c.preshared_key.expose()));
        let mut allowed = format!("{}/32", c.address_v4);
        if let Some(v6) = c.address_v6 {
            allowed.push_str(&format!(", {v6}/128"));
        }
        out.push_str(&format!("AllowedIPs = {allowed}\n"));
    }
    Ok(out)
}

fn sanitize_comment(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).collect()
}

fn endpoint_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// A complete `awg-quick` profile for fallback clients (AmneziaVPN /
/// AmneziaWG apps). Contains the client private key: a credential.
pub fn render_client_awg_quick(config: &AwgNodeConfig, credential: &AwgCredential) -> Result<String, TransportError> {
    config.validate_config()?;
    let mut out = String::from("[Interface]\n");
    out.push_str(&format!("PrivateKey = {}\n", credential.private_key.expose()));
    let mut addresses = format!("{}/32", credential.address_v4);
    if let Some(v6) = credential.address_v6 {
        addresses.push_str(&format!(", {v6}/128"));
    }
    out.push_str(&format!("Address = {addresses}\n"));
    if !config.fallback_client_dns.is_empty() {
        out.push_str(&format!("DNS = {}\n", config.fallback_client_dns.join(", ")));
    }
    out.push_str(&format!("MTU = {}\n", config.mtu));
    push_obfuscation(&mut out, &config.params, true);
    out.push_str("\n[Peer]\n");
    out.push_str(&format!("PublicKey = {}\n", config.server_public_key));
    out.push_str(&format!("PresharedKey = {}\n", credential.preshared_key.expose()));
    let allowed = if config.subnet_v6.is_some() { "0.0.0.0/0, ::/0" } else { "0.0.0.0/0" };
    out.push_str(&format!("AllowedIPs = {allowed}\n"));
    out.push_str(&format!("Endpoint = {}:{}\n", endpoint_host(&config.public_host), config.listen_port));
    if config.persistent_keepalive > 0 {
        out.push_str(&format!("PersistentKeepalive = {}\n", config.persistent_keepalive));
    }
    Ok(out)
}

/// First-party (v2 contract) endpoint parameters for one user. Carries no
/// DNS/MTU/route policy beyond what the tunnel itself needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwgClientEndpoint {
    pub protocol: AwgProfile,
    pub server_public_key: String,
    pub client_private_key: String,
    pub preshared_key: String,
    pub addresses: Vec<String>,
    pub mtu: u16,
    pub persistent_keepalive: u16,
    pub jc: u16,
    pub jmin: u16,
    pub jmax: u16,
    pub s1: u16,
    pub s2: u16,
    pub s3: u16,
    pub s4: u16,
    pub h1: String,
    pub h2: String,
    pub h3: String,
    pub h4: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature_packets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_protection_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_padding_addition: Option<String>,
}

// ---------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct AmneziaWgProvider;

impl TransportDescriptor for AmneziaWgProvider {
    fn kind(&self) -> TransportKind {
        TransportKind::AmneziaWg
    }

    fn capabilities(&self) -> TransportCapabilities {
        TransportKind::AmneziaWg.capabilities().expect("known transport")
    }

    fn health_probes(&self) -> Vec<ProbeSpec> {
        vec![
            ProbeSpec { layer: HealthLayer::Handshake, name: "awg-latest-handshake-advanced".into(), requires_privilege: true },
            ProbeSpec { layer: HealthLayer::Internet, name: "https-204-through-tunnel".into(), requires_privilege: true },
            ProbeSpec { layer: HealthLayer::Transfer, name: "bounded-download-through-tunnel".into(), requires_privilege: true },
        ]
    }
}

impl TransportProvider for AmneziaWgProvider {
    type NodeConfig = AwgNodeConfig;
    type Credential = AwgCredential;
    type ServerArtifact = String;
    type ClientEndpoint = AwgClientEndpoint;

    fn validate(&self, config: &AwgNodeConfig) -> Result<(), TransportError> {
        config.validate_config()
    }

    fn issue_credentials(
        &self,
        config: &AwgNodeConfig,
        user_id: &str,
        existing: &[(&str, &AwgCredential)],
    ) -> Result<AwgCredential, TransportError> {
        config.validate_config()?;
        check_no_reuse(existing)?;
        if existing.iter().any(|(u, _)| *u == user_id) {
            return Err(TransportError::CredentialReuse(format!("{user_id} already has a credential")));
        }
        let used = used_indexes(config, existing);
        let size = 1u64 << (32 - config.subnet_v4.prefix);
        let index = (2..(size - 1) as u32)
            .find(|i| !used.contains(i))
            .ok_or(TransportError::AddressPoolExhausted)?;
        let private_key = generate_private_key();
        let public_key = derive_public_key(private_key.expose()).expect("generated key is valid");
        Ok(AwgCredential {
            private_key,
            public_key,
            preshared_key: generate_symmetric_key(),
            address_v4: config.subnet_v4.host(index).expect("index in range"),
            address_v6: config.subnet_v6.map(|n| n.host(index)),
        })
    }

    fn rotate_credentials(
        &self,
        config: &AwgNodeConfig,
        _user_id: &str,
        current: &AwgCredential,
        _existing: &[(&str, &AwgCredential)],
    ) -> Result<AwgCredential, TransportError> {
        config.validate_config()?;
        let private_key = generate_private_key();
        let public_key = derive_public_key(private_key.expose()).expect("generated key is valid");
        // Addresses are stable across rotation; key material is not.
        Ok(AwgCredential {
            private_key,
            public_key,
            preshared_key: generate_symmetric_key(),
            address_v4: current.address_v4,
            address_v6: current.address_v6,
        })
    }

    fn revocation_effect(&self) -> RevocationEffect {
        RevocationEffect::OnApply
    }

    fn render_server_artifact(
        &self,
        config: &AwgNodeConfig,
        active: &[(&str, &AwgCredential)],
    ) -> Result<String, TransportError> {
        render_server_setconf(config, active)
    }

    fn render_client_endpoint(
        &self,
        config: &AwgNodeConfig,
        _user_id: &str,
        c: &AwgCredential,
    ) -> Result<AwgClientEndpoint, TransportError> {
        config.validate_config()?;
        let p = &config.params;
        let mut addresses = vec![format!("{}/32", c.address_v4)];
        if let Some(v6) = c.address_v6 {
            addresses.push(format!("{v6}/128"));
        }
        Ok(AwgClientEndpoint {
            protocol: p.profile,
            server_public_key: config.server_public_key.clone(),
            client_private_key: c.private_key.expose().to_string(),
            preshared_key: c.preshared_key.expose().to_string(),
            addresses,
            mtu: config.mtu,
            persistent_keepalive: config.persistent_keepalive,
            jc: p.jc,
            jmin: p.jmin,
            jmax: p.jmax,
            s1: p.s1,
            s2: p.s2,
            s3: p.s3,
            s4: p.s4,
            h1: p.h1.to_string(),
            h2: p.h2.to_string(),
            h3: p.h3.to_string(),
            h4: p.h4.to_string(),
            signature_packets: p.signature_packets.clone(),
            header_protection_key: p.header_protection_key.as_ref().map(|k| k.expose().to_string()),
            content_padding_addition: p.content_padding_addition.map(|r| r.to_string()),
        })
    }

    fn install_plan(&self, config: &AwgNodeConfig) -> InstallPlan {
        InstallPlan {
            transport: TransportKind::AmneziaWg,
            artifacts: vec![
                PinnedArtifact {
                    name: "amneziawg-go".into(),
                    version: AMNEZIAWG_GO_VERSION.into(),
                    sha256: None,
                    git_commit: Some(AMNEZIAWG_GO_COMMIT.into()),
                    license: "MIT".into(),
                },
                PinnedArtifact {
                    name: "amneziawg-tools".into(),
                    version: AMNEZIAWG_TOOLS_VERSION.into(),
                    sha256: None,
                    git_commit: Some(AMNEZIAWG_TOOLS_COMMIT.into()),
                    license: "GPL-2.0".into(),
                },
            ],
            systemd_units: vec!["vpn-amneziawg.service".into()],
            firewall: vec![PortRule { l4: L4Protocol::Udp, port: config.listen_port }],
            sysctls: vec![
                ("net.ipv4.ip_forward".into(), "1".into()),
                ("net.ipv6.conf.all.forwarding".into(), if config.subnet_v6.is_some() { "1" } else { "0" }.into()),
            ],
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn node_config() -> AwgNodeConfig {
        let server_private_key = generate_private_key();
        let server_public_key = derive_public_key(server_private_key.expose()).unwrap();
        AwgNodeConfig {
            interface: "awg0".into(),
            public_host: "vpn.example.com".into(),
            listen_port: 51820,
            subnet_v4: "10.66.0.0/24".parse().unwrap(),
            subnet_v6: Some("fd66:0:0:1::/64".parse().unwrap()),
            mtu: 1380,
            persistent_keepalive: 25,
            fallback_client_dns: vec!["1.1.1.1".into()],
            server_private_key: Some(server_private_key),
            server_public_key,
            params: AwgParams::generate(AwgProfile::Awg3),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::node_config;
    use super::*;

    #[test]
    fn keys_match_the_wireguard_format_and_known_vector() {
        // RFC 7748 §6.1 Alice's keypair, expressed in WireGuard base64.
        let alice_private = STANDARD.encode(hex::decode("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a").unwrap());
        let alice_public = STANDARD.encode(hex::decode("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a").unwrap());
        assert_eq!(derive_public_key(&alice_private).unwrap(), alice_public);
        let k = generate_private_key();
        assert_eq!(k.expose().len(), 44);
        let bytes = validate_key(k.expose()).unwrap();
        assert_eq!(bytes[0] & 7, 0);
        assert_eq!(bytes[31] & 0xc0, 0x40);
        assert!(validate_key("not-a-key").is_err());
        assert!(validate_key(&STANDARD.encode([0u8; 31])).is_err());
    }

    #[test]
    fn generated_params_always_validate() {
        for profile in [AwgProfile::Awg3, AwgProfile::Awg2Compatible] {
            for _ in 0..500 {
                let p = AwgParams::generate(profile);
                p.validate().unwrap_or_else(|e| panic!("{profile:?}: {e}: {p:?}"));
            }
        }
    }

    #[test]
    fn parameter_rules() {
        let base = AwgParams::generate(AwgProfile::Awg3);
        let invalid = |f: &dyn Fn(&mut AwgParams)| {
            let mut p = base.clone();
            f(&mut p);
            p.validate().is_err()
        };
        assert!(invalid(&|p| p.h2 = p.h1));
        assert!(invalid(&|p| p.h3 = U32Range { lo: 3, hi: 9 }));
        assert!(invalid(&|p| p.h4 = U32Range { lo: 0, hi: 100 }));
        assert!(invalid(&|p| p.s3 = 11));
        assert!(invalid(&|p| p.header_protection_key = None));
        assert!(invalid(&|p| p.jmin = p.jmax + 1));
        assert!(invalid(&|p| p.jmax = 2000));
        assert!(invalid(&|p| p.jc = 129));
        assert!(invalid(&|p| p.header_protection_key = Some(SecretString::new("short"))));
        assert!(invalid(&|p| p.signature_packets = vec!["<d>".into()]));
        assert!(invalid(&|p| p.signature_packets = vec!["<r 2000>".into()]));
        assert!(invalid(&|p| p.signature_packets = vec!["<t>".into(); 6]));

        let mut compat = AwgParams::generate(AwgProfile::Awg2Compatible);
        compat.validate().unwrap();
        compat.header_protection_key = Some(generate_symmetric_key());
        assert!(compat.validate().is_err());
    }

    #[test]
    fn signature_packet_syntax_matches_upstream_tags() {
        assert_eq!(validate_signature_packet("<b 0xc0ffee><r 16><rd 4><rc 8><t>"), Ok(3 + 16 + 4 + 8 + 4));
        assert_eq!(validate_signature_packet("<b c0ffee>"), Ok(3));
        for bad in ["", "<b 0xabc>", "<r x>", "<r 0>", "<q 1>", "<b>", "junk<t>", "<t> junk", "<r 1 2>", "<t", "<t>\n<t>"] {
            assert!(validate_signature_packet(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ranges_round_trip() {
        for s in ["5", "100-200"] {
            assert_eq!(s.parse::<U32Range>().unwrap().to_string(), s);
        }
        assert!("9-1".parse::<U32Range>().is_err());
        assert!("1-2-3".parse::<U32Range>().is_err());
        assert!("-1".parse::<U32Range>().is_err());
    }

    #[test]
    fn subnets_are_validated() {
        assert!("10.66.0.0/24".parse::<Ipv4Net>().is_ok());
        assert!("10.66.0.1/24".parse::<Ipv4Net>().is_err());
        assert!("8.8.8.0/24".parse::<Ipv4Net>().is_err());
        assert!("10.0.0.0/31".parse::<Ipv4Net>().is_err());
        assert!("fd66::/64".parse::<Ipv6Net>().is_ok());
        assert!("2001:db8::/64".parse::<Ipv6Net>().is_err());
        assert!("fd66::1/64".parse::<Ipv6Net>().is_err());
    }

    #[test]
    fn allocation_is_sequential_unique_and_bounded() {
        let mut cfg = node_config();
        cfg.subnet_v4 = "10.66.0.0/29".parse().unwrap(); // hosts .1-.6, server .1
        let p = AmneziaWgProvider;
        let mut issued: Vec<(String, AwgCredential)> = Vec::new();
        for i in 0..5 {
            let existing: Vec<(&str, &AwgCredential)> = issued.iter().map(|(u, c)| (u.as_str(), c)).collect();
            let c = p.issue_credentials(&cfg, &format!("u{i}"), &existing).unwrap();
            assert_eq!(c.address_v4, Ipv4Addr::new(10, 66, 0, 2 + i));
            assert_eq!(c.address_v6.unwrap().segments()[7], 2 + i as u16);
            issued.push((format!("u{i}"), c));
        }
        let existing: Vec<(&str, &AwgCredential)> = issued.iter().map(|(u, c)| (u.as_str(), c)).collect();
        assert_eq!(p.issue_credentials(&cfg, "u9", &existing), Err(TransportError::AddressPoolExhausted));

        // A revoked user's address is reused only after it is gone.
        issued.remove(1);
        let existing: Vec<(&str, &AwgCredential)> = issued.iter().map(|(u, c)| (u.as_str(), c)).collect();
        assert_eq!(p.issue_credentials(&cfg, "u9", &existing).unwrap().address_v4, Ipv4Addr::new(10, 66, 0, 3));
        assert!(matches!(p.issue_credentials(&cfg, "u0", &existing), Err(TransportError::CredentialReuse(_))));
    }

    #[test]
    fn rotation_keeps_addresses_and_changes_all_key_material() {
        let cfg = node_config();
        let p = AmneziaWgProvider;
        let c = p.issue_credentials(&cfg, "u", &[]).unwrap();
        let r = p.rotate_credentials(&cfg, "u", &c, &[]).unwrap();
        assert_eq!((r.address_v4, r.address_v6), (c.address_v4, c.address_v6));
        assert_ne!(r.private_key, c.private_key);
        assert_ne!(r.public_key, c.public_key);
        assert_ne!(r.preshared_key, c.preshared_key);
    }

    #[test]
    fn server_render_excludes_revoked_users_and_client_secrets() {
        let cfg = node_config();
        let p = AmneziaWgProvider;
        let a = p.issue_credentials(&cfg, "alice", &[]).unwrap();
        let b = p.issue_credentials(&cfg, "bob", &[("alice", &a)]).unwrap();
        let text = p.render_server_artifact(&cfg, &[("alice", &a)]).unwrap();
        assert!(text.contains(&a.public_key));
        assert!(!text.contains(&b.public_key), "revoked peer rendered");
        assert!(!text.contains(a.private_key.expose()), "client private key on server");
        assert!(text.contains("HeaderProtectionKey = "));
        assert!(!text.contains("\nI1 = "), "client-side I packets are not server config");
        assert!(text.contains("AllowedIPs = 10.66.0.2/32, fd66:0:0:1::2/128"));
        assert!(matches!(
            p.render_server_artifact(&cfg, &[("alice", &a), ("mallory", &a)]),
            Err(TransportError::CredentialReuse(_))
        ));
    }

    #[test]
    fn client_profile_carries_matching_parameters_and_no_server_secret() {
        let mut cfg = node_config();
        cfg.params.signature_packets = vec!["<b 0xc0ffee><r 32>".into()];
        let p = AmneziaWgProvider;
        let c = p.issue_credentials(&cfg, "alice", &[]).unwrap();
        let ini = render_client_awg_quick(&cfg, &c).unwrap();
        let server = render_server_setconf(&cfg, &[("alice", &c)]).unwrap();
        assert!(!ini.contains(cfg.server_private_key.as_ref().unwrap().expose()));
        for key in ["S1", "S2", "S3", "S4", "H1", "H2", "H3", "H4", "HeaderProtectionKey", "ContentPaddingAddition"] {
            let line = |t: &str| t.lines().find(|l| l.starts_with(&format!("{key} = "))).map(str::to_string);
            assert_eq!(line(&ini), line(&server), "{key} must match on both sides");
        }
        assert!(ini.contains("I1 = <b 0xc0ffee><r 32>"));
        assert!(ini.contains("Endpoint = vpn.example.com:51820"));
        assert!(ini.contains("AllowedIPs = 0.0.0.0/0, ::/0"));

        let ep = p.render_client_endpoint(&cfg, "alice", &c).unwrap();
        let json = serde_json::to_string(&ep).unwrap();
        assert!(!json.contains(cfg.server_private_key.as_ref().unwrap().expose()));
        assert!(!json.contains("DNS") && !json.contains("dns"), "first-party endpoint must not carry DNS policy");
    }

    #[test]
    fn public_only_config_renders_clients_but_never_the_server() {
        let mut cfg = node_config();
        let c = AmneziaWgProvider.issue_credentials(&cfg, "u", &[]).unwrap();
        cfg.server_private_key = None;
        assert!(render_client_awg_quick(&cfg, &c).is_ok());
        assert!(AmneziaWgProvider.render_client_endpoint(&cfg, "u", &c).is_ok());
        assert!(render_server_setconf(&cfg, &[("u", &c)]).is_err());
    }

    #[test]
    fn ipv6_endpoints_are_bracketed() {
        let mut cfg = node_config();
        cfg.public_host = "2001:db8::10".into();
        let c = AmneziaWgProvider.issue_credentials(&cfg, "u", &[]).unwrap();
        assert!(render_client_awg_quick(&cfg, &c).unwrap().contains("Endpoint = [2001:db8::10]:51820"));
    }

    #[test]
    fn node_config_rejects_split_keypair_and_bad_values() {
        let mut cfg = node_config();
        cfg.server_public_key = derive_public_key(generate_private_key().expose()).unwrap();
        assert!(AmneziaWgProvider.validate(&cfg).is_err());
        let mut cfg = node_config();
        cfg.interface = "a-very-long-interface".into();
        assert!(AmneziaWgProvider.validate(&cfg).is_err());
        let mut cfg = node_config();
        cfg.fallback_client_dns = vec!["dns.example".into()];
        assert!(AmneziaWgProvider.validate(&cfg).is_err());
    }

    #[test]
    fn install_plan_is_pinned_and_matches_versions_env() {
        let plan = AmneziaWgProvider.install_plan(&node_config());
        plan.validate().unwrap();
        let env = include_str!("../../../deploy/lib/versions.env");
        for (key, value) in [
            ("AMNEZIAWG_GO_VERSION", AMNEZIAWG_GO_VERSION),
            ("AMNEZIAWG_GO_COMMIT", AMNEZIAWG_GO_COMMIT),
            ("AMNEZIAWG_TOOLS_VERSION", AMNEZIAWG_TOOLS_VERSION),
            ("AMNEZIAWG_TOOLS_COMMIT", AMNEZIAWG_TOOLS_COMMIT),
        ] {
            assert!(env.lines().any(|l| l.trim() == format!("{key}={value}")), "versions.env lacks {key}={value}");
        }
    }

    #[test]
    fn debug_output_never_contains_key_material() {
        let cfg = node_config();
        let c = AmneziaWgProvider.issue_credentials(&cfg, "u", &[]).unwrap();
        let dbg = format!("{cfg:?} {c:?}");
        assert!(!dbg.contains(cfg.server_private_key.as_ref().unwrap().expose()));
        assert!(!dbg.contains(c.private_key.expose()));
        assert!(!dbg.contains(c.preshared_key.expose()));
        assert!(!dbg.contains(cfg.params.header_protection_key.as_ref().unwrap().expose()));
    }
}
