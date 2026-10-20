# I-53 — root cause and controlled experiment (2026-10-20)

**Root cause:** SIGPIPE under `set -o pipefail`. An early-exit consumer (`grep -q`, `awk ... exit`,
`head`) ends at its first match; a producer that writes more than one chunk is killed by SIGPIPE;
pipefail then reports status 141 although the pattern was found (and under `set -e` an assignment
aborts the script).

**Microbenchmark** (wrong answers, same static 10.7 KB file, same pattern):
`echo "$body" | grep -q PATTERN` 14/6000 (earlier run 25/3000) · here-string `grep -q PATTERN <<<"$body"` 0/6000 ·
200-byte value (single write) 0/6000.

**Controlled experiment** (hermetic runner, every test once per iteration, **no retries**):
| tree | condition | failing runs | which |
|---|---|---|---|
| original | idle, 6 iterations | 2 | test-ssh-harden (it 5, 6) |
| original | 2 busy CPU loops, 3 iterations (243 runs) | 3 | test-ssh-harden (every iteration) |
| fixed | 2 busy CPU loops, 4 iterations (328 runs) | **0** | - |

The loaded original failed because `ssh-harden.sh effective()` ran `sshd -T | awk ... exit`: awk exits,
sshd gets SIGPIPE, `post="$(effective ...)"` aborts under pipefail+set -e AFTER the drop-in was written.
Production exposure of the same class: `preflight.sh` `sshd -T | awk ... exit` (SSH-port detection that
gates firewall activation), install.sh's REALITY-decoy TLS probe (`printf "$probe_out" | grep -qE`), and
others. Fix: 306 echo/printf/cat|grep sites rewritten to here-strings, early-exit awk/head after real
commands get `|| true`, lint `deploy/lib/lint_pipefail_pipes.py` + guard test. Residual limit: the lint
cannot see every producer (e.g. a quoted path `"$x/cmd" | grep -q`); a retry-pass is now FLAKY (exit 3).
