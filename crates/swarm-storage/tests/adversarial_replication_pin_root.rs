//! Agent 10 independent red-phase reproduction (PR #124), retained verbatim
//! in behavior against Agent 02's candidate fix for issue #102.
//! The expected invariant is fail-closed garbage collection when a live
//! replication lease's root has been replaced with a dangling symlink.

#[cfg(unix)]
mod unix {
    use std::{fs, os::unix::fs::symlink};
    use swarm_protocol::{BlobEncoding, WorldId};
    use swarm_storage::Storage;

    #[test]
    fn dangling_replication_pin_root_must_block_destructive_gc() {
        let temp = tempfile::tempdir().unwrap();
        let world = WorldId([0xa7; 32]);
        let storage = Storage::open(temp.path().join("store")).unwrap();
        let blob = storage.put_blob(world, b"in-flight-authority-recovery-state").unwrap();
        let suffix = match blob.encoding {
            BlobEncoding::Raw => "raw",
            BlobEncoding::Zstd => "zst",
        };
        let blob_path = storage.world_dir(world).join("blobs").join(format!("{}.{}", blob.hash.to_hex(), suffix));
        assert!(blob_path.is_file(), "the complete blob must exist before corruption");

        // A live replication lease promises that complete uncommitted data is
        // retained until its publication/lease lifecycle completes.
        let lease = storage.pin_replication_hashes(world, &[blob.hash]).unwrap();
        assert_eq!(lease.pinned_blobs(), 1);
        let pins = storage.world_dir(world).join(".replication-pins");
        let saved_pins = storage.world_dir(world).join(".replication-pins-injected-quarantine");
        fs::rename(&pins, &saved_pins).unwrap();
        symlink(temp.path().join("nonexistent-pin-directory"), &pins).unwrap();
        assert!(fs::symlink_metadata(&pins).unwrap().file_type().is_symlink());

        // A damaged pin root must not be indistinguishable from a genuinely
        // missing root. Never sweep a complete blob while its liveness is unknown.
        let result = storage.garbage_collect_blobs(world);
        assert!(result.is_err(), "GC accepted a dangling replication-pin directory: {result:?}");
        assert!(blob_path.is_file(), "GC deleted a potentially live in-flight blob");
        drop(lease);
    }
}
