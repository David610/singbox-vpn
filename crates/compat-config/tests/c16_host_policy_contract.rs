use compat_config::model::{C16_DENY_IPV4_CIDRS, C16_DENY_IPV6_CIDRS, C16_DENY_TCP_PORT};
use std::collections::BTreeSet;

fn shell_value<'a>(source: &'a str, name: &str) -> &'a str {
    let prefix = format!("{name}=\"");
    source
        .lines()
        .find_map(|line| line.strip_prefix(&prefix)?.strip_suffix('"'))
        .unwrap_or_else(|| panic!("missing {name} shell policy"))
}

fn cidrs(value: &str) -> BTreeSet<&str> {
    value.split(',').map(str::trim).collect()
}

#[test]
fn host_policy_matches_authoritative_c16_minus_only_loopback() {
    let shell = include_str!("../../../deploy/almalinux/nftables-egress-isolation.sh");
    let mut expected_v4: BTreeSet<_> = C16_DENY_IPV4_CIDRS.iter().copied().collect();
    assert!(expected_v4.remove("127.0.0.0/8"));
    let mut expected_v6: BTreeSet<_> = C16_DENY_IPV6_CIDRS.iter().copied().collect();
    assert!(expected_v6.remove("::1/128"));

    assert_eq!(cidrs(shell_value(shell, "IPV4_DENY")), expected_v4);
    assert_eq!(cidrs(shell_value(shell, "IPV6_DENY")), expected_v6);
    assert!(shell.contains(&format!("tcp dport {C16_DENY_TCP_PORT} reject")));
}
