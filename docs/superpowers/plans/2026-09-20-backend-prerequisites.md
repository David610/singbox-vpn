# singbox-vpn backend prerequisites for vpn-web — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two `singbox-vpn` backend capabilities the vpn-web MVP depends on: a `set-expiry`/`clear-expiry` CLI command (subscriptions renew; today only `user create` takes an expiry) and a base64-encoded subscription body for import-by-URL clients (fixes the confirmed Shadowrocket import failure).

**Architecture:** Both changes follow this repo's existing mutating-command pattern exactly (`cmd_user_set_enabled` for set-expiry: load → mutate → `apply_users_and_save` → print) and existing query-parameter pattern (`compat` for the new `encoding` parameter: validated up front, rejected loudly if combined with an incompatible format, byte-identical output when absent). No new crates, no schema changes, no changes to `format=singbox` or the provisioning contract.

**Tech Stack:** Rust, clap (CLI), axum (HTTP), existing `compat-config`/`common` crates, `base64` crate (already a dependency of `compat-config`).

**Spec:** `docs/superpowers/specs/2026-09-20-vpn-website-mvp-design.md` §5 (items 1 and 2).

## Global Constraints

- Every mutating CLI command must go through `apply_users_and_save` (validate → apply to sing-box → save `users.json`), never write `users.json` directly — this is what makes revocation fail-closed (spec §5 item 1 references the existing `apps/admin` locking/atomic-render discipline).
- `format=uri`/`format=hiddify` output with no `encoding` parameter must remain byte-for-byte identical to today (existing test `default_and_existing_format_outputs_are_byte_for_byte_unchanged` in `services/subscription/src/lib.rs` must keep passing unmodified).
- No secret (VLESS UUID, Hysteria2 password, subscription token, REALITY private key) may appear in a log line — follow the existing pattern in `services/subscription/src/lib.rs`'s `tracing::debug!` calls (user id only, never the token).
- New CLI flags/query values follow existing naming: kebab-case CLI subcommands (`set-expiry`, `clear-expiry`), lower-case query values (`encoding=base64`).

---

## Task 1: `vpn-admin user set-expiry` / `clear-expiry`

**Files:**
- Modify: `apps/admin/src/main.rs` (add two `UserCommands` variants, two `main()` match arms, one new `cmd_user_set_expiry` function, placed immediately after `cmd_user_set_enabled` at line 2424)
- Test: `apps/admin/tests/cli.rs` (extend the existing user-lifecycle integration test)

**Interfaces:**
- Consumes: `find_user_mut(&mut users, id) -> Result<&mut CompatUser>` (existing, `apps/admin/src/main.rs:2417`), `apply_users_and_save(cfg, &previous_users, &users) -> Result<bool>` (existing, `apps/admin/src/main.rs:3060`), `CompatUser.expires_at: Option<i64>` (existing field).
- Produces: `cmd_user_set_expiry(cfg: &DeploymentConfig, id: &str, expires_at: Option<i64>) -> Result<()>` — later tasks (the provisioning agent, out of scope for this plan) will shell out to `vpn-admin user set-expiry <id> --expires-at <unix_seconds>` and `vpn-admin user clear-expiry <id>`.

- [ ] **Step 1: Add the failing CLI integration test**

Open `apps/admin/tests/cli.rs` and find the block around line 467-472 (the `// re-enable.` block, right after the disable/enable test in the main lifecycle test). Insert this immediately after it, before the `// rotate-token prints a fresh URL.` comment:

```rust
    // set-expiry takes effect in the store and is reported back.
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "set-expiry", &user_id, "--expires-at", "4102444800"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&format!("{user_id}: expires_at=4102444800")));

    // clear-expiry removes it again.
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "clear-expiry", &user_id])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&format!("{user_id}: expires_at=none")));

    // set-expiry on an unknown user fails cleanly, same as other user
    // subcommands.
    admin(dir.path(), &cfg_path)
        .args(["user", "set-expiry", "no-such-user", "--expires-at", "1"])
        .assert()
        .failure();
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p admin --test cli user_lifecycle -- --nocapture` (use the actual test function name containing the block above if different — grep `apps/admin/tests/cli.rs` for `fn ` near line 400 to confirm; if the surrounding test has a different name, target that name instead of `user_lifecycle`)

Expected: FAIL — `error: unrecognized subcommand 'set-expiry'` (clap rejects the unknown subcommand), because `UserCommands::SetExpiry` does not exist yet.

- [ ] **Step 3: Add the `SetExpiry` and `ClearExpiry` variants to `UserCommands`**

In `apps/admin/src/main.rs`, in the `enum UserCommands` block, insert immediately after the `Disable { user_id: String }` variant (currently at line 355-357):

```rust
    /// Set (or replace) this user's expiry. Used for subscription
    /// renewals — unlike `create`, which only takes an expiry at
    /// account-creation time, this is the only way to move an existing
    /// user's expiry forward (or into the past, which immediately makes
    /// `CompatUser::is_active` treat them as expired). Same
    /// validate-apply-reload path as `enable`/`disable`.
    SetExpiry {
        user_id: String,
        /// Unix-seconds expiry timestamp.
        #[arg(long)]
        expires_at: i64,
    },
    /// Remove this user's expiry entirely (they no longer expire on
    /// their own; `disable`/`remove` are still separate, explicit
    /// actions).
    ClearExpiry {
        user_id: String,
    },
```

- [ ] **Step 4: Wire the two new variants into `main()`'s match**

In `apps/admin/src/main.rs`, immediately after the existing arm

```rust
        Commands::User(UserCommands::Disable { user_id }) => {
            cmd_user_set_enabled(&cfg, &user_id, false)
        }
```

(around line 525-527), add:

```rust
        Commands::User(UserCommands::SetExpiry {
            user_id,
            expires_at,
        }) => cmd_user_set_expiry(&cfg, &user_id, Some(expires_at)),
        Commands::User(UserCommands::ClearExpiry { user_id }) => {
            cmd_user_set_expiry(&cfg, &user_id, None)
        }
```

Also add both new variants to the `command_mutates_state` exclusion check's *complement* — i.e. do nothing there: `command_mutates_state` (line 453-464) excludes specific read-only commands and defaults every other command to "mutates state, take the lock." `SetExpiry`/`ClearExpiry` are mutating, so they correctly fall through to `true` with no change needed — just confirm (read, don't edit) that neither variant appears in that `matches!` list.

- [ ] **Step 5: Implement `cmd_user_set_expiry`**

In `apps/admin/src/main.rs`, immediately after `cmd_user_set_enabled`'s closing brace (search for the end of that function, it starts at line 2424), add:

```rust
/// Same shape as `cmd_user_set_enabled`: load, mutate one field, then the
/// standard validate-apply-reload-save path. `expires_at: None` clears the
/// expiry (the user no longer expires on their own); `Some(t)` sets/replaces
/// it, including to a value in the past, which `CompatUser::is_active`
/// immediately treats as expired — same effective blast radius as
/// `disable`, but expressed through the existing expiry mechanism rather
/// than a second flag.
fn cmd_user_set_expiry(cfg: &DeploymentConfig, id: &str, expires_at: Option<i64>) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    let previous_users = users.clone();
    find_user_mut(&mut users, id)?.expires_at = expires_at;
    apply_users_and_save(cfg, &previous_users, &users)?;
    match expires_at {
        Some(t) => println!("{id}: expires_at={t}"),
        None => println!("{id}: expires_at=none"),
    }
    Ok(())
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p admin --test cli` (the whole file — the existing lifecycle test that Step 1 extended must still pass end to end)

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/admin/src/main.rs apps/admin/tests/cli.rs
git commit -m "Add vpn-admin user set-expiry/clear-expiry commands

Subscriptions renew, and until now only \`user create\` could set an
expiry — there was no way to move an existing user's expiry forward
on renewal short of remove+recreate. Follows the same
validate-apply-reload path as enable/disable."
```

---

## Task 2: Base64-encoded subscription body for import-by-URL clients

**Files:**
- Modify: `crates/compat-config/src/render.rs` (add `to_base64_subscription`)
- Modify: `services/subscription/src/lib.rs` (add `encoding` query param + response wrapping)
- Test: `crates/compat-config/src/render.rs` (unit test, inline `#[cfg(test)] mod tests`), `services/subscription/src/lib.rs` (integration tests, inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing new — wraps the existing `String` bodies `render::render_uri_list` and `render::render_vision_off_uri_list` already produce.
- Produces: `compat_config::render::to_base64_subscription(body: &str) -> String`. Later work (out of scope here) — the website's setup instructions and `vpn-admin user create`'s printed URLs — can start advertising `&encoding=base64` once this is verified against a real Shadowrocket import per the spec's acceptance criteria; that verification step itself is also out of scope for this plan (it needs a real deployed VPS).

- [ ] **Step 1: Add the failing unit test for the encoding helper**

In `crates/compat-config/src/render.rs`, find the `#[cfg(test)] mod tests` block (search for `mod tests` near the end of the file) and add:

```rust
    #[test]
    fn to_base64_subscription_round_trips_and_matches_known_vector() {
        let input = "vless://uuid@host:443?x=1#label\nhysteria2://pw@host:443?y=2#label";
        let encoded = to_base64_subscription(input);
        // Standard (not URL-safe) base64 with padding — the conventional
        // V2Ray/Shadowrocket subscription encoding.
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let decoded = STANDARD.decode(&encoded).expect("valid standard base64");
        assert_eq!(String::from_utf8(decoded).unwrap(), input);
    }

    #[test]
    fn to_base64_subscription_of_empty_string_is_empty() {
        assert_eq!(to_base64_subscription(""), "");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p compat-config to_base64_subscription`

Expected: FAIL with `cannot find function \`to_base64_subscription\` in this scope`.

- [ ] **Step 3: Implement `to_base64_subscription`**

In `crates/compat-config/src/render.rs`, near the top-level `pub fn render_uri_list` (the function the new helper wraps output from — add this new function directly above or below it), add:

```rust
/// Base64-encode a subscription body (standard alphabet, padded) — the
/// conventional V2Ray/Shadowrocket subscription-import encoding. Several
/// import-by-URL clients (Shadowrocket confirmed) expect the response body
/// of a subscription URL to be base64, not raw share-link text; the plain
/// `format=uri`/`format=hiddify` body stays unchanged for every existing
/// consumer, and this is applied only when a caller opts in via
/// `?encoding=base64` (see `services/subscription`). Not used for
/// `format=singbox`, which is JSON, not share-link text.
pub fn to_base64_subscription(body: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.encode(body)
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p compat-config to_base64_subscription`

Expected: PASS.

- [ ] **Step 5: Add the failing integration tests for the `encoding` query parameter**

In `services/subscription/src/lib.rs`, inside `#[cfg(test)] mod tests` (starts around line 740), add these tests near the other `format=uri`/`compat` tests (e.g. right after `uri_format_returns_plaintext_share_links`):

```rust
    #[tokio::test]
    async fn uri_format_with_encoding_base64_returns_decodable_body() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let plain_resp = oneshot_with_addr(state.clone(), "/sub/goodtoken?format=uri").await;
        let plain_body = axum::body::to_bytes(plain_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let b64_resp =
            oneshot_with_addr(state, "/sub/goodtoken?format=uri&encoding=base64").await;
        assert_eq!(b64_resp.status(), StatusCode::OK);
        let b64_body = axum::body::to_bytes(b64_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let decoded = STANDARD
            .decode(&b64_body)
            .expect("encoding=base64 body must be valid standard base64");
        assert_eq!(
            decoded, plain_body,
            "decoding the base64 body must reproduce exactly the format=uri plain body"
        );
    }

    #[tokio::test]
    async fn hiddify_format_with_encoding_base64_returns_decodable_body() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp =
            oneshot_with_addr(state, "/sub/goodtoken?format=hiddify&encoding=base64").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let decoded = STANDARD.decode(&body).expect("valid standard base64");
        assert!(String::from_utf8(decoded).unwrap().starts_with("vless://"));
    }

    #[tokio::test]
    async fn vision_off_with_encoding_base64_returns_decodable_body_without_flow() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(
            state,
            "/sub/goodtoken?format=uri&compat=vision-off&encoding=base64",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let decoded = STANDARD.decode(&body).expect("valid standard base64");
        let s = String::from_utf8(decoded).unwrap();
        let reality_line = s.lines().find(|l| l.starts_with("vless://")).unwrap();
        assert!(!reality_line.contains("flow="));
    }

    #[tokio::test]
    async fn unknown_encoding_value_returns_400() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(state, "/sub/goodtoken?format=uri&encoding=garbage").await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn encoding_base64_with_singbox_format_is_rejected_not_silently_ignored() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp =
            oneshot_with_addr(state, "/sub/goodtoken?format=singbox&encoding=base64").await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn absent_encoding_leaves_uri_format_byte_identical_to_before() {
        // Regression guard: adding the encoding parameter must not change
        // the default (no `encoding`) behavior at all.
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(state, "/sub/goodtoken?format=uri").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let s = String::from_utf8(body.to_vec()).unwrap();
        assert!(s.starts_with("vless://"));
    }
```

- [ ] **Step 6: Run the new tests to verify they fail**

Run: `cargo test -p subscription encoding_base64 && cargo test -p subscription unknown_encoding && cargo test -p subscription uri_format_with_encoding && cargo test -p subscription hiddify_format_with_encoding && cargo test -p subscription vision_off_with_encoding`

Expected: FAIL — `SubQuery` has no field `encoding`, so these requests currently 400 with axum's query-deserialize rejection (`unknown field encoding`) rather than the intended, more specific behavior; the `absent_encoding...` test should already pass since it exercises no new code.

- [ ] **Step 7: Add the `encoding` field to `SubQuery`**

In `services/subscription/src/lib.rs`, inside `pub struct SubQuery` (starts at line 146), add this field after `compat`:

```rust
    /// Opt-in body transform for `format=uri`/`format=hiddify` only:
    /// `base64` returns the same share-link text, standard-base64-encoded
    /// (conventional V2Ray/Shadowrocket subscription encoding). Several
    /// import-by-URL clients (Shadowrocket confirmed) fail to import the
    /// plain-text body `format=uri` has always returned; existing
    /// consumers that already parse the plain body are unaffected because
    /// this is opt-in and the plain body is unchanged when `encoding` is
    /// absent. Not supported with `format=singbox` (JSON, not share-link
    /// text) — rejected explicitly rather than silently ignored, same
    /// rule as `compat`'s format-specific guards below. Any value other
    /// than `base64` is rejected (400) rather than silently falling back.
    pub encoding: Option<String>,
```

- [ ] **Step 8: Validate `encoding` and reject the `format=singbox` combination**

In `services/subscription/src/lib.rs`, in `get_subscription`, immediately after the existing `compat_mode` computation block ends (right after the closing `};` that follows the `compat_mode` match, before `match format {` at line 295), add:

```rust
    let want_base64 = match query.encoding.as_deref() {
        None => false,
        Some("base64") => true,
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "unknown encoding value (expected \"base64\")",
            )
                .into_response()
        }
    };
    if want_base64 && format == "singbox" {
        return (
            StatusCode::BAD_REQUEST,
            "encoding=base64 is only supported with format=uri/hiddify — format=singbox \
             already returns JSON, which no importer expects base64-wrapped",
        )
            .into_response();
    }
```

- [ ] **Step 9: Apply the encoding to both `Ok(body) =>` success arms in the `"uri" | "hiddify"` match branch**

In `services/subscription/src/lib.rs`, in the `"uri" | "hiddify" =>` arm, there are two `Ok(body) => ( StatusCode::OK, [("content-type", "text/plain; charset=utf-8")], body, ).into_response(),` occurrences — one inside the `if compat_mode == render::CompatibilityMode::VisionOff` block, one in the final `match render::render_uri_list(&user, &share_endpoints)`. Replace **both** occurrences of the bare `body` third tuple element with:

```rust
                Ok(body) => {
                    let body = if want_base64 {
                        compat_config::render::to_base64_subscription(&body)
                    } else {
                        body
                    };
                    (
                        StatusCode::OK,
                        [("content-type", "text/plain; charset=utf-8")],
                        body,
                    )
                        .into_response()
                }
```

(This replaces the existing `Ok(body) => ( StatusCode::OK, [...], body, ).into_response(),` line in each of the two places — the `Err(e) => { ... }` arms directly below each are unchanged.)

- [ ] **Step 10: Run the tests to verify they pass**

Run: `cargo test -p subscription`

Expected: PASS — every test in the file, including the pre-existing ones (`default_and_existing_format_outputs_are_byte_for_byte_unchanged` especially, to confirm no regression to the unencoded path).

- [ ] **Step 11: Run the full workspace test suite**

Run: `cargo test --workspace`

Expected: PASS. This catches any other test (e.g. in `apps/admin` doctor/report paths) that might assert on `SubQuery`'s field set or on subscription-service response shape.

- [ ] **Step 12: Commit**

```bash
git add crates/compat-config/src/render.rs services/subscription/src/lib.rs
git commit -m "Add opt-in base64 encoding for format=uri/hiddify subscription bodies

Shadowrocket (confirmed) and other import-by-URL clients expect a
subscription URL's body to be base64-encoded, not raw share-link
text. format=uri/hiddify's default (no encoding param) output is
byte-for-byte unchanged; ?encoding=base64 opts a caller into the
conventional V2Ray/Shadowrocket encoding. Rejected explicitly when
combined with format=singbox (JSON, not share-link text)."
```

---

## Explicitly not in this plan

- Advertising `&encoding=base64` anywhere (e.g. in `vpn-admin user create`'s printed output, or the eventual vpn-web setup instructions) — that requires the real-Shadowrocket verification called out in the spec's acceptance criteria (§11), which needs a real deployed VPS and a real device, not something this plan can produce.
- The vpn-web repo itself (Supabase schema, Stripe webhook, provisioning agent, frontend) — separate plan(s), tracked next per the spec's §3/§4/§6.
- Any capacity/load testing for ~100 concurrent users (spec §9 item 5) — needs a real VPS.
