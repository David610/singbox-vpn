# Host-trust decision for the five historical root-password logins (issue #167)

**Status: BLOCKED on the owner. No host was accessed or changed.** Compromise is
**not** established and not ruled out. The current state of the staging host is
UNKNOWN. This page is the procedure for turning that into a recorded decision;
the tools named here were tested on synthetic logs only (documentation address
ranges), never on a real host.

## What only the owner can do

1. **Provider console, not SSH.** Open the provider's web console for the host.
   An SSH session might be the very thing in question.
2. **Snapshot first.** Take a provider snapshot before touching anything, so the
   evidence cannot be lost by a later rebuild or log rotation.
3. **Collect read-only evidence** (as root, in the console):
   `bash deploy/lib/ssh-auth-evidence.sh --since '2026-10-22 00:00'`
   (add `--expected FILE` with one of your own source addresses per line to flag
   anything else). It creates one new `0700` directory under `/root`, writes the
   raw logs and a label-to-address map there, and prints a **redacted** summary:
   sources appear as `S1`, `S2`, ...; no addresses, no invalid-user names (people
   mistype passwords there), no key material.
4. **Match labels to your own addresses**, privately, using
   `private-sources.json` in that directory. For each of the five historical
   logins decide: `OWNER` (it was me), `NOT_OWNER`, or `UNKNOWN`.
   Copy the directory off the host over an encrypted channel. Do not paste,
   commit or upload it.
5. **Write the decision record** (template below), keep it with your private
   evidence, and check it:
   `python3 deploy/lib/host_trust_decision.py decision.json`
   Exit 0 = TRUSTED (consistent), 3 = UNTRUSTED, 4 = UNDETERMINED, 1 = the record
   contradicts itself or contains an address / key material.

Flags in the summary are hints, not verdicts: `ROOT_PASSWORD_SUCCESS`,
`SUCCESS_AFTER_FAILURES` (a source that succeeded after at least five failures
looks like password guessing), `UNEXPECTED_SOURCE`. Missing logs prove nothing:
logs can be rotated or removed, and the collector warns when it finds none.

## Decision and consequences

| Decision | Meaning | Next step |
|---|---|---|
| `TRUSTED` | Every listed session is `OWNER` and a snapshot was taken | Continue with key-based SSH below, then the install checklist. |
| `UNTRUSTED` | At least one session is not yours, or you cannot vouch for it | **Do not install onto or "harden" this host.** Trusted rebuild and rotation, below. |
| `UNDETERMINED` | A fact is missing | Stays blocked. Say which fact in `notes`. |

`TRUSTED` is refused by the validator if any session is `NOT_OWNER` or `UNKNOWN`,
if no sessions are listed, or if no provider snapshot was taken.

## Key-based SSH without locking yourself out (only after TRUSTED)

`deploy/lib/ssh-harden.sh` already enforces the order; this is the human side.
1. `ssh-harden.sh add-key --pubkey-file KEY.pub`, then **keep your current session open**.
2. From a second machine or session: `ssh -i KEY root@host true`.
3. `ssh-harden.sh verify` must see `Accepted publickey` with the exact fingerprint.
4. Only then `ssh-harden.sh disable-passwords`, and immediately open a third
   session with the key to confirm.
5. Keep the provider console open throughout. If anything fails:
   `ssh-harden.sh rollback`, or restore the previous sshd drop-in from the console.
   The provider console works without SSH, which is why it comes first.

## If the decision is `UNTRUSTED`: trusted rebuild and rotation

Run only with your explicit go-ahead; each step changes something outside this repository.
- Build a **new** host from a clean provider image; do not reuse the old disk or snapshot as a base.
- Set a new root password and use key-only SSH from the start (above).
- Treat everything the old host held as exposed and replace it: SSH host keys and
  your own authorized keys, REALITY keypair and short ids, Hysteria2 and obfuscation
  passwords, every user's subscription token and credential, the TLS certificate
  private key (re-issue), and the off-host backup upload key and recipient.
- Revoke the old host's access to backup storage and any provider API token it could read.
- Re-issue every user's link; old profiles must be deleted in the clients.
- Re-run the install checklist on the new host. Keep the old snapshot, offline, for forensics.

## Decision record template (no addresses, no keys)

```json
{
  "schema": 1,
  "host_label": "fin1",
  "decided_on": "YYYY-MM-DD",
  "decided_by": "owner",
  "decision": "UNDETERMINED",
  "provider_snapshot_taken": false,
  "evidence_private_location": "where you keep the raw evidence (a label, not a path to secrets)",
  "sessions": [
    {"id": "L1", "classification": "UNKNOWN", "basis": "why you believe this"}
  ],
  "notes": ""
}
```

Closing #167 requires this record with `TRUSTED` or `UNTRUSTED` **and** the
follow-through for that outcome. The tools prepare the decision; they do not make it.
