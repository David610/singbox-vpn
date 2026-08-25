# Real VPS acceptance test

This is the release-blocking, **manual** acceptance procedure for the supported
AlmaLinux 9 x86_64 target. Run it on a new, disposable VPS. Local containers,
mocked `systemctl`, and CI configuration checks do not count as this test.
Never commit filled-in credentials, provisioning URLs, IP addresses, or a
claimed `PASS` to this file; record evidence in the private release record.

## Before starting

1. Record the release tag and commit SHA being tested.
2. Take a provider snapshot or confirm the VPS is disposable.
3. Configure provider-level firewall rules for the operator's SSH source,
   TCP/443, UDP/443 (when Hysteria2 is enabled), and the chosen public HTTPS
   provisioning port (8443 by default). The installer can configure the OS
   firewall, but cannot inspect or change the provider firewall.
4. Point a dedicated DNS name at the VPS and choose a legitimate TLS 1.3
   REALITY handshake server. Do not reuse production credentials.
5. Confirm the host starts blank: no singbox-vpn installation and no listeners
   occupying TCP/443, UDP/443, or the provisioning port.

Keep a private transcript of every command and its exit status. Replace the
shell variables below locally; do not paste their values into an issue.

```bash
export VPN_VERSION='vX.Y.Z'
export VPN_DOMAIN='vpn.example.test'
export REALITY_DECOY='www.example.com'
export TEST_USER='vps-acceptance'
```

## 1. Fresh installation

Install the pinned release, never a moving branch:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- --version "$VPN_VERSION" --domain "$VPN_DOMAIN" \
      --reality-handshake-server "$REALITY_DECOY" --non-interactive
```

The command must return zero only after configuration validation, services,
listeners, and health checks complete. Record `systemctl --failed`, then reboot
once and ensure both services recover without manual intervention:

```bash
sudo systemctl --failed
sudo reboot
# reconnect over SSH
sudo systemctl is-active sing-box vpn-subscription
sudo vpn doctor --protocol --require-protocol
```

The final doctor output may say `SERVER-SIDE CHECKS PASSED` and must still say
`REAL CLIENT / RUSSIAN NETWORK TEST STILL REQUIRED`. Treat any warning about a
required protocol check as failure.

## 2. Provision and test a real client

```bash
sudo vpn user create --name "$TEST_USER" --json | sudo tee /root/vpn-user.json >/dev/null
sudo chmod 600 /root/vpn-user.json
sudo jq -r .provisioning_url /root/vpn-user.json
# .subscription_url is the ?format=hiddify share-link URL (JSON output only
# includes that one, per apps/admin/src/main.rs's subscription_url()); swap
# its query string to ?format=singbox for the URL that actually imports into
# singbox-client today — see below.
```

**`.provisioning_url` (`/v1/provision/{token}`) does not currently import
into `singbox-client`.** As of `singbox-client` main `d04a8b4`, its
subscription importer only parses a raw sing-box config
(`{"outbounds": [...]}`) — it has no code for this contract's
`schema_version`/`capabilities`/`endpoints` shape (see
`docs/PROVISIONING_CONTRACT.md`). Still fetch and shape-check it, since
that is real coverage this test can give beyond CI:

```bash
read -rsp 'Provisioning URL: ' PROVISION_URL; echo
curl --fail --silent --show-error --dump-header /root/provision.headers \
  --output /root/provision.json "$PROVISION_URL"
unset PROVISION_URL
sudo chmod 600 /root/provision.headers /root/provision.json
jq -e '.schema_version == 1' /root/provision.json
grep -iE '^(cache-control: no-store|x-robots-tag: noindex)' /root/provision.headers
```

For the actual on-device import, hand the client the **`?format=singbox`**
URL instead (`.subscription_url` above, or append `?format=singbox` to the
bare `/sub/{token}` URL) — this is the format `crates/compat-config`'s
`reality_interop.rs`/`hysteria2_interop.rs` already prove against a real
`sing-box` binary in CI, so a device failure here means a real gap, not an
untested format. From a real client, test VLESS+REALITY and Hysteria2
separately, then automatic selection. Test from both an ordinary network
and the actual difficult Russian network. Record client version,
network/provider, transport selected, DNS and IPv4/IPv6 observations, but
no secrets or full URLs.

## 3. Revocation and token rotation

Save the user ID locally, disable the user, and prove both control-plane and
transport access are gone:

```bash
USER_ID="$(sudo jq -r .id /root/vpn-user.json)"
sudo vpn user disable "$USER_ID"
# paste the old URL again without echoing it
read -rsp 'Old provisioning URL: ' OLD_URL; echo
test "$(curl -sS -o /dev/null -w '%{http_code}' "$OLD_URL")" = 404
unset OLD_URL
sudo vpn doctor --protocol --require-protocol
```

The already-imported client must fail on both transports. Re-enable the user,
rotate only the provisioning token, and verify the old URL returns 404 while
already-imported transport credentials continue to work. Then rotate VLESS and
Hysteria2 credentials and verify the previous imported credentials fail:

```bash
sudo vpn user enable "$USER_ID"
sudo vpn user rotate-token "$USER_ID"
sudo vpn user rotate-vless "$USER_ID"
sudo vpn user rotate-hysteria "$USER_ID"
sudo vpn doctor --protocol --require-protocol
```

## 4. Backup, update, and restore

Create the archive outside a publicly served directory and verify its mode and
contents. The archive is a master secret:

```bash
sudo vpn backup --output /root/singbox-vpn-acceptance.tar
stat -c '%a %U:%G %n' /root/singbox-vpn-acceptance.tar
sudo tar -tf /root/singbox-vpn-acceptance.tar | sort
```

Mode must be `600`; the listing must contain only the documented bounded state.
Exercise the pinned update transaction (or its documented no-op behavior when
the same version is current), then re-run doctor and the real-client checks:

```bash
sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version "$VPN_VERSION"
sudo vpn doctor --protocol --require-protocol
```

For a disposable VPS, create a second temporary user, restore the archive, and
verify that the second user disappears while the archived user works again.
Do not perform this destructive check on a production server without a provider
snapshot:

```bash
sudo vpn user create --name restore-canary
sudo vpn restore /root/singbox-vpn-acceptance.tar
sudo vpn user list
sudo vpn doctor --protocol --require-protocol
```

Finally inspect sanitized diagnostics and confirm they contain no UUID,
password, private key, bearer token, or complete provisioning URL:

```bash
sudo vpn doctor --report --report-output /root/vpn-support-report.txt
stat -c '%a %U:%G %n' /root/vpn-support-report.txt
sudo less /root/vpn-support-report.txt
```

## 5. Completion criteria

Acceptance is complete only when all commands returned their expected status,
rollback/reboot recovery was observed, and the current first-party client was
tested from a real Russian network. Server-side CI or loopback doctor results
alone are insufficient. Securely delete the local JSON, headers, response, and
backup when the evidence has been recorded, then destroy the disposable VPS.
