use compat_config::store::{load_users, parse_users_bytes, save_users_atomic};
use compat_config::{CompatError, CompatUser, SecretString};

fn user(id: &str) -> CompatUser {
    CompatUser {
        id: id.to_string(),
        name: format!("user-{id}"),
        enabled: true,
        vless_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
        hysteria2_password: SecretString::new("test-hysteria2-password"),
        subscription_token_hash_hex:
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        created_at: 1_700_000_000,
        expires_at: None,
        vision_off_experiment: false,
    }
}

#[test]
fn every_strict_prefix_of_users_file_is_rejected_as_truncated_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("users.json");
    save_users_atomic(&path, &[user("u1"), user("u2")]).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert!(parse_users_bytes(&bytes).is_ok());

    for cut in 0..bytes.len() {
        assert!(
            parse_users_bytes(&bytes[..cut]).is_err(),
            "truncated users.json unexpectedly parsed at byte offset {cut}/{}",
            bytes.len()
        );
    }
}

#[test]
fn failed_pre_rename_write_preserves_the_previous_live_state_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("users.json");
    save_users_atomic(&path, &[user("old")]).unwrap();
    let before = std::fs::read(&path).unwrap();

    // save_users_atomic writes to users.json.tmp.<pid> before the atomic
    // rename. Occupy that exact path with a directory so opening the temp
    // file deterministically fails on every OS/filesystem without relying on
    // permission tricks (which are unreliable when tests run as root).
    let temp_collision = dir
        .path()
        .join(format!("users.json.tmp.{}", std::process::id()));
    std::fs::create_dir(&temp_collision).unwrap();

    let err = save_users_atomic(&path, &[user("new")]).unwrap_err();
    assert!(matches!(err, CompatError::Io(_)));

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a failed pre-rename write must never alter the last known-good users.json"
    );
    let loaded = load_users(&path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "old");
}

#[test]
fn future_users_schema_fails_closed_in_the_public_parse_boundary() {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 999,
        "users": []
    }))
    .unwrap();

    let err = parse_users_bytes(&bytes).unwrap_err();
    assert!(matches!(
        err,
        CompatError::UnsupportedSchema {
            what: "users.json",
            found: 999,
            ..
        }
    ));
}

#[test]
fn syntactically_valid_but_wrong_top_level_shape_is_not_treated_as_empty_state() {
    for input in [
        br#"{}"#.as_slice(),
        br#"{"schema_version":1}"#.as_slice(),
        br#"{"users":[]}"#.as_slice(),
        br#"null"#.as_slice(),
        br#"true"#.as_slice(),
    ] {
        assert!(
            parse_users_bytes(input).is_err(),
            "malformed state must fail closed instead of becoming an empty user database: {}",
            String::from_utf8_lossy(input)
        );
    }
}
