# CodeQL Alert Triage — 2026-09-29

Scope: alerts #22–#29 (`rust/hard-coded-cryptographic-value`) and #33–#60
(`rust/cleartext-logging`, plus two `rust/path-injection` alerts that
happened to fall inside the numeric range and are noted as out-of-scope).

Methodology: for every alert, `gh api repos/David610/singbox-vpn/code-scanning/alerts/<N>`
was queried for its **current** state and `most_recent_instance.location`
(never a previously-reported line number), then the flagged file was read
at that exact location to trace what value actually reaches the sink.

This is a read-only triage. No alert was dismissed, closed, or otherwise
mutated via the API. No code was changed. Only this document was added.

## Summary

| Classification | Count |
|---|---|
| TEST-FIXTURE-ONLY | 8 (#22–#29) |
| FALSE POSITIVE | 22 (#33–#47, #52–#54, #57–#60) |
| ALREADY RESOLVED (dismissed or fixed prior to this pass) | 4 (#50, #51, #55, #56) |
| OUT OF SCOPE (different rule, already fixed) | 2 (#48, #49) |
| REAL FINDING | 0 |
| UNCLEAR | 0 |

**No urgent findings.** Nothing in this batch is a real leaked secret. Every
`rust/cleartext-logging` alert in #33–#60 traces to one of: the `user_id`
value (a `user_<uuid4>` string that the surrounding code explicitly
documents as "NOT a credential and NOT your subscription token" —
`apps/admin/src/main.rs:2720`), a non-secret integer/boolean (cert-expiry
day-count, applied-revision counter, an `enabled` flag), a fixed
non-secret string literal (a CLI flag name), or a line that CodeQL's
`most_recent_instance.location` points at but which — after multiple
commits on this branch shifted line numbers — no longer contains a print
statement at all (doc comments, enum variants, closing braces). All 8
`rust/hard-coded-cryptographic-value` alerts land on obviously-fake
placeholder strings (e.g. `"test-password-not-a-real-secret"`,
`"correct-password"`, `"fake-hysteria2-password"`) inside `#[test]`
functions used only to drive a real `sing-box` binary over loopback for
deterministic interop testing — consistent with this repo's documented
"real sing-box interop tests" pattern, not a vulnerability.

## Table

| Alert # | Rule | Location (file:line) | Current State | Classification | Reasoning | Recommended action |
|---|---|---|---|---|---|---|
| 22 | hard-coded-cryptographic-value | crates/compat-config/tests/hysteria2_interop.rs:245-246 | open | TEST-FIXTURE-ONLY | `password = "test-password-not-a-real-secret"`, local var in `#[test] fn`, feeds a loopback sing-box interop test only. | DISMISS-AS-TEST-FIXTURE |
| 23 | hard-coded-cryptographic-value | crates/compat-config/tests/hysteria2_interop.rs:310 | open | TEST-FIXTURE-ONLY | `"correct-password"` literal in `#[test] fn hysteria2_handshake_fails_with_wrong_password`, loopback-only fixture. | DISMISS-AS-TEST-FIXTURE |
| 24 | hard-coded-cryptographic-value | crates/compat-config/tests/hysteria2_interop.rs:320 | open | TEST-FIXTURE-ONLY | `"a-completely-different-password"` literal, same test, intentionally-mismatched fixture. | DISMISS-AS-TEST-FIXTURE |
| 25 | hard-coded-cryptographic-value | crates/compat-config/tests/hysteria2_interop.rs:370-371 | open | TEST-FIXTURE-ONLY | `"test-password-not-a-real-secret"` / `"test-obfs-password-not-a-real-secret"` in `#[test] fn hysteria2_obfuscated_handshake_succeeds_with_matched_obfs_password`. | DISMISS-AS-TEST-FIXTURE |
| 26 | hard-coded-cryptographic-value | crates/compat-config/tests/hysteria2_interop.rs:430 | open | TEST-FIXTURE-ONLY | `"correct-password"` / `"server-obfs-password"` literals, `#[test] fn hysteria2_handshake_fails_with_wrong_obfs_password`. | DISMISS-AS-TEST-FIXTURE |
| 27 | hard-coded-cryptographic-value | crates/compat-config/tests/hysteria2_interop.rs:440 | open | TEST-FIXTURE-ONLY | `"a-completely-different-obfs-password"` literal, same test, intentionally-mismatched fixture. | DISMISS-AS-TEST-FIXTURE |
| 28 | hard-coded-cryptographic-value | crates/provisioning-contract/src/lib.rs:1571 | open | TEST-FIXTURE-ONLY | `Hysteria2Obfs::salamander("")` inside `#[test] fn empty_obfs_password_is_rejected`, under `#[cfg(test)] mod tests` (starts line 1389); empty-string is the point of the test. | DISMISS-AS-TEST-FIXTURE |
| 29 | hard-coded-cryptographic-value | crates/provisioning-contract/src/lib.rs:1725 | open | TEST-FIXTURE-ONLY | `"fake-hysteria2-password"` / `"fake-obfs-password"` in `#[test] fn json_round_trip_preserves_every_field`, same `#[cfg(test)]` module. | DISMISS-AS-TEST-FIXTURE |
| 33 | cleartext-logging | apps/admin/src/main.rs:9213 | open | FALSE POSITIVE | Location drifted onto `#[test] fn build_dns_query_contains_labels`; no `println!`/log call there. Underlying tainted value (`cert_expiry_days`) is a non-secret `i64` day-count in all other instances of this pattern (see #46/#47). | DISMISS-AS-FALSE-POSITIVE |
| 34 | cleartext-logging | apps/admin/src/main.rs:2692 | open | FALSE POSITIVE | Location drifted onto `"enabled": true` (a JSON literal), not a `generate_user_id()` call. Source value is `user_id`, explicitly documented as non-credential (main.rs:2720). | DISMISS-AS-FALSE-POSITIVE |
| 35 | cleartext-logging | apps/admin/src/main.rs:2717-2718 | open | FALSE POSITIVE | `println!("  {id}")` prints the `user_id`, documented in the very next lines as "NOT a credential and NOT your subscription token." | DISMISS-AS-FALSE-POSITIVE |
| 36 | cleartext-logging | apps/admin/src/main.rs:2750-2753 | open | FALSE POSITIVE | Flagged text is a static doc string about the provisioning-contract schema version, not a secret value. | DISMISS-AS-FALSE-POSITIVE |
| 37 | cleartext-logging | apps/admin/src/main.rs:2857 | open | FALSE POSITIVE | Drifted onto `let machine_stdout = if json {...}`; no `user_id`/secret printed at this line. | DISMISS-AS-FALSE-POSITIVE |
| 38 | cleartext-logging | apps/admin/src/main.rs:2924 | open | FALSE POSITIVE | Drifted onto the closing `}` of `cmd_user_create_probe`. The function's actual prints (lines 2882, 2918) emit `id` (`user_id`, non-secret) and the fixed reserved probe name `"arcana-probe"` — no credential. This contradicts the prior pass's "real leak" classification; re-review recommended but current evidence shows no secret reaches the sink. | DISMISS-AS-FALSE-POSITIVE |
| 39 | cleartext-logging | apps/admin/src/main.rs:3301 | open | FALSE POSITIVE | Drifted onto the string literal `"--credential-stdin"` (a flag name passed to `read_secret_from_stdin`, used only in an error message), not the secret value itself, which is deliberately never logged. | DISMISS-AS-FALSE-POSITIVE |
| 40 | cleartext-logging | apps/admin/src/main.rs:3371 | open | FALSE POSITIVE | `println!("{id}: no peer credentials configured.")` — `id` is `user_id`, non-secret; the function's own doc comment says "Ids and transports only — never a value." | DISMISS-AS-FALSE-POSITIVE |
| 41 | cleartext-logging | apps/admin/src/main.rs:3392 | open | FALSE POSITIVE | Drifted onto `Some(MachineStdout::divert_human_output_to_stderr()?)`; no secret printed at this line. | DISMISS-AS-FALSE-POSITIVE |
| 42 | cleartext-logging | apps/admin/src/main.rs:3465-3468 | open | FALSE POSITIVE | Drifted onto a doc comment (`/// local endpoints, in declaration order...`); no print statement present. | DISMISS-AS-FALSE-POSITIVE |
| 43 | cleartext-logging | apps/admin/src/main.rs:3465-3468 | open | FALSE POSITIVE | Same drifted doc-comment location as #42 (duplicate instance). | DISMISS-AS-FALSE-POSITIVE |
| 44 | cleartext-logging | apps/admin/src/main.rs:3732 | open | FALSE POSITIVE | Drifted onto the `Corrupt,` enum variant of `RevisionStampState`; no print statement. | DISMISS-AS-FALSE-POSITIVE |
| 45 | cleartext-logging | apps/admin/src/main.rs:3771 | open | FALSE POSITIVE | Drifted onto doc-comment prose about revision rollback; no print statement. | DISMISS-AS-FALSE-POSITIVE |
| 46 | cleartext-logging | apps/admin/src/main.rs:3841 | open | FALSE POSITIVE | `println!("{}", json!({{ "revision": revision }}))` — `revision` is a non-secret `Option<u64>` applied-config counter, not a credential. | DISMISS-AS-FALSE-POSITIVE |
| 47 | cleartext-logging | apps/admin/src/main.rs:3842 | open | FALSE POSITIVE | Same `revision` counter as #46 (duplicate instance, `match` arm). | DISMISS-AS-FALSE-POSITIVE |
| 48 | path-injection (out of scope) | crates/compat-config/src/store.rs:88 | fixed | N/A | Different rule (`rust/path-injection`), not `cleartext-logging`; GitHub already reports it `fixed`. Included only because it fell in the #33–#60 numeric range. | NO ACTION NEEDED |
| 49 | path-injection (out of scope) | crates/compat-config/src/store.rs:85 | fixed | N/A | Same as #48. | NO ACTION NEEDED |
| 50 | cleartext-logging | apps/admin/src/main.rs:1991-1995 | state: null (most_recent_instance state: "fixed", only ever observed on `refs/pull/66/merge`) | FALSE POSITIVE / already resolved | Alert's only recorded instance is on a PR merge ref, already marked "fixed" by GitHub, not currently open against this branch's head. Underlying source is again `generate_user_id()` → non-secret `user_id`. | NO ACTION NEEDED (verify not reopened) |
| 51 | cleartext-logging | apps/admin/src/main.rs:3208-3210 | dismissed ("false positive") | FALSE POSITIVE | Already dismissed by a prior pass; consistent with the `user_id`-is-not-a-secret pattern confirmed throughout this batch. | ALREADY RESOLVED |
| 52 | cleartext-logging | apps/admin/src/main.rs:3227 | open | FALSE POSITIVE | Drifted onto `fn read_secret_from_stdin(prompt: &str, flag_name: &str) -> Result<String> {` — a function signature, not a value being logged; this function's entire purpose is to keep the secret OUT of argv/logs. | DISMISS-AS-FALSE-POSITIVE |
| 53 | cleartext-logging | apps/admin/src/main.rs:3244 | open | FALSE POSITIVE | Drifted onto `stdin.lock().read_line(&mut line)?;` inside `read_secret_from_stdin` — reads into a local buffer, never printed. | DISMISS-AS-FALSE-POSITIVE |
| 54 | cleartext-logging | apps/admin/src/main.rs:3247 | open | FALSE POSITIVE | Drifted onto `bail!("{flag_name}: no value was provided on standard input");` — `flag_name` is a fixed non-secret string like `"--credential-stdin"`, not the secret value. | DISMISS-AS-FALSE-POSITIVE |
| 55 | cleartext-logging | apps/admin/src/main.rs:9176 | dismissed ("used in tests") | TEST-FIXTURE-ONLY | Already dismissed; location is inside the `#[cfg(test)]` `cert_expiry_days` test module. | ALREADY RESOLVED |
| 56 | cleartext-logging | apps/admin/src/main.rs:9180 | dismissed ("used in tests") | TEST-FIXTURE-ONLY | Same as #55. | ALREADY RESOLVED |
| 57 | cleartext-logging | apps/admin/src/main.rs:2965 | open | FALSE POSITIVE | `println!("{:<20} {:<16} {:<8}", u.id, u.name, ...)` in `cmd_user_list` — prints `user_id` and account `name`, both non-secret account identifiers by design (this is the `user list` command's entire purpose). | DISMISS-AS-FALSE-POSITIVE |
| 58 | cleartext-logging | apps/admin/src/main.rs:2976 | open | FALSE POSITIVE | `anyhow::anyhow!("no such user: {id}")` in `find_user_mut` — `id` is the non-secret `user_id` used to look up a user, not the flagged `vless_uuid`/token. | DISMISS-AS-FALSE-POSITIVE |
| 59 | cleartext-logging | apps/admin/src/main.rs:2892-2893 | open | FALSE POSITIVE | `id: id.clone(), name: name.to_string()` — struct field assignment in `cmd_user_create_probe`, not a print; underlying `id` is non-secret `user_id`. | DISMISS-AS-FALSE-POSITIVE |
| 60 | cleartext-logging | apps/admin/src/main.rs:2892-2893 | open | FALSE POSITIVE | Duplicate instance of #59 at the same drifted location. | DISMISS-AS-FALSE-POSITIVE |

## Notes for the human reviewer

- **#38 deserves a second look**, not because the code turned out to be a leak (it did not, per the current locations traced above), but because a prior pass flagged it as a distinct real finding and this pass disagrees. The disagreement is about what the *current* code does; if the prior pass was looking at an older commit where a real `println!` of a genuine secret existed at the probe-creation path, that code has since changed. Worth a second human pass specifically diffing what changed there.
- **Systemic pattern**: nearly every `rust/cleartext-logging` alert in this range resolves to CodeQL's taint tracker treating anything flowing from the `compat_config::credentials` module as "sensitive," including `generate_user_id()`, even though that specific function's output is deliberately a non-secret identifier (documented in-repo as such). If these are dismissed, consider whether `generate_user_id()`'s naming/module placement is itself worth revisiting to stop tripping this rule on every call site — a `credentials::generate_user_id` name reads as sensitive even though the value is not, which is presumably part of why CodeQL keeps flagging it (and worth a genuinely-human decision, not this pass's call).
- **Location drift**: a large fraction of `most_recent_instance.location` values point at lines that, in the current tree, are comments, enum variants, function signatures, or closing braces — not the sink CodeQL originally found. This means CodeQL's last analysis run (commit `77e3cf2a...` for alert #50, similar for others) predates several commits on this branch that shifted line numbers without the alert being re-scanned since. A fresh CodeQL run against the current HEAD of this branch would likely re-anchor (or auto-resolve) several of these alerts and should be considered before any bulk dismissal.
