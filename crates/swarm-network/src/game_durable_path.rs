//! Validate the ancestry of locally persisted game-route security state.
//!
//! A private leaf directory is insufficient if another user can rename its
//! parent and substitute an older generation floor or a different world-port
//! mapping. Reject ancestor symlinks and group/world-writable ancestors.
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let mode = metadata.permissions().mode();
            let writable_by_others = mode & 0o022 != 0;
            // Root-owned sticky temporary directories protect user-owned
            // children against deletion/replacement by other local users.
            let trusted_sticky = mode & 0o1000 != 0 && metadata.uid() == 0;
            if writable_by_others && !trusted_sticky {
                return Err(untrusted_ancestor());
            }
        }
    }
    Ok(())
}

fn untrusted_ancestor() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe game-route durable storage parent")
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
}
