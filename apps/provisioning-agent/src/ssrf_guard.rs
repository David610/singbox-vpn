//! SSRF guard for the protocol prober (Batch 7a, Area 4).
//!
//! The provisioning agent is privileged: it spawns a root `sing-box`
//! client process and drives real handshakes against destinations the
//! control plane hands it (`fetch_targets`). A compromised or buggy
//! control plane must not be able to turn this into a root network probe
//! against arbitrary local/internal services (the node's own loopback
//! admin surfaces, cloud metadata, other hosts on the private network,
//! etc).
//!
//! Every destination — whether it came from the operator's own
//! `static_targets` config or from `fetch_targets`'s control-plane
//! response — is resolved exactly once and the resolved IP is checked
//! against a denylist before any connection is attempted. The resolved
//! IP (not the original hostname) is what the sing-box client config
//! then dials, so a second, different resolution at connect time (DNS
//! rebinding) cannot bypass this check.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Why a destination was refused. Kept short and non-sensitive — safe to
/// log and to report upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    Loopback,
    LinkLocal,
    PrivateUseIpv4,
    CarrierGradeNat,
    UniqueLocalIpv6,
    Multicast,
    Unspecified,
    Broadcast,
    Documentation,
    Benchmarking,
    MetadataService,
    Ipv4MappedDenied,
    Reserved,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::Loopback => "loopback",
            DenyReason::LinkLocal => "link_local",
            DenyReason::PrivateUseIpv4 => "private_use_ipv4",
            DenyReason::CarrierGradeNat => "carrier_grade_nat",
            DenyReason::UniqueLocalIpv6 => "unique_local_ipv6",
            DenyReason::Multicast => "multicast",
            DenyReason::Unspecified => "unspecified",
            DenyReason::Broadcast => "broadcast",
            DenyReason::Documentation => "documentation",
            DenyReason::Benchmarking => "benchmarking",
            DenyReason::MetadataService => "metadata_service",
            DenyReason::Ipv4MappedDenied => "ipv4_mapped_denied",
            DenyReason::Reserved => "reserved",
        }
    }
}

/// The cloud-metadata address used by AWS/GCP/Azure/DO/etc. Blocked
/// explicitly and independently of the RFC 3927 link-local range check
/// below, so this stays denied even if that range check is ever narrowed.
const METADATA_V4: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);

/// Classifies a single resolved IP address. Returns `Some(reason)` when
/// it must never be dialed by a probe, `None` when it is an ordinary
/// public address.
pub fn classify_denied(ip: IpAddr) -> Option<DenyReason> {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => {
            // IPv4-mapped IPv6 (::ffff:a.b.c.d) must be judged by its
            // embedded IPv4 address, not treated as an ordinary IPv6
            // literal — this is a classic SSRF filter bypass.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return classify_v4(mapped).map(|_| DenyReason::Ipv4MappedDenied);
            }
            classify_v6(v6)
        }
    }
}

fn classify_v4(v4: Ipv4Addr) -> Option<DenyReason> {
    if v4 == METADATA_V4 {
        return Some(DenyReason::MetadataService);
    }
    if v4.is_loopback() {
        return Some(DenyReason::Loopback);
    }
    if v4.is_unspecified() {
        return Some(DenyReason::Unspecified);
    }
    if v4.is_broadcast() {
        return Some(DenyReason::Broadcast);
    }
    if v4.is_private() {
        return Some(DenyReason::PrivateUseIpv4);
    }
    if v4.is_link_local() {
        return Some(DenyReason::LinkLocal);
    }
    if v4.is_multicast() {
        return Some(DenyReason::Multicast);
    }
    if v4.is_documentation() {
        return Some(DenyReason::Documentation);
    }
    // 100.64.0.0/10 (RFC 6598 carrier-grade NAT / shared address space).
    let octets = v4.octets();
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Some(DenyReason::CarrierGradeNat);
    }
    // 198.18.0.0/15 (RFC 2544 benchmarking).
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return Some(DenyReason::Benchmarking);
    }
    // 0.0.0.0/8 ("this network") beyond the unspecified address itself.
    if octets[0] == 0 {
        return Some(DenyReason::Reserved);
    }
    None
}

fn classify_v6(v6: Ipv6Addr) -> Option<DenyReason> {
    if v6.is_loopback() {
        return Some(DenyReason::Loopback);
    }
    if v6.is_unspecified() {
        return Some(DenyReason::Unspecified);
    }
    if v6.is_multicast() {
        return Some(DenyReason::Multicast);
    }
    let seg0 = v6.segments()[0];
    // fe80::/10 link-local.
    if (seg0 & 0xffc0) == 0xfe80 {
        return Some(DenyReason::LinkLocal);
    }
    // fc00::/7 unique local (ULA).
    if (seg0 & 0xfe00) == 0xfc00 {
        return Some(DenyReason::UniqueLocalIpv6);
    }
    // 2001:db8::/32 documentation range.
    if v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8 {
        return Some(DenyReason::Documentation);
    }
    None
}

/// Result of validating and resolving one probe destination. `port` is
/// carried through so a caller building a `SocketAddr` from the result
/// does not need to thread the original port separately.
pub struct ResolvedTarget {
    pub ip: IpAddr,
    #[allow(dead_code)]
    pub port: u16,
}

/// Resolves `host:port` exactly once and validates the resolved address.
/// Returns the resolved IP so the caller can pin the outbound connection
/// to it (rather than re-resolving the hostname at connect time, which
/// would reopen the DNS-rebinding window this function exists to close).
///
/// Synchronous/blocking (`std::net::ToSocketAddrs`) — this only runs a
/// handful of times per probe round, not on a hot path, and the caller
/// already runs it inside `tokio::task::block_in_place`-friendly contexts
/// or an async task that can tolerate a short blocking resolve.
pub fn resolve_and_validate(host: &str, port: u16) -> Result<ResolvedTarget, String> {
    if host.is_empty() {
        return Err("empty host".into());
    }
    // An IP literal never needs resolution; validate it directly so a
    // literal denied address (e.g. "127.0.0.1") is caught even when the
    // system resolver would otherwise short-circuit successfully.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match classify_denied(ip) {
            Some(reason) => Err(format!("denied destination ({})", reason.as_str())),
            None => Ok(ResolvedTarget { ip, port }),
        };
    }
    use std::net::ToSocketAddrs;
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("resolution failed: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err("no addresses resolved".into());
    }
    // Every resolved address must be safe — a hostname that resolves to
    // both a public and a private address (multi-A-record rebinding
    // setup) must not be allowed to pick the public one, then have the
    // connection actually land on the private one via round-robin.
    for addr in &addrs {
        if let Some(reason) = classify_denied(addr.ip()) {
            return Err(format!(
                "denied destination (hostname resolved to {}: {})",
                addr.ip(),
                reason.as_str()
            ));
        }
    }
    Ok(ResolvedTarget {
        ip: addrs[0].ip(),
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    fn v6(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn denies_loopback_v4() {
        assert!(classify_denied(v4("127.0.0.1")).is_some());
        assert!(classify_denied(v4("127.0.0.2")).is_some());
    }

    #[test]
    fn denies_loopback_v6() {
        assert!(classify_denied(v6("::1")).is_some());
    }

    #[test]
    fn denies_rfc1918() {
        assert!(classify_denied(v4("10.0.0.1")).is_some());
        assert!(classify_denied(v4("172.16.0.1")).is_some());
        assert!(classify_denied(v4("192.168.1.1")).is_some());
    }

    #[test]
    fn denies_metadata_service() {
        assert_eq!(
            classify_denied(v4("169.254.169.254")),
            Some(DenyReason::MetadataService)
        );
    }

    #[test]
    fn denies_link_local_v4_generally_too() {
        assert!(classify_denied(v4("169.254.1.1")).is_some());
    }

    #[test]
    fn denies_carrier_grade_nat() {
        assert!(classify_denied(v4("100.64.0.1")).is_some());
        assert!(classify_denied(v4("100.100.0.1")).is_some());
        assert!(classify_denied(v4("100.127.255.255")).is_some());
        assert!(classify_denied(v4("100.63.255.255")).is_none());
        assert!(classify_denied(v4("100.128.0.0")).is_none());
    }

    #[test]
    fn denies_ipv6_ula_and_link_local() {
        assert_eq!(
            classify_denied(v6("fc00::1")),
            Some(DenyReason::UniqueLocalIpv6)
        );
        assert_eq!(
            classify_denied(v6("fd12:3456::1")),
            Some(DenyReason::UniqueLocalIpv6)
        );
        assert_eq!(classify_denied(v6("fe80::1")), Some(DenyReason::LinkLocal));
    }

    #[test]
    fn denies_ipv4_mapped_private_addresses() {
        assert!(classify_denied(v6("::ffff:10.0.0.1")).is_some());
        assert!(classify_denied(v6("::ffff:127.0.0.1")).is_some());
        assert!(classify_denied(v6("::ffff:169.254.169.254")).is_some());
    }

    #[test]
    fn allows_ipv4_mapped_public_addresses() {
        assert!(classify_denied(v6("::ffff:1.1.1.1")).is_none());
    }

    #[test]
    fn allows_ordinary_public_addresses() {
        assert!(classify_denied(v4("1.1.1.1")).is_none());
        assert!(classify_denied(v4("8.8.8.8")).is_none());
        assert!(classify_denied(v6("2606:4700:4700::1111")).is_none());
    }

    #[test]
    fn resolve_and_validate_rejects_ip_literal_targets() {
        assert!(resolve_and_validate("127.0.0.1", 443).is_err());
        assert!(resolve_and_validate("10.0.0.5", 443).is_err());
        assert!(resolve_and_validate("169.254.169.254", 443).is_err());
        assert!(resolve_and_validate("fc00::1", 443).is_err());
        assert!(resolve_and_validate("fe80::1", 443).is_err());
    }

    #[test]
    fn resolve_and_validate_allows_public_ip_literal() {
        let t = resolve_and_validate("1.1.1.1", 443).unwrap();
        assert_eq!(t.ip, v4("1.1.1.1"));
        assert_eq!(t.port, 443);
    }

    #[test]
    fn resolve_and_validate_rejects_empty_host() {
        assert!(resolve_and_validate("", 443).is_err());
    }
}
