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
    // Every reject is scoped to the sing-box uid with a POSITIVE match; the
    // old `meta skuid != UID accept` form let socketless kernel packets (IPv6
    // neighbour discovery) fall into the rejects and broke host IPv6 (I-54).
    assert!(shell.contains(&format!(
        "meta skuid $SING_BOX_UID tcp dport {C16_DENY_TCP_PORT} counter reject"
    )));
    assert!(shell.contains("meta skuid $SING_BOX_UID ip daddr { $IPV4_DENY } counter reject"));
    assert!(shell.contains("meta skuid $SING_BOX_UID ip6 daddr { $IPV6_DENY } counter reject"));
    let rules: String = shell
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rules.contains("skuid !="),
        "a negated uid match must not return: it also catches socketless kernel packets"
    );
}
