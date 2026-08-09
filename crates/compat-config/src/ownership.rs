//! Ownership preservation across atomic tmp-file+rename replacement.
//!
//! `std::fs::rename` never changes ownership — the destination inode
//! keeps whatever owner/group the *source* (temp) file already had. Since
//! every temp file in this crate is created fresh by whichever process is
//! writing (normally `vpn-admin` running as root, whose default group is
//! `root`), a naive write-tmp-then-rename silently turns a carefully
//! `root:vpn-subscription`/`root:sing-box`-owned file back into
//! `root:root` on every single mutation after install — see
//! docs/FINAL_PRODUCTION_AUDIT.md P0-2. This module is the fix: call
//! [`preserve_ownership_before_rename`] on the temp file, after it is
//! fully written and before it is renamed over the live target.
//!
//! Two cases:
//!  - the target already exists: re-apply *its current* owner/group to
//!    the temp file (self-healing — correct even if this is literally the
//!    first time this code has ever run against an older, already-broken
//!    file, as long as something has since repaired it once).
//!  - the target does not exist yet (first-ever write): fall back to the
//!    parent directory's group, which `deploy/almalinux/install.sh` now
//!    sets up front (setgid directories, docs/FINAL_PRODUCTION_AUDIT.md
//!    P0-1) — a newly created directory entry normally inherits this via
//!    setgid alone, but we set it explicitly too so behavior does not
//!    depend on the setgid bit never being stripped.
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::CompatError;

#[cfg(unix)]
pub fn preserve_ownership_before_rename(
    tmp_path: &Path,
    target_path: &Path,
) -> Result<(), CompatError> {
    let (uid, gid) = if let Ok(meta) = std::fs::metadata(target_path) {
        (meta.uid(), meta.gid())
    } else if let Some(parent) = target_path.parent() {
        // First-ever write: the tmp file's own uid is already correct
        // (it's whatever process created it, i.e. the correct owner —
        // usually root); only the group needs to come from the parent
        // directory's setgid-established group.
        let current_uid = std::fs::metadata(tmp_path)
            .map_err(|e| CompatError::Io(e.to_string()))?
            .uid();
        let parent_gid = std::fs::metadata(parent)
            .map_err(|e| CompatError::Io(e.to_string()))?
            .gid();
        (current_uid, parent_gid)
    } else {
        return Ok(());
    };
    std::os::unix::fs::chown(tmp_path, Some(uid), Some(gid))
        .map_err(|e| CompatError::Io(format!("failed to preserve ownership on {tmp_path:?}: {e}")))
}

#[cfg(not(unix))]
pub fn preserve_ownership_before_rename(
    _tmp_path: &Path,
    _target_path: &Path,
) -> Result<(), CompatError> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn no_prior_target_falls_back_to_parent_group_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        let tmp = dir.path().join("config.json.tmp.1");
        std::fs::write(&tmp, b"{}").unwrap();
        // Not running as root in the test sandbox, so an actual gid change
        // to a different group would fail — but chown to the SAME
        // uid/gid the process already has must always succeed, and that's
        // exactly the case exercised here (tempdir is owned by us).
        preserve_ownership_before_rename(&tmp, &target).unwrap();
        let mode = std::fs::metadata(&tmp).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o170000,
            0o100000,
            "tmp file must still be a regular file"
        );
    }

    #[test]
    fn existing_target_ownership_is_reapplied_to_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        std::fs::write(&target, b"old").unwrap();
        let target_gid = std::fs::metadata(&target).unwrap().gid();

        let tmp = dir.path().join("config.json.tmp.1");
        std::fs::write(&tmp, b"new").unwrap();
        preserve_ownership_before_rename(&tmp, &target).unwrap();
        let tmp_gid = std::fs::metadata(&tmp).unwrap().gid();
        assert_eq!(tmp_gid, target_gid);
    }
}
