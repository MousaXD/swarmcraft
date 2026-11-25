//! Storage-only capacity observations. Not gameplay or network throughput.
#[path = "support/research_v1_fixture.rs"]
mod fixture;

use fixture::{capture, copy, random_bytes, tree, LabResult, WORLD};
use std::{fs, time::Instant};
use swarm_storage::Storage;
use walkdir::WalkDir;

fn disk_bytes(root: &std::path::Path) -> u64 {
    WalkDir::new(root)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.metadata().unwrap().len())
        .sum()
}

#[test]
#[ignore = "manual capacity measurement; local disk and bounded synthetic worlds"]
fn separate_world_size_and_verified_replica_count() -> LabResult<()> {
    for mib in [1usize, 8, 32] {
        let temp = tempfile::tempdir()?;
        let world_root = temp.path().join("world");
        fs::create_dir(&world_root)?;
        for index in 0..mib {
            fs::write(world_root.join(format!("region-{index}.mca")), random_bytes(index as u64 * 2 + 1, 1024 * 1024))?;
        }
        let source = Storage::open(temp.path().join("source"))?;
        let start = Instant::now();
        let manifest = capture(&source, &world_root, None)?;
        let capture_ms = start.elapsed().as_secs_f64() * 1000.0;
        let expected = tree(&world_root)?;
        let mut total_bytes = 0;
        let mut total_disk = 0;
        let mut cumulative_verified_ms = 0.0;
        for replicas in 1..=3 {
            let start = Instant::now();
            let target_root = temp.path().join(format!("replica-{replicas}"));
            let target = Storage::open(&target_root)?;
            total_bytes += copy(&source, &target, &manifest)?;
            drop(target);
            let target = Storage::open(&target_root)?;
            target.verify_snapshot(&manifest)?;
            assert!(target.missing_blobs(&manifest).is_empty());
            assert_eq!(target.latest_snapshot(WORLD)?.as_ref(), Some(&manifest));
            total_disk += disk_bytes(&target_root);
            cumulative_verified_ms += start.elapsed().as_secs_f64() * 1000.0;
            let restored = temp.path().join("restored");
            let restore_start = Instant::now();
            target.restore_snapshot(&manifest, &restored)?;
            assert_eq!(tree(&restored)?, expected);
            let restore_ms = restore_start.elapsed().as_secs_f64() * 1000.0;
            fs::remove_dir_all(restored)?;
            eprintln!("capacity world_mib={mib} replicas={replicas} capture_ms={capture_ms:.3} cumulative_verified_ms={cumulative_verified_ms:.3} cumulative_payload_bytes={total_bytes} cumulative_disk_bytes={total_disk} last_restore_ms={restore_ms:.3}");
        }
    }
    Ok(())
}
