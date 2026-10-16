#!/usr/bin/env bash
# deterministic fingerprint of everything an update touches
{
 echo "manifest  $(sha256sum /var/lib/singbox-vpn/install-state.json | cut -c1-16)"
 echo "binaries  $(sha256sum /usr/local/bin/vpn /usr/local/bin/vpn-admin /usr/local/bin/vpn-subscription-svc /usr/local/bin/sing-box | cut -c1-12 | tr '\n' ' ')"
 echo "tree      $(find /opt/singbox-vpn -path /opt/singbox-vpn/target -prune -o -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)"
 echo "deploycfg $(sha256sum /etc/vpn/deployment.toml | cut -c1-16)"
 echo "singboxcfg $(sha256sum /etc/vpn/compat/sing-box/config.json | cut -c1-16)"
 echo "units     $(cat /etc/systemd/system/sing-box.service /etc/systemd/system/vpn-subscription.service 2>/dev/null | sha256sum | cut -c1-16)"
 echo "services  $(systemctl is-active sing-box vpn-subscription nginx | tr '\n' ' ')"
 echo "leftovers $(ls -d /opt/singbox-vpn.prev* /opt/.singbox-vpn-source.* 2>/dev/null | wc -l)"
}
