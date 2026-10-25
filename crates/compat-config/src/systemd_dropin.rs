//! systemd drop-in that grants `sing-box.service` the extra capabilities the TLS spoof needs.
//!
//! sing-box `tls_spoof` (docs/TLS_SPOOF.md) needs CAP_NET_RAW and CAP_NET_ADMIN. The shipped unit grants
//! only CAP_NET_BIND_SERVICE, and without the extra capabilities sing-box STARTS and the spoof silently does
//! nothing. This module owns the drop-in's content and the decision of what to do with the file; the admin
//! tool performs the I/O. Everything here is pure so the lifecycle can be tested without systemd.
//!
//! Policy (explicit, because it touches service privileges):
//! * Created only while `[google_egress_tls_spoof]` is configured; granted capabilities are exactly the two
//!   the feature needs (plus the shipped CAP_NET_BIND_SERVICE the drop-in must repeat because the lists
//!   replace the unit's). The shipped unit file is never edited and nothing else is weakened.
//! * The file carries a marker line. Only a file carrying the marker is ever rewritten or removed.
//! * A file WITHOUT the marker is operator-made: if its effective content equals the desired content it is
//!   adopted (rewritten once with the marker, same capabilities); otherwise it is left alone and reported.
//! * When the feature is disabled the managed file is removed (capabilities revert to the shipped unit's).

use crate::deployment::DeploymentConfig;

/// File name inside `/etc/systemd/system/sing-box.service.d/`. It matches the name the operator-made drop-in
/// on the reference hosts already uses, so an upgrade adopts that file instead of leaving two.
pub const TLS_SPOOF_DROPIN_FILE: &str = "tls-spoof.conf";
pub const SING_BOX_DROPIN_DIR: &str = "sing-box.service.d";
pub const MANAGED_MARKER: &str =
    "# managed by vpn-admin (singbox-vpn): sing-box TLS spoof capabilities. Do not edit; remove [google_egress_tls_spoof] to remove this file.";

const CAPABILITIES: &str = "CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN";

/// The body systemd reads (without the marker line).
fn body() -> String {
    format!("[Service]\nAmbientCapabilities={CAPABILITIES}\nCapabilityBoundingSet={CAPABILITIES}\n")
}

/// Full desired file content, or `None` when the feature is not configured.
pub fn desired_tls_spoof_dropin(cfg: &DeploymentConfig) -> Option<String> {
    cfg.google_egress_tls_spoof
        .as_ref()
        .map(|_| format!("{MANAGED_MARKER}\n{}", body()))
}

#[derive(Debug, PartialEq, Eq)]
pub enum DropinAction {
    /// Nothing to do.
    None,
    /// Create or rewrite the managed file with `content`.
    Write(String),
    /// Delete the managed file.
    Remove,
    /// A file this tool does not own is in the way; leave it, tell the operator.
    LeaveUnmanaged(&'static str),
}

fn is_managed(existing: &str) -> bool {
    existing.lines().next().map(str::trim_end) == Some(MANAGED_MARKER)
}

/// Normalised effective content: comments and blank lines dropped, whitespace around `=` ignored.
fn effective(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut parts = l.splitn(2, '=');
            match (parts.next(), parts.next()) {
                (Some(k), Some(v)) => format!(
                    "{}={}",
                    k.trim(),
                    v.split_whitespace().collect::<Vec<_>>().join(" ")
                ),
                _ => l.to_string(),
            }
        })
        .collect()
}

/// Decide what to do given the file currently on disk (`existing`) and what the configuration wants.
pub fn plan_tls_spoof_dropin(existing: Option<&str>, desired: Option<&str>) -> DropinAction {
    match (existing, desired) {
        (None, None) => DropinAction::None,
        (None, Some(want)) => DropinAction::Write(want.to_string()),
        (Some(have), Some(want)) => {
            if have == want {
                DropinAction::None
            } else if is_managed(have) || effective(have) == effective(want) {
                // Managed file out of date, or an operator-made file with the same effect: (re)write it
                // with the marker so it is owned from now on.
                DropinAction::Write(want.to_string())
            } else {
                DropinAction::LeaveUnmanaged(
                    "an operator-made sing-box drop-in with different capabilities exists at this path; it was not changed",
                )
            }
        }
        (Some(have), None) => {
            if is_managed(have) {
                DropinAction::Remove
            } else {
                DropinAction::LeaveUnmanaged(
                    "an operator-made capability drop-in exists but the TLS spoof is not configured; it was not removed",
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desired() -> String {
        format!("{MANAGED_MARKER}\n{}", body())
    }

    #[test]
    fn content_grants_exactly_the_spoof_capabilities_and_nothing_else() {
        let text = desired();
        assert!(text.starts_with(MANAGED_MARKER));
        assert!(
            text.contains("AmbientCapabilities=CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN\n")
        );
        assert!(
            text.contains("CapabilityBoundingSet=CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN\n")
        );
        for forbidden in [
            "CAP_SYS_ADMIN",
            "CAP_DAC",
            "CAP_SETUID",
            "NoNewPrivileges",
            "ProtectSystem",
            "User=",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} must not appear");
        }
        assert_eq!(text.matches("[Service]").count(), 1);
    }

    #[test]
    fn lifecycle_decisions() {
        let want = desired();
        // fresh install with the feature on
        assert_eq!(
            plan_tls_spoof_dropin(None, Some(&want)),
            DropinAction::Write(want.clone())
        );
        // idempotent
        assert_eq!(
            plan_tls_spoof_dropin(Some(&want), Some(&want)),
            DropinAction::None
        );
        // feature never enabled
        assert_eq!(plan_tls_spoof_dropin(None, None), DropinAction::None);
        // managed file out of date is rewritten
        let stale = format!("{MANAGED_MARKER}\n[Service]\nAmbientCapabilities=CAP_NET_RAW\n");
        assert_eq!(
            plan_tls_spoof_dropin(Some(&stale), Some(&want)),
            DropinAction::Write(want.clone())
        );
        // feature disabled: managed file removed
        assert_eq!(
            plan_tls_spoof_dropin(Some(&want), None),
            DropinAction::Remove
        );
    }

    #[test]
    fn an_operator_made_file_is_adopted_only_when_it_has_the_same_effect() {
        let want = desired();
        // the reference hosts' hand-made drop-in (no marker, same capabilities)
        let made = "[Service]\nAmbientCapabilities=CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN\n";
        assert_eq!(
            plan_tls_spoof_dropin(Some(made), Some(&want)),
            DropinAction::Write(want.clone())
        );
        // different capabilities: never overwritten
        let other = "[Service]\nAmbientCapabilities=CAP_NET_BIND_SERVICE CAP_SYS_PTRACE\n";
        assert!(matches!(
            plan_tls_spoof_dropin(Some(other), Some(&want)),
            DropinAction::LeaveUnmanaged(_)
        ));
        // feature off: an operator-made file is never deleted
        assert!(matches!(
            plan_tls_spoof_dropin(Some(made), None),
            DropinAction::LeaveUnmanaged(_)
        ));
    }

    #[test]
    fn uninstall_script_recognises_exactly_this_marker() {
        let script = include_str!("../../../deploy/almalinux/uninstall.sh");
        assert!(
            script.contains(MANAGED_MARKER),
            "deploy/almalinux/uninstall.sh must remove the managed drop-in by the same marker line"
        );
    }
    #[test]
    fn only_a_file_whose_first_line_is_the_marker_counts_as_managed() {
        let sneaky = format!("[Service]\n{MANAGED_MARKER}\nAmbientCapabilities=CAP_SYS_ADMIN\n");
        assert!(matches!(
            plan_tls_spoof_dropin(Some(&sneaky), None),
            DropinAction::LeaveUnmanaged(_)
        ));
    }
}
