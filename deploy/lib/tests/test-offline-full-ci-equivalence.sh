#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
python3 - <<'PY'
import json,pathlib,re
manifest=json.load(open('fixtures/arcana-node/offline-full-ci-v1.json'))
workflow=pathlib.Path('.github/workflows/ci.yml').read_text()
requirements=manifest['requirements']
gate=pathlib.Path('deploy/lib/offline-release-gate.sh').read_text()
jobs=[r['job'] for r in requirements]
assert len(jobs)==len(set(jobs)), 'duplicate CI mapping'
for step in manifest['full_gate_steps']:
    assert step['gate_evidence'] in gate, f"full gate step disappeared: {step['gate_evidence']}"
    assert step['ci_jobs'], f"unmapped full gate step: {step['gate_evidence']}"
    assert set(step['ci_jobs']) <= set(jobs), (step['gate_evidence'],step['ci_jobs'])
for requirement in requirements:
    job=requirement['job']; evidence=requirement['evidence']
    assert re.search(rf'^  {re.escape(job)}:$',workflow,re.M), f'missing CI job {job}'
    assert evidence in workflow, f'{job} lacks evidence command {evidence!r}'
verdict=manifest['verdict_job']
match=re.search(rf'^  {re.escape(verdict)}:\n(?P<body>.*?)(?=^  [a-z0-9-]+:|\Z)',workflow,re.M|re.S)
assert match, f'missing verdict job {verdict}'
needs=re.search(r'^    needs: \[(.*?)\]$',match.group('body'),re.M)
assert needs, 'verdict needs must be an explicit inline list'
actual={x.strip() for x in needs.group(1).split(',')}
assert actual==set(jobs),(sorted(actual),sorted(jobs))
assert 'bash deploy/lib/tests/test-offline-full-ci-equivalence.sh' in match.group('body')
print(f'offline full-gate CI equivalence: PASS ({len(jobs)} required jobs)')
PY
