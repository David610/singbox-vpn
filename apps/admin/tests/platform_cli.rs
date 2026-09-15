//! `vpn-admin` platform v2 commands against persisted state: transports,
//! AmneziaWG credential lifecycle, endpoints and nodes. Level 2
//! (integration) evidence; the live AmneziaWG data-plane proof is
//! `crates/compat-config/tests/amneziawg_interop.rs`.

#![cfg(unix)]

use assert_cmd::Command;
use std::path::{Path, PathBuf};

mod support;

const REALITY_PRIVATE: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
const REALITY_PUBLIC: &str = "pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk";

fn toml_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn write_deployment(dir: &Path) -> PathBuf {
    let singbox = dir.join("fake-sing-box.sh");
    executable(
        &singbox,
        &format!(
            "#!/usr/bin/env bash\ncase \"$1\" in\n  generate) echo \"PrivateKey: {REALITY_PRIVATE}\"; echo \"PublicKey: {REALITY_PUBLIC}\"; exit 0 ;;\n  check) exit 0 ;;\n  version) echo 'sing-box test-fake 1.0.0'; exit 0 ;;\nesac\nexit 1\n"
        ),
    );
    let text = format!(
        "# operator comment\nschema_version = 2\nnode_id = \"fi1\"\nrole = \"exit\"\npublic_host = \"fi1.example.test\"\nsubscription_host = \"fi1.example.test\"\nstate_dir = \"{state}\"\nsingbox_binary = \"{singbox}\"\n\n[reality]\nlisten_port = 443\nhandshake_server = \"www.example.com\"\n\n[hysteria2]\nlisten_port = 443\n\n[subscription]\nlisten_port = 9100\n",
        state = toml_path(&dir.join("state")),
        singbox = toml_path(&singbox),
    );
    let cfg = dir.join("deployment.toml");
    std::fs::write(&cfg, text).unwrap();
    cfg
}

/// A fake `awg` modelling a running interface: `syncconf` stores the file,
/// `show <if> peers` reports its peers. With `stale`, syncconf is ignored,
/// so the live peer set never changes.
fn fake_awg(dir: &Path, stale: bool) -> PathBuf {
    let path = dir.join("fake-awg.sh");
    let live = dir.join("fake-awg-live.conf");
    let calls = dir.join("fake-awg-calls.log");
    let sync = if stale { ":" } else { "cp \"$3\" \"$LIVE\"" };
    executable(
        &path,
        &format!(
            "#!/usr/bin/env bash\nLIVE='{live}'\necho \"$*\" >> '{calls}'\ncase \"$1 $3\" in\n  'show ') touch \"$LIVE\"; exit 0 ;;\n  'show peers') grep '^PublicKey = ' \"$LIVE\" 2>/dev/null | sed 's/^PublicKey = //'; exit 0 ;;\n  'show latest-handshakes') exit 0 ;;\nesac\nif [ \"$1\" = syncconf ]; then {sync}; exit 0; fi\nexit 1\n",
            live = live.display(),
            calls = calls.display(),
        ),
    );
    path
}

fn admin(dir: &Path, cfg: &Path) -> Command {
    let mut cmd = support::vpn_admin();
    cmd.arg("--config").arg(cfg);
    cmd.current_dir(dir);
    cmd.env("SINGBOX_VPN_ALLOW_OFFLINE_MUTATION", "1");
    cmd.env("SINGBOX_VPN_LOCK_PATH", dir.join("state.lock"));
    cmd
}

fn out(assert: &assert_cmd::assert::Assert) -> String {
    let o = assert.get_output();
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn create_user(dir: &Path, cfg: &Path, name: &str) -> String {
    let text = String::from_utf8(
        admin(dir, cfg)
            .args(["user", "create", "--name", name, "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["id"].as_str().unwrap().to_string()
}

fn users_json(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("state/users/users.json")).unwrap()).unwrap()
}

fn setconf(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("state/amneziawg/awg0.conf")).unwrap()
}

fn secrets_in(dir: &Path) -> Vec<String> {
    let mut s = vec![std::fs::read_to_string(dir.join("state/amneziawg/server.key")).unwrap().trim().to_string()];
    let params: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("state/amneziawg/params.json")).unwrap()).unwrap();
    s.push(params["header_protection_key"].as_str().unwrap().to_string());
    for u in users_json(dir)["users"].as_array().unwrap() {
        if let Some(a) = u.get("amneziawg") {
            s.push(a["private_key"].as_str().unwrap().to_string());
            s.push(a["preshared_key"].as_str().unwrap().to_string());
        }
    }
    s
}

fn setup() -> (tempfile::TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path());
    admin(dir.path(), &cfg).arg("init").assert().success();
    let alice = create_user(dir.path(), &cfg, "alice");
    (dir, cfg, alice)
}

#[test]
fn enabling_amneziawg_is_schema_guarded_issues_credentials_and_renders_peers() {
    let (dir, cfg, alice) = setup();
    let d = dir.path();
    let status = out(&admin(d, &cfg).args(["transport", "status"]).assert().success());
    assert!(status.contains("amneziawg      disabled"), "{status}");

    let enable = out(&admin(d, &cfg).args(["transport", "enable", "amneziawg", "--subnet-v4", "10.77.0.0/24"]).assert().success());
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("schema_version = 3") && text.contains("[amneziawg]"));
    assert!(text.contains("# operator comment"), "operator comments must survive");
    assert_eq!(users_json(d)["schema_version"], 2);
    let alice_awg = &users_json(d)["users"][0]["amneziawg"];
    assert_eq!(alice_awg["address_v4"], "10.77.0.2");
    let conf = setconf(d);
    assert!(conf.contains(alice_awg["public_key"].as_str().unwrap()));
    assert!(conf.contains(&format!("# {alice}")));

    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &str| std::fs::metadata(d.join(p)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode("state/amneziawg/server.key"), 0o600);
    assert_eq!(mode("state/amneziawg/awg0.conf"), 0o600);
    assert_eq!(mode("state/amneziawg/params.json"), 0o640);

    for secret in secrets_in(d) {
        assert!(!enable.contains(&secret), "enable printed a secret");
    }

    // Users created afterwards get a credential automatically.
    create_user(d, &cfg, "bob");
    assert_eq!(users_json(d)["users"][1]["amneziawg"]["address_v4"], "10.77.0.3");

    let validate = out(&admin(d, &cfg).args(["config", "validate"]).assert().success());
    assert!(validate.contains("capability: amneziawg-3 (enabled on this node: true)"), "{validate}");
    assert!(validate.contains("CURRENT (schema_version 3"), "{validate}");

    admin(d, &cfg).args(["transport", "enable", "amneziawg"]).assert().failure();
    admin(d, &cfg).args(["transport", "enable", "vless-reality"]).assert().failure();

    let status = out(&admin(d, &cfg).args(["transport", "status", "--json"]).assert().success());
    let rows: serde_json::Value = serde_json::from_str(status.trim()).unwrap();
    let awg = rows.as_array().unwrap().iter().find(|r| r["transport"] == "amneziawg").unwrap();
    assert_eq!(awg["users_with_credentials"], 2);
    assert_eq!(awg["interface_running"], false);
}

#[test]
fn credential_lifecycle_reaches_the_rendered_data_plane() {
    let (dir, cfg, alice) = setup();
    let d = dir.path();
    admin(d, &cfg).args(["transport", "enable", "amneziawg"]).assert().success();
    let key = |d: &Path| users_json(d)["users"][0]["amneziawg"]["public_key"].as_str().map(str::to_string);
    let first = key(d).unwrap();

    admin(d, &cfg).args(["user", "awg", "rotate", &alice]).assert().success();
    let rotated = key(d).unwrap();
    assert_ne!(first, rotated);
    assert!(!setconf(d).contains(&first) && setconf(d).contains(&rotated));

    admin(d, &cfg).args(["user", "awg", "revoke", &alice]).assert().success();
    assert!(key(d).is_none());
    assert!(!setconf(d).contains("[Peer]"));
    assert_eq!(users_json(d)["schema_version"], 1, "no AWG credential left: schema 1");
    admin(d, &cfg).args(["user", "awg", "revoke", &alice]).assert().failure();

    admin(d, &cfg).args(["user", "awg", "issue", &alice]).assert().success();
    admin(d, &cfg).args(["user", "awg", "issue", &alice]).assert().failure();
    let reissued = key(d).unwrap();
    assert!(setconf(d).contains(&reissued));

    // Disabling the user removes the peer without deleting the credential.
    admin(d, &cfg).args(["user", "disable", &alice]).assert().success();
    assert!(!setconf(d).contains(&reissued));
    admin(d, &cfg).args(["user", "enable", &alice]).assert().success();
    assert!(setconf(d).contains(&reissued));

    let show = out(&admin(d, &cfg).args(["user", "awg", "show", &alice]).assert().success());
    assert!(show.starts_with("[Interface]\nPrivateKey = "));
    assert!(show.contains("Endpoint = fi1.example.test:51820"));
    let suppressed = out(&admin(d, &cfg)
        .env("SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS", "1")
        .args(["user", "awg", "show", &alice])
        .assert()
        .success());
    assert!(!suppressed.contains("PrivateKey"));
}

#[test]
fn live_apply_is_verified_and_a_mismatch_fails_closed() {
    let (dir, cfg, alice) = setup();
    let d = dir.path();
    let good = fake_awg(d, false);
    admin(d, &cfg).env("SINGBOX_VPN_AWG", &good).args(["transport", "enable", "amneziawg"]).assert().success();
    let calls = std::fs::read_to_string(d.join("fake-awg-calls.log")).unwrap();
    assert!(calls.contains("syncconf awg0"), "{calls}");

    // A data plane that does not accept the change must leave users.json untouched.
    let stale_dir = d.join("stale");
    std::fs::create_dir_all(&stale_dir).unwrap();
    std::fs::copy(d.join("fake-awg-live.conf"), stale_dir.join("fake-awg-live.conf")).unwrap();
    let stale = fake_awg(&stale_dir, true);
    let before = std::fs::read(d.join("state/users/users.json")).unwrap();
    let failure = out(&admin(d, &cfg).env("SINGBOX_VPN_AWG", &stale).args(["user", "awg", "revoke", &alice]).assert().failure());
    assert!(failure.contains("does not match users.json"), "{failure}");
    assert_eq!(std::fs::read(d.join("state/users/users.json")).unwrap(), before);

    admin(d, &cfg).env("SINGBOX_VPN_AWG", &good).args(["user", "awg", "revoke", &alice]).assert().success();
    assert!(!std::fs::read_to_string(d.join("fake-awg-live.conf")).unwrap().contains("[Peer]"));
}

#[test]
fn endpoints_and_nodes_are_listed_without_secrets() {
    let (dir, cfg, alice) = setup();
    let d = dir.path();
    admin(d, &cfg).args(["transport", "enable", "amneziawg"]).assert().success();
    let secrets = secrets_in(d);
    let uuid = users_json(d)["users"][0]["vless_uuid"].as_str().unwrap().to_string();

    let endpoints = out(&admin(d, &cfg).args(["endpoint", "list"]).assert().success());
    assert!(endpoints.contains("amneziawg-1") && endpoints.contains("reality-1"), "{endpoints}");
    let routes = out(&admin(d, &cfg).args(["endpoint", "list", "--user", &alice, "--json"]).assert().success());
    let catalog: serde_json::Value = serde_json::from_str(routes.trim()).unwrap();
    let ids: Vec<&str> = catalog["routes"].as_array().unwrap().iter().map(|r| r["route_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["fi1/vless-reality", "fi1/hysteria2", "fi1/amneziawg"]);
    let nodes = out(&admin(d, &cfg).args(["node", "list", "--json"]).assert().success());
    for text in [&endpoints, &routes, &nodes] {
        assert!(!text.contains(&uuid));
        for s in &secrets {
            assert!(!text.contains(s), "secret printed");
        }
    }
}

#[test]
fn backup_and_restore_carry_amneziawg_state_all_or_nothing() {
    let (dir, cfg, _alice) = setup();
    let d = dir.path();
    admin(d, &cfg).args(["transport", "enable", "amneziawg"]).assert().success();
    let archive = d.join("backup.tar");
    admin(d, &cfg).args(["backup", "--output"]).arg(&archive).assert().success();
    let public = std::fs::read_to_string(d.join("state/amneziawg/server.pub")).unwrap();
    let users_before = users_json(d);

    // Lose the node state, then restore it.
    std::fs::remove_dir_all(d.join("state/amneziawg")).unwrap();
    std::fs::remove_file(d.join("state/users/users.json")).unwrap();
    admin(d, &cfg).arg("restore").arg(&archive).assert().success();
    assert_eq!(std::fs::read_to_string(d.join("state/amneziawg/server.pub")).unwrap(), public);
    assert_eq!(users_json(d)["users"][0]["amneziawg"], users_before["users"][0]["amneziawg"]);
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(d.join("state/amneziawg/server.key")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "restored AmneziaWG server key must stay root-only");
    assert!(setconf(d).contains(users_before["users"][0]["amneziawg"]["public_key"].as_str().unwrap()));

    // An archive with the key but without its parameters is refused.
    let partial = d.join("partial.tar");
    {
        let mut src = tar::Archive::new(std::fs::File::open(&archive).unwrap());
        let mut dst = tar::Builder::new(std::fs::File::create(&partial).unwrap());
        for entry in src.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().into_owned();
            if path.ends_with("params.json") {
                continue;
            }
            let mut header = entry.header().clone();
            dst.append_data(&mut header, &path, &mut entry).unwrap();
        }
        dst.finish().unwrap();
    }
    let refused = out(&admin(d, &cfg).arg("restore").arg(&partial).assert().failure());
    assert!(refused.contains("incomplete AmneziaWG state"), "{refused}");
}

#[test]
fn rotation_requires_confirmation_and_disable_restores_old_schemas() {
    let (dir, cfg, _alice) = setup();
    let d = dir.path();
    admin(d, &cfg).args(["transport", "enable", "amneziawg"]).assert().success();
    let pub1 = std::fs::read_to_string(d.join("state/amneziawg/server.pub")).unwrap();
    admin(d, &cfg).args(["transport", "rotate", "amneziawg"]).assert().failure();
    admin(d, &cfg).args(["transport", "rotate", "amneziawg", "--yes"]).assert().success();
    assert_ne!(pub1, std::fs::read_to_string(d.join("state/amneziawg/server.pub")).unwrap());

    let plan = out(&admin(d, &cfg).args(["transport", "plan", "amneziawg"]).assert().success());
    assert!(plan.contains("b5928efb6ca19f0153958460c3d141f04abc5c2e"), "{plan}");

    admin(d, &cfg).args(["transport", "disable", "amneziawg", "--purge-credentials"]).assert().success();
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("schema_version = 2") && !text.contains("[amneziawg]"));
    assert!(text.contains("# operator comment"));
    assert_eq!(users_json(d)["schema_version"], 1);
    let validate = out(&admin(d, &cfg).args(["config", "validate"]).assert().success());
    assert!(validate.contains("enabled on this node: false"));
    admin(d, &cfg).args(["transport", "disable", "amneziawg"]).assert().failure();
    admin(d, &cfg).args(["user", "awg", "issue", "--all"]).assert().failure();
}
