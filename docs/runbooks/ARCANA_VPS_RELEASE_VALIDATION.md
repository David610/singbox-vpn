# Arcana node release validation on two disposable VPSs

## Status and safety

This is the **remaining live gate** after `deploy/lib/offline-release-gate.sh`; it is not evidence that a VPS test happened. Use two newly-created AlmaLinux 9 x86_64 hosts, `VPS1` (entry) and `VPS2` (exit), with public IPv4 and IPv6. Never use production nodes or credentials. Set `RC_TAG` to the immutable release tag whose archives, checksums and attestations were produced from the candidate commit.

On the operator workstation:

```bash
export VPS1=root@203.0.113.10 VPS2=root@203.0.113.20
export VPS1_V4=203.0.113.10 VPS2_V4=203.0.113.20
export VPS1_V6=2001:db8:1::10 VPS2_V6=2001:db8:2::20
export RC_TAG=v1.1.0-rc.N PREVIOUS_TAG=v1.0.0
mkdir -p evidence/{local,vps1,vps2}; script -af evidence/session.typescript
```

For every numbered test, **expected** is mandatory; any **failure** means stop publication. The standard **stop/rollback** is `sudo systemctl stop vpn-provisioning-agent sing-box` followed by restore/rebuild or disposal. Save stdout/stderr, `journalctl` and named artifacts without publishing secrets; encrypt the evidence directory at rest.

## 0. Offline and release identity

Command:

```bash
git checkout "$RC_TAG" && git rev-parse HEAD | tee evidence/local/commit.txt
bash deploy/lib/offline-release-gate.sh --full 2>&1 | tee evidence/local/offline-gate.log
sha256sum singbox-vpn-*.tar.gz SHA256SUMS | tee evidence/local/artifact-hashes.txt
```

Expected: gate says `PASS`; commit and artifact subjects equal the release provenance. Failure: any skip/failure or mutable ref. Stop: do not create VPSs. Save: listed logs, SBOM, license inventory, checksums and Sigstore bundles.

## 1. Clean install and enrollment (both hosts)

Command (repeat with `H=$VPS1`, then `$VPS2`; use each node's one-time enrollment values):

```bash
ssh "$H" 'cat /etc/os-release; uname -a; systemd --version; nft --version'
scp install.sh "$H:/root/install.sh"
ssh -t "$H" "sudo env SINGBOX_VPN_VERSION=$RC_TAG bash /root/install.sh --non-interactive --public-host vpn.example.test --subscription-host sub.example.test"
ssh "$H" 'sudo vpn-admin doctor --json; sudo systemctl status --no-pager sing-box vpn-provisioning-agent vpn-egress-isolation; sudo nft list table inet arcana_egress_isolation'
```

Expected: checksum/attestation-verified install; enrollment binds the intended opaque node; services active and doctor healthy. Failure: unsigned/mismatched asset, wrong node, unhealthy unit, or install over 20 minutes. Rollback: `ssh "$H" 'sudo /usr/local/lib/singbox-vpn/uninstall.sh --yes'`, then dispose. Save: installer output, doctor JSON, unit status, nft rules, `journalctl -b`.

## 2. Permissions and systemd

Command:

```bash
for H in "$VPS1" "$VPS2"; do ssh "$H" 'sudo systemd-analyze verify /etc/systemd/system/{sing-box,vpn-provisioning-agent,vpn-egress-isolation}.service; sudo systemctl show vpn-provisioning-agent -p UMask -p StateDirectoryMode -p User; sudo namei -l /var/lib/vpn-provisioning-agent; sudo find /etc/vpn /var/lib/vpn-provisioning-agent -xdev -type f -printf "%m %u:%g %p\n" | sort; sudo systemd-analyze security sing-box.service vpn-provisioning-agent.service'; done | tee evidence/local/permissions.txt
```

Expected: agent `UMask=0077`, state directory `0700`, claim/report/lease/auth/API-key files `0600`, service-readable runtime secrets no broader than `0640`, no unit errors. Failure: group/other-readable bearer token, key, UUID/password state, or inability to chmod. Stop: stop both services and dispose; never weaken modes. Save: complete output.

## 3. One hop, DNS and IPv6

Create a disposable external authorization and download its full sing-box client config as documented by the test control plane, then:

```bash
sing-box check -c evidence/client-vps2.json
sudo ip netns add arcana-client; sudo ip link add ac-host type veth peer name ac-client
sudo ip link set ac-client netns arcana-client
sudo ip addr add 192.0.2.1/30 dev ac-host; sudo ip link set ac-host up
sudo ip netns exec arcana-client ip addr add 192.0.2.2/30 dev ac-client
sudo ip netns exec arcana-client ip link set lo up; sudo ip netns exec arcana-client ip link set ac-client up
sudo ip netns exec arcana-client ip route add default via 192.0.2.1
sudo ip netns exec arcana-client sing-box run -c "$PWD/evidence/client-vps2.json" >evidence/local/one-hop-singbox.log 2>&1 & echo $! >evidence/local/client.pid
sudo ip netns exec arcana-client curl --fail --max-time 15 https://1.1.1.1/cdn-cgi/trace
sudo ip netns exec arcana-client getent ahosts example.com
sudo ip netns exec arcana-client curl -6 --fail --max-time 15 https://[2606:4700:4700::1111]/cdn-cgi/trace
```

Expected: real client and server handshakes; egress IP is VPS2; DNS resolves through configured tunnel; IPv6 either works through the declared route or fails closed with no direct fallback (record which capability was advertised). Failure: direct workstation IP, DNS leak, unexpected IPv4 fallback, or config rejection. Stop: `sudo kill $(cat evidence/local/client.pid); sudo ip netns del arcana-client`. Save: config with secrets redacted, client/server journals, traces, `ip -6 route`, resolver state and packet capture restricted to endpoint metadata.

## 4. Private, metadata, link-local and loopback blocking

Command inside the client namespace:

```bash
for u in http://127.0.0.1 http://10.0.0.1 http://172.16.0.1 http://192.168.0.1 http://169.254.169.254/latest/meta-data http://[::1] http://[fc00::1] http://[fe80::1]; do sudo ip netns exec arcana-client curl -g -sS --connect-timeout 3 "$u" && { echo "UNEXPECTED $u"; exit 1; } || echo "blocked $u"; done | tee evidence/local/blocked-destinations.txt
ssh "$VPS2" 'sudo nft list table inet arcana_egress_isolation -a; sudo journalctl -u sing-box --since -10min'
```

Expected: every destination fails; public one-hop remains healthy afterward. Failure: any response from a denied range. Stop: stop sing-box, preserve nft rules and journals, dispose. Save: command output and nft counters/rules.

## 5. Two hop and injected failures

Publish VPS1 as relay to VPS2 as exit and obtain the full nested sing-box config:

```bash
sing-box check -c evidence/client-two-hop.json
sudo ip netns exec arcana-client sing-box run -c "$PWD/evidence/client-two-hop.json" >evidence/local/two-hop.log 2>&1 &
sudo ip netns exec arcana-client curl --fail --max-time 15 https://1.1.1.1/cdn-cgi/trace
ssh "$VPS2" 'sudo systemctl stop sing-box'; timeout 20 sudo ip netns exec arcana-client curl --fail https://1.1.1.1 && exit 1 || true
ssh "$VPS2" 'sudo systemctl start sing-box'
ssh "$VPS1" 'sudo systemctl stop sing-box'; timeout 20 sudo ip netns exec arcana-client curl --fail https://1.1.1.1 && exit 1 || true
ssh "$VPS1" 'sudo systemctl start sing-box'
```

Expected: trace egress is VPS2, not VPS1; either hop outage fails closed; recovery reconnects without Fast fallback. Failure: egress VPS1/direct, traffic survives a required-hop outage, or no recovery. Stop: kill client and stop both servers. Save: both server journals, client log, traces and outage timestamps.

## 6. Snapshot A → A+B → B → reboot → B; rotation and revoke

Use the control-plane test API/CLI to publish successively revision `N` with A, `N+1` with A+B, and `N+2` with B. Record returned revision/digest each time. Then:

```bash
ssh "$VPS2" 'sudo cat /var/lib/vpn-provisioning-agent/external-authorizations.json | jq "del(..|.vless_uuid?,..|.hysteria2_password?)"; sudo systemctl restart vpn-provisioning-agent; sudo reboot'
ssh -o ConnectTimeout=5 "$VPS2" true || true; sleep 60
ssh "$VPS2" 'sudo systemctl is-active sing-box vpn-provisioning-agent; sudo vpn-admin user list --json'
```

Expected: A works, A+B both work, then only B works; reboot retains only B; a pre-existing native/operator user still works. Publish empty `N+3`: all external credentials fail while native/operator remains. Failure: A resurrects, empty removes native users, or acknowledgment precedes live apply. Rollback: republish last known-good higher revision; never reuse a lower revision. Save: redacted state, acknowledgments, list output and journals.

## 7. Stale/equivocating revision

Command: replay revision `N+2` after `N+3`, then send different content with revision `N+3` through the isolated test control plane.

```bash
ssh "$VPS2" 'sudo journalctl -u vpn-provisioning-agent --since -10min; sudo sha256sum /var/lib/vpn-provisioning-agent/external-authorizations.json /etc/vpn/compat/sing-box/config.json'
```

Expected: both are rejected, hashes/live access unchanged, no ACK. Failure: mutation/restart/ACK. Rollback: stop agent, preserve state, republish a higher good revision. Save: request IDs, sanitized bodies, logs and hashes.

## 8. Claim crash, reclaim, stale completion, and lost report

Configure a **300-second or longer** test lease. Pause the completion endpoint after a mutating job is claimed; kill the agent during/after mutation:

```bash
ssh "$VPS2" 'sudo journalctl -fu vpn-provisioning-agent' | tee evidence/vps2/claim.log &
# submit one idempotent ENABLE_USER test job, wait for claim id/token fingerprint server-side
ssh "$VPS2" 'sudo systemctl kill -s KILL vpn-provisioning-agent; sudo systemctl start vpn-provisioning-agent'
# after lease expiry, have a second test agent/node claim the same job; release delayed old completion
```

Expected: new claim has a different secret token; stale completion gets 409/410 and cannot overwrite the new attempt. For lost-report injection, allow mutation, return 503/close connection on `/complete`, reboot VPS2, restore endpoint: persisted queue retries the original token and receives idempotent 2xx or terminal stale response, never remutating destructively. Failure: old token accepted after reclaim, queue mode not 0600, unbounded retry, duplicate destructive mutation. Stop: disable test fault proxy and agent; restore user snapshot. Save: control-plane audit rows with token values hashed/redacted, queue metadata redacted, timestamps and journals.

## 9. Agent/control-plane/server outages and reboot

```bash
ssh "$VPS2" 'sudo systemctl kill -s KILL vpn-provisioning-agent; sleep 8; systemctl is-active vpn-provisioning-agent'
# firewall the test control-plane IP for 3 minutes, then remove that exact rule
ssh "$VPS2" 'sudo systemctl reboot'
sleep 60; ssh "$VPS2" 'sudo systemctl is-active sing-box vpn-provisioning-agent vpn-egress-isolation; sudo vpn-admin doctor --json'
```

Expected: systemd restarts agent; VPN retains only unexpired persisted authorization during control-plane outage; local expiry still removes expired access; reboot restores firewall before sing-box. Failure: fail-open, expired credential works, firewall absent, or secrets logged. Rollback: remove only injected firewall rule, stop units, dispose if uncertain. Save: before/after unit dependencies, journals and doctor JSON.

## 10. Upgrade, rollback, uninstall and partial install

First build a fresh third lifecycle by reinstalling VPS1 at `PREVIOUS_TAG`, create native and external fixtures, then:

```bash
ssh -t "$VPS1" "sudo env SINGBOX_VPN_VERSION=$PREVIOUS_TAG bash /root/install.sh --non-interactive ..."
ssh "$VPS1" "sudo vpn-admin update --version $RC_TAG"
ssh "$VPS1" 'sudo vpn-admin doctor --json; sudo systemctl is-active sing-box vpn-provisioning-agent'
# exercise documented rollback using the transaction backup printed by update
ssh "$VPS1" "sudo vpn-admin update --version $PREVIOUS_TAG"
ssh "$VPS1" 'sudo /usr/local/lib/singbox-vpn/uninstall.sh --yes; ! systemctl is-active sing-box; ! nft list table inet arcana_egress_isolation'
```

Expected: upgrade preserves IDs/state/modes, rollback restores previous version and access, uninstall removes only owned files/table/users. Failure: state loss, mode widening, orphaned active service/rules, or unrelated state removed. Stop: use update's recorded transaction rollback; dispose.

Partial-install injection on a newly reprovisioned VPS1:

```bash
ssh "$VPS1" 'sudo env PATH=/root/fault-bin:$PATH SINGBOX_VPN_VERSION='"$RC_TAG"' bash /root/install.sh ...' # fault-bin makes systemctl enable fail once
ssh "$VPS1" 'sudo find /etc/vpn /var/lib/vpn-provisioning-agent /usr/local/lib/singbox-vpn -maxdepth 2 -ls; sudo systemctl list-unit-files "vpn-*" "sing-box*"; sudo nft list tables'
```

Expected: installer exits nonzero and transaction cleanup removes only newly-created assets; retry without fault succeeds. Failure: active partial service, leaked secret, owned nft table or ambiguous ownership. Stop: run uninstall only if ownership manifest exists; otherwise dispose. Save: fault shim, installer/rollback output and filesystem/unit/nft inventories.

## 11. Claim-token enforcement canary

Prerequisite: every active node heartbeat advertises **exactly** `arcana.node.capabilities.v1`, provisioning protocol `>=2`, `claim_token.version>=1`, and `minimum_lease_seconds>=300`; every node runs release `>=1.1.0`; no unexpired tokenless claims or old queued reports exist; monitoring can distinguish 401/409 from 5xx; rollback toggle is tested.

```bash
# control-plane-specific commands, run first on the two disposable node IDs only
vpn-web-admin node-capabilities --all --require-contract arcana.node.capabilities.v1 --require claim_token=1 --require-min-lease 300
vpn-web-admin feature set REQUIRE_CLAIM_TOKEN --nodes "$VPS1_NODE_ID,$VPS2_NODE_ID"
vpn-web-admin job submit --node "$VPS2_NODE_ID" --type ENABLE_USER --fixture canary
vpn-web-admin job attempt-complete --without-claim-token "$CANARY_JOB_ID" # must be 401/400
vpn-web-admin job wait "$CANARY_JOB_ID" --state completed --timeout 300
```

Expected: capability preflight has zero exceptions; tokenless completion rejected; genuine claimed completion accepted; no claim/report error spike. Failure: unknown/missing capability, lease under 300 seconds, tokenless acceptance, or canary not complete. Rollback: `vpn-web-admin feature unset REQUIRE_CLAIM_TOKEN --nodes ...`; stop new jobs and investigate. Save: capability export, sanitized request/status audit, metrics before/during/after and toggle audit.

## Release decision

Only after every section passes on both disposable VPSs may the operator promote beyond **OFFLINE-QUALIFIED**. Archive encrypted evidence with commit/tag, exact timestamps, distro images and test control-plane commit. Any omitted command remains an explicit blocker; mocks, containers and namespaces do not substitute for these host/kernel/network results.
