//! Installed-release identity check for `vpn-admin doctor`.
//!
//! The supported installer/updater records, in the root-owned install-state
//! manifest, which release it installed and the SHA-256 of the executables it
//! placed in `/usr/local/bin`. Copying a development build over them (the
//! "manual binary swap" this check exists for) leaves the manifest naming the
//! old release while the binaries are something else, and both report the
//! same Cargo version. Comparing the bytes with what the installer recorded
//! is the only reliable way to tell the cases apart, and it needs no
//! build-time identity embedded in the binary — so release artifacts stay
//! byte-reproducible and no wire format changes.
//!
//! What a match does and does not mean: the hashes are the ones the installer
//! computed from the executables it installed. A match says "these are still the
//! bytes the installer put here". It is NOT independent proof that those bytes
//! are the published artifact (compare `SHA256SUMS`/the attestation for that),
//! and it says nothing about unit files, helper scripts or templates, which are
//! not hashed.
//!
//! The running process is checked too: `doctor` may be started from a
//! development build in a build directory while the installed copies are
//! genuine, and reporting the installed copies as "this build" would be false.
//!
//! Classes reported: the release the installer RECORDED (stable, candidate or
//! development channel), an inconsistent/partial deployment, or UNKNOWN for a
//! host installed before recording existed (never reported as a match).

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const INSTALL_STATE_MANIFEST: &str = "/var/lib/singbox-vpn/install-state.json";
pub const BIN_DIR: &str = "/usr/local/bin";
/// Set by `update.sh` around its pre-commit verification: the new binaries are
/// already in place but the manifest is only committed afterwards, so a
/// mismatch at that moment is expected, not an inconsistency.
pub const PENDING_ENV: &str = "SINGBOX_VPN_RELEASE_IDENTITY_PENDING";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Info,
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseClass {
    Stable,
    ReleaseCandidate,
    Development,
}

impl ReleaseClass {
    fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable release",
            Self::ReleaseCandidate => "release candidate",
            Self::Development => "development channel (not a published release)",
        }
    }
}

/// `vX.Y.Z` is stable, `vX.Y.Z-rc.N` (any prerelease suffix) is a candidate;
/// anything else (`main`, a branch, empty) is the development channel.
pub fn classify(tag: &str) -> ReleaseClass {
    match release_core(tag) {
        Some(_) if tag.contains('-') => ReleaseClass::ReleaseCandidate,
        Some(_) => ReleaseClass::Stable,
        None => ReleaseClass::Development,
    }
}

/// The SemVer core (`X.Y.Z`) of a release tag, or `None` when `tag` is not a
/// `vX.Y.Z[-suffix]` tag.
pub fn release_core(tag: &str) -> Option<&str> {
    let rest = tag.strip_prefix('v')?;
    let core = rest.split(['-', '+']).next()?;
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = |p: &&str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    (parts.len() == 3 && parts.iter().all(numeric)).then_some(core)
}

/// (executable name in the bin dir, manifest key holding its recorded SHA-256)
const RECORDED: [(&str, &str); 3] = [
    ("vpn-admin", "vpn_admin_sha256"),
    ("vpn", "vpn_admin_sha256"),
    ("vpn-subscription-svc", "subscription_sha256"),
];

pub fn sha256_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// The executable that is actually running this check.
pub struct RunningExecutable {
    /// SHA-256 of the running executable's bytes (`None` if unreadable).
    pub sha256: Option<String>,
}

const ADVICE: &str = "Restore a consistent deployment with the supported updater for the \
release you intend to run (`update.sh --version <tag>`, from a tree that release shipped); \
never copy executables by hand. Do NOT use `update.sh --repair` when the recorded release is \
older than this vpn-admin or your configuration needs a newer one (for example an [egress] \
policy): repair re-installs the recorded, older executables.";

/// Pure decision logic. `hash_of` returns the SHA-256 of the named installed
/// executable, or `None` when it is missing/unreadable; `running` describes the
/// process executing this check.
pub fn evaluate(
    manifest_text: Option<&str>,
    running_core: &str,
    hash_of: impl Fn(&str) -> Option<String>,
    running: &RunningExecutable,
    update_in_progress: bool,
) -> (Verdict, String) {
    let Some(text) = manifest_text else {
        return (
            Verdict::Warn,
            "release identity UNKNOWN: no install-state manifest, so the installed release \
             cannot be told from a development build or a manual copy (not installed by the \
             supported installer, or the manifest was removed)"
                .to_string(),
        );
    };
    let Ok(manifest) = serde_json::from_str::<Value>(text) else {
        return (
            Verdict::Warn,
            "release identity UNKNOWN: the install-state manifest is not valid JSON".to_string(),
        );
    };
    let tag = manifest["singbox_vpn_version"].as_str().unwrap_or("");
    let class = classify(tag);
    let shown = if tag.is_empty() {
        "<none recorded>"
    } else {
        tag
    };
    if update_in_progress {
        return (
            Verdict::Info,
            format!(
                "release identity: update in progress (recorded {shown}); the new binaries are \
                 verified against the manifest once the update commits"
            ),
        );
    }

    let mut problems: Vec<String> = Vec::new();
    if class != ReleaseClass::Development {
        if let Some(core) = release_core(tag) {
            if core != running_core {
                problems.push(format!(
                    "this vpn-admin is {running_core} but the install-state manifest records \
                     {shown}: a different release's executables are installed"
                ));
            }
        }
    }

    let recorded_admin = manifest["vpn_admin_sha256"]
        .as_str()
        .filter(|h| !h.is_empty());
    // The process running this very check must be the recorded build, not just
    // the files next to it.
    if let (Some(recorded), Some(running_sha)) = (recorded_admin, running.sha256.as_deref()) {
        if !recorded.eq_ignore_ascii_case(running_sha) {
            problems.push(
                "the vpn-admin executable running this check is NOT the recorded build (a \
                 development or manually started binary, not the installed release)"
                    .to_string(),
            );
        }
    }

    let mut unrecorded: Vec<&str> = Vec::new();
    let mut matched = 0usize;
    for (file, key) in RECORDED {
        let recorded = manifest[key].as_str().filter(|h| !h.is_empty());
        let actual = hash_of(file);
        match (recorded, actual) {
            (_, None) => problems.push(format!("{file} is missing or unreadable")),
            (None, Some(_)) => unrecorded.push(file),
            (Some(want), Some(have)) if want.eq_ignore_ascii_case(&have) => matched += 1,
            (Some(_), Some(_)) => problems.push(format!(
                "{file} differs from the build the installer recorded (manual replacement, a \
                 development build, or a partial update)"
            )),
        }
    }

    if !problems.is_empty() {
        return (
            Verdict::Warn,
            format!(
                "release identity INCONSISTENT with the recorded {} {shown}: {}. {ADVICE}",
                class.label(),
                problems.join("; ")
            ),
        );
    }
    if !unrecorded.is_empty() {
        return (
            Verdict::Info,
            format!(
                "release identity UNKNOWN for {}: the manifest records {} {shown} but no \
                 executable hashes (installed before identity recording), so this host cannot be \
                 told from a hand-modified one. Updating with the supported updater records them.",
                unrecorded.join(", "),
                class.label()
            ),
        );
    }
    (
        Verdict::Ok,
        format!(
            "release identity: recorded {} {shown}; {matched} installed executables and the \
             running vpn-admin match the hashes the installer recorded (not independent proof of \
             published-artifact provenance; unit files and helper scripts are not hashed)",
            class.label()
        ),
    )
}

/// Reads the real manifest, the installed executables and the running
/// executable. Read-only.
pub fn check(running_core: &str) -> (Verdict, String) {
    let manifest = std::fs::read_to_string(INSTALL_STATE_MANIFEST).ok();
    let pending = std::env::var_os(PENDING_ENV).is_some_and(|v| v == "1");
    evaluate(
        manifest.as_deref(),
        running_core,
        |file| sha256_file(&Path::new(BIN_DIR).join(file)),
        &RunningExecutable {
            sha256: std::env::current_exe()
                .ok()
                .and_then(|path| sha256_file(&path)),
        },
        pending,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const H_ADMIN: &str = "aa";
    const H_SUB: &str = "bb";

    fn manifest(tag: &str, admin: Option<&str>, sub: Option<&str>) -> String {
        let mut m = serde_json::json!({"singbox_vpn_version": tag});
        if let Some(a) = admin {
            m["vpn_admin_sha256"] = a.into();
        }
        if let Some(s) = sub {
            m["subscription_sha256"] = s.into();
        }
        m.to_string()
    }

    fn installed(admin: &'static str, sub: &'static str) -> impl Fn(&str) -> Option<String> {
        move |file| match file {
            "vpn-admin" | "vpn" => Some(admin.to_string()),
            "vpn-subscription-svc" => Some(sub.to_string()),
            _ => None,
        }
    }

    fn running(sha: &str) -> RunningExecutable {
        RunningExecutable {
            sha256: Some(sha.to_string()),
        }
    }

    #[test]
    fn classifies_release_tags() {
        assert_eq!(classify("v1.2.0"), ReleaseClass::Stable);
        assert_eq!(classify("v1.2.1-rc.3"), ReleaseClass::ReleaseCandidate);
        assert_eq!(classify("main"), ReleaseClass::Development);
        assert_eq!(classify(""), ReleaseClass::Development);
        assert_eq!(classify("v1.2"), ReleaseClass::Development);
        assert_eq!(release_core("v10.20.30-rc.1"), Some("10.20.30"));
        assert_eq!(release_core("1.2.3"), None);
    }

    #[test]
    fn matching_stable_install_is_ok_without_claiming_provenance() {
        let m = manifest("v1.2.1", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.1",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Ok, "{msg}");
        assert!(msg.contains("recorded stable release v1.2.1"), "{msg}");
        assert!(msg.contains("3 installed executables"), "{msg}");
        assert!(msg.contains("not independent proof"), "{msg}");
        assert!(!msg.contains("published stable"), "{msg}");
    }

    #[test]
    fn release_candidate_is_labelled_as_such() {
        let m = manifest("v1.2.1-rc.2", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.1",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Ok);
        assert!(msg.contains("release candidate v1.2.1-rc.2"), "{msg}");
    }

    #[test]
    fn manual_binary_swap_on_the_same_version_is_detected() {
        // The owner's case: main's vpn-admin copied over v1.2.0 — both report 1.2.0.
        let m = manifest("v1.2.0", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.0",
            installed("cc", H_SUB),
            &running("cc"),
            false,
        );
        assert_eq!(v, Verdict::Warn);
        assert!(msg.contains("INCONSISTENT"), "{msg}");
        assert!(msg.contains("vpn-admin differs"), "{msg}");
        assert!(msg.contains("vpn differs"), "{msg}");
        assert!(!msg.contains("vpn-subscription-svc differs"), "{msg}");
        assert!(msg.contains("never copy executables by hand"), "{msg}");
    }

    #[test]
    fn a_development_binary_run_by_path_is_not_reported_as_the_installed_release() {
        // The installed files are the genuine recorded release; the process doing
        // the check is a development build started from a build directory.
        let m = manifest("v1.2.0", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.0",
            installed(H_ADMIN, H_SUB),
            &running("dev-build"),
            false,
        );
        assert_eq!(v, Verdict::Warn, "{msg}");
        assert!(
            msg.contains("running this check is NOT the recorded build"),
            "{msg}"
        );
    }

    #[test]
    fn an_unreadable_running_executable_does_not_invent_a_mismatch() {
        let m = manifest("v1.2.0", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.0",
            installed(H_ADMIN, H_SUB),
            &RunningExecutable { sha256: None },
            false,
        );
        assert_eq!(v, Verdict::Ok, "{msg}");
    }

    #[test]
    fn repair_advice_never_recommends_repair_toward_an_older_release() {
        let m = manifest("v1.2.0", Some(H_ADMIN), Some(H_SUB));
        let (_, msg) = evaluate(
            Some(&m),
            "1.2.0",
            installed("cc", H_SUB),
            &running("cc"),
            false,
        );
        assert!(msg.contains("update.sh --version <tag>"), "{msg}");
        assert!(msg.contains("Do NOT use `update.sh --repair`"), "{msg}");
        assert!(msg.contains("[egress]"), "{msg}");
    }

    #[test]
    fn version_core_mismatch_is_detected_even_when_hashes_agree() {
        let m = manifest("v1.2.0", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.1",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Warn);
        assert!(msg.contains("this vpn-admin is 1.2.1 but"), "{msg}");
    }

    #[test]
    fn missing_executable_is_a_problem() {
        let m = manifest("v1.2.1", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.1",
            |f| (f != "vpn-subscription-svc").then(|| H_ADMIN.to_string()),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Warn);
        assert!(msg.contains("vpn-subscription-svc is missing"), "{msg}");
    }

    #[test]
    fn unrecorded_hashes_are_unknown_not_a_match() {
        let m = manifest("v1.2.0", None, None);
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.0",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Info);
        assert!(msg.contains("UNKNOWN"), "{msg}");
        assert!(!msg.contains("match the hashes"), "{msg}");
    }

    #[test]
    fn development_channel_checks_hashes_but_not_the_version_core() {
        let m = manifest("main", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "9.9.9",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Ok, "{msg}");
        assert!(msg.contains("development channel"), "{msg}");
        let (v, _) = evaluate(
            Some(&m),
            "9.9.9",
            installed("zz", H_SUB),
            &running("zz"),
            false,
        );
        assert_eq!(v, Verdict::Warn);
    }

    #[test]
    fn missing_or_corrupt_manifest_is_unknown() {
        let (v, msg) = evaluate(
            None,
            "1.2.1",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Warn);
        assert!(msg.contains("UNKNOWN"), "{msg}");
        let (v, msg) = evaluate(
            Some("{not json"),
            "1.2.1",
            installed(H_ADMIN, H_SUB),
            &running(H_ADMIN),
            false,
        );
        assert_eq!(v, Verdict::Warn);
        assert!(msg.contains("not valid JSON"), "{msg}");
    }

    #[test]
    fn update_in_progress_is_informational_only() {
        let m = manifest("v1.2.0", Some(H_ADMIN), Some(H_SUB));
        let (v, msg) = evaluate(
            Some(&m),
            "1.2.1",
            installed("new", "new"),
            &running("new"),
            true,
        );
        assert_eq!(v, Verdict::Info);
        assert!(msg.contains("update in progress"), "{msg}");
    }

    #[test]
    fn hashing_matches_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(sha256_file(&dir.path().join("absent")), None);
    }
}
