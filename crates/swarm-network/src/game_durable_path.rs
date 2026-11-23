//! Validate the ancestry of locally persisted game-route security state.
//!
//! A private leaf directory is insufficient if another user can rename its
//! parent and substitute an older generation floor or a different world-port
//! mapping. Reject ancestor symlinks, group/world-writable ancestors, and
//! ancestors owned by a different unprivileged user. Mode 0755 is NOT
//! sufficient if an unrelated account owns the directory.
//! Allow only root-owned sticky temporary roots such as /tmp: their sticky
//! bit prevents another unprivileged user from removing our owned child.
//!
//! This is a defense against local unprivileged users, not an assurance
//! against a malicious process running with the SAME user credentials.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// Existing components are checked without following symlinks, both before
/// and after durable directory creation. Missing suffix components are fine
/// before creation; their creator must set private permissions.
pub(crate) fn validate_durable_ancestors(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return validate_durable_ancestors_for_uid(path, effective_uid());
    #[cfg(not(unix))]
    return validate_ancestor_components(path, |_| Ok(()));
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: The platform's POSIX geteuid() takes no arguments and has no
    // memory-safety preconditions. uid_t is u32 on supported Unix targets.
    unsafe { geteuid() }
}

#[cfg(unix)]
fn validate_durable_ancestors_for_uid(path: &Path, effective_uid: u32) -> io::Result<()> {
    validate_ancestor_components(path, |metadata| {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mode = metadata.permissions().mode();
        let owner = metadata.uid();
        // Any foreign unprivileged owner can rename its own 0755 directory
        // and replace the private child (including in a root-sticky /tmp).
        if owner != 0 && owner != effective_uid {
            return Err(untrusted_ancestor());
        }
        // Root-owned sticky temporary directories protect the immediate
        // owned entry against deletion by unrelated local users.
        let root_sticky = owner == 0 && mode & 0o1000 != 0;
        if mode & 0o022 != 0 && !root_sticky {
            return Err(untrusted_ancestor());
        }
        Ok(())
    })
}

fn validate_ancestor_components(
    path: &Path,
    mut validate_metadata: impl FnMut(&fs::Metadata) -> io::Result<()>,
) -> io::Result<()> {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => current.push(component),
            Component::CurDir => continue,
            // Do not normalize '..' through a symlink or writable parent.
            Component::ParentDir => return Err(untrusted_ancestor()),
        }
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(untrusted_ancestor());
        }
        validate_metadata(&metadata)?;
    }
    Ok(())
}

fn untrusted_ancestor() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe game-route durable storage parent")
}

/// A trusted directory does not make an existing 0666 record safe. Refuse
/// hard links, symlinks, non-files, foreign-owned or writable-by-others files
/// before consulting durable game authorization state. Only the actual owner
/// can create/replace entries in a private parent directory; same-UID
/// malicious processes remain outside this boundary.
pub(crate) fn validate_private_state_file(path: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(untrusted_ancestor());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != effective_uid() || metadata.permissions().mode() & 0o022 != 0 || metadata.nlink() != 1 {
            return Err(untrusted_ancestor());
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal() {
        assert_eq!(
            validate_durable_ancestors(Path::new("../world-ports")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn existing_secure_path_and_missing_private_suffix_are_allowed() {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(validate_durable_ancestors(&temp.path().join("not-created-yet").join("ports")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_writable_ancestor_and_symlinked_parent() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let writable = temp.path().join("writable-parent");
        fs::create_dir(&writable).unwrap();
        fs::set_permissions(&writable, fs::Permissions::from_mode(0o770)).unwrap();
        assert_eq!(
            validate_durable_ancestors(&writable.join("private-child")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        let destination = temp.path().join("actual");
        fs::create_dir(&destination).unwrap();
        let link = temp.path().join("linked");
        symlink(&destination, &link).unwrap();
        assert_eq!(
            validate_durable_ancestors(&link.join("private-child")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[test]
    fn foreign_owner_cannot_replace_a_private_child_even_with_mode_0755() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let user_id = fs::symlink_metadata(temp.path()).unwrap().uid();
        if user_id == 0 {
            // Under a root test runner the temporary directory itself is
            // trusted root-owned; ownership of an untrusted parent cannot be
            // exercised without chown/root privileges. Skip just this case.
            return;
        }
        let foreign_id = user_id.checked_add(1).unwrap();
        assert_eq!(
            validate_durable_ancestors_for_uid(&temp.path().join("private-generation-floor"), foreign_id,)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(validate_durable_ancestors_for_uid(&temp.path().join("private-generation-floor"), user_id).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn existing_world_writable_state_file_is_never_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let record = dir.path().join("game.floor");
        fs::write(&record, b"private-authority-record").unwrap();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(validate_private_state_file(&record).unwrap());
        fs::set_permissions(&record, fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(validate_private_state_file(&record).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        let second = dir.path().join("hard-linked-record");
        fs::hard_link(&record, &second).unwrap();
        assert_eq!(validate_private_state_file(&record).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(!validate_private_state_file(&dir.path().join("not-created")).unwrap());
    }
}
