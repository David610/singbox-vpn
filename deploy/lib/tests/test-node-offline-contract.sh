#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
python3 - <<'PY'
import json,re,pathlib
inventory=json.load(open('fixtures/arcana-node/provisioning-job-runtime-v1.json'))
source=pathlib.Path('apps/provisioning-agent/src/dispatch.rs').read_text()
worker=pathlib.Path('apps/provisioning-agent/src/worker_client.rs').read_text()
dispatched=set(re.findall(r'^\s*"([A-Z_]+)"\s*=>',source,re.M))
listed={j['type'] for j in inventory['jobs']}
assert dispatched == listed, (dispatched,listed)
def one(pattern, text):
 values=re.findall(pattern,text,re.M)
 assert len(values)==1, (pattern,values)
 return int(values[0])
# Derive the model from the actual dispatch/client constants, not a parallel
# hand-maintained assumption. Any code-bound change breaks this contract.
assert inventory['admin_attempts']==one(r'^const MAX_ATTEMPTS: u32 = (\d+);$',source)
assert inventory['retry_backoff_seconds']==one(r'^const RETRY_BACKOFF: Duration = Duration::from_secs\((\d+)\);$',source)
assert inventory['admin_attempt_timeout_seconds']==one(r'^const VPN_ADMIN_TIMEOUT: Duration = Duration::from_secs\((\d+)\);$',source)
assert inventory['agent_http_timeout_seconds']==one(r'^\s*\.timeout\(std::time::Duration::from_secs\((\d+)\)\)',worker)
for job in inventory['jobs']:
 expected=inventory['admin_attempts']*(job['network_fetches_per_attempt']*inventory['agent_http_timeout_seconds']+job['admin_invocations_per_attempt']*inventory['admin_attempt_timeout_seconds'])+(inventory['admin_attempts']-1)*inventory['retry_backoff_seconds']
 assert expected == job['worst_case_seconds'], (job,expected)
 assert job['worst_case_seconds'] < inventory['minimum_claim_lease_seconds'], job
 assert job['worst_case_seconds'] < inventory['control_plane_claim_lease_seconds'], job
caps=json.load(open('fixtures/arcana-node/node-capabilities-v1.json'))
assert caps['node_release']==re.search(r'version\s*=\s*"([^"]+)"',pathlib.Path('Cargo.toml').read_text()).group(1)
claim=caps['capabilities']['claim_token']
assert claim['minimum_lease_seconds']==inventory['minimum_claim_lease_seconds']
assert claim['minimum_lease_seconds'] <= inventory['control_plane_claim_lease_seconds']
assert all(claim[k] for k in ('required_in_claim','echoed_on_complete','echoed_on_fail','absolute_lease_deadline'))
telemetry=pathlib.Path('apps/provisioning-agent/src/telemetry.rs').read_text()
assert caps['capability_contract'] in telemetry
assert caps['provisioning_protocol']==one(r'^const PROVISIONING_PROTOCOL: u64 = (\d+);$',telemetry)
assert claim['minimum_lease_seconds']==one(r'^const MINIMUM_CLAIM_LEASE_SECONDS: u64 = (\d+);$',telemetry)
print('job inventory, runtime derivation, and capability contract: PASS')
PY
unit=apps/provisioning-agent/vpn-provisioning-agent.service
grep -qx 'UMask=0077' "$unit"
grep -qx 'StateDirectoryMode=0700' "$unit"
for file in apps/provisioning-agent/src/{report_queue,op_dedup,lease_pool,external_authorization}.rs; do
  grep -q 'from_mode(0o600)' "$file" || { echo "missing 0600 write: $file" >&2; exit 1; }
done
! rg -n 'set_permissions\([^;]+\)\.ok\(\)|let _ = .*set_permissions' apps/provisioning-agent/src
echo 'systemd and secret-state fail-closed contract: PASS'
