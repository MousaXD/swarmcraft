use crate::StorageError;
use std::collections::BTreeSet;
use swarm_protocol::SnapshotEntry;

/// Returns the project-level cross-platform collision identity for a snapshot path.
///
/// SwarmCraft's canonical snapshot namespace is intentionally conservative: path
/// components must be ASCII. That makes Unicode normalization behavior explicit
/// and fail-closed instead of depending on NTFS/APFS/HFS+/ext filesystem-specific
/// normalization. Components are then ASCII-case-folded for collision detection.
/// Windows-reserved device names and trailing dot/space aliases are rejected even
/// when the snapshot is created on a case-sensitive Unix filesystem.
pub(crate) fn portable_path_collision_key(path: &str) -> Result<String, StorageError> {
    validate_portable_path(path)?;
    let mut folded = Vec::new();
    for component in path.split('/') {
        folded.push(portable_component_key(component, path)?);
    }
    Ok(folded.join("/"))
}

pub(crate) fn validate_manifest_paths(entries: &[SnapshotEntry]) -> Result<(), StorageError> {
    let mut exact = BTreeSet::new();
    let mut portable = BTreeSet::new();
    for entry in entries {
        validate_portable_path(&entry.path)?;
        if !exact.insert(entry.path.as_str()) {
            return Err(StorageError::UnsafeRelativePath(entry.path.clone()));
        }
        let key = portable_path_collision_key(&entry.path)?;
        if !portable.insert(key) {
            return Err(StorageError::PortablePathCollision(entry.path.clone()));
        }
    }
    Ok(())
}

pub(crate) fn validate_portable_path(path: &str) -> Result<(), StorageError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || (path.len() >= 2 && path.as_bytes()[1] == b':')
    {
        return Err(StorageError::UnsafeRelativePath(path.to_owned()));
    }

    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(StorageError::UnsafeRelativePath(path.to_owned()));
        }
        let _ = portable_component_key(component, path)?;
    }
    Ok(())
}

fn portable_component_key(component: &str, full_path: &str) -> Result<String, StorageError> {
    if !component.is_ascii() {
        return Err(StorageError::UnsafeRelativePath(full_path.to_owned()));
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(StorageError::UnsafeRelativePath(full_path.to_owned()));
    }
    if component
        .bytes()
        .any(|byte| byte < 0x20 || byte == 0x7f || matches!(byte, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*'))
    {
        return Err(StorageError::UnsafeRelativePath(full_path.to_owned()));
    }

    let folded = component.to_ascii_lowercase();
    let device_stem = folded.split('.').next().unwrap_or(&folded);
    let reserved = matches!(device_stem, "con" | "prn" | "aux" | "nul")
        || matches!(device_stem.strip_prefix("com"), Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        || matches!(device_stem.strip_prefix("lpt"), Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    if reserved {
        return Err(StorageError::UnsafeRelativePath(full_path.to_owned()));
    }
    Ok(folded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_protocol::{BlobDescriptor, BlobEncoding, Hash32};

    fn entry(path: &str) -> SnapshotEntry {
        SnapshotEntry {
            path: path.into(),
            blob: BlobDescriptor {
                hash: Hash32([1; 32]),
                uncompressed_size: 1,
                encoded_size: 1,
                encoding: BlobEncoding::Raw,
            },
        }
    }

    #[test]
    fn rejects_cross_platform_aliases() {
        assert!(validate_manifest_paths(&[entry("region/Foo.dat"), entry("region/foo.dat")]).is_err());
        assert!(validate_manifest_paths(&[entry("region/same.dat"), entry("region/same.dat")]).is_err());
        for path in [
            "CON",
            "aux.txt",
            "COM1.log",
            "LPT9",
            "folder/name. ",
            "folder/name.",
            "folder/name<.dat",
            "folder/name>.dat",
            "folder/name:.dat",
            "folder/name\".dat",
            "folder/name|.dat",
            "folder/name?.dat",
            "folder/name*.dat",
            "../escape.dat",
            "folder/../escape.dat",
            "/rooted.dat",
            "\\rooted.dat",
            "C:/rooted.dat",
            "folder\\name.dat",
            ".",
            "folder/.",
            "folder/é.dat",
        ] {
            assert!(validate_portable_path(path).is_err(), "portable alias unexpectedly accepted: {path}");
        }
        assert!(validate_portable_path("region/r.0.0.mca").is_ok());
    }
}
