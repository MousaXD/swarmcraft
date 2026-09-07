from pathlib import Path

path = Path("apps/desktop/src-tauri/src/launcher_commands.rs")
text = path.read_text()
if "symlinked_staging_session_is_rejected" not in text:
    text += r'''

#[cfg(all(test, unix))]
mod agent5_staging_symlink_tests {
    use super::*;
    use std::{os::unix::fs::symlink, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn symlinked_staging_session_is_rejected() {
        let session = provider_staging_dir().expect("create opaque provider staging session");
        let paths = DataPaths::discover().expect("discover SwarmCraft data directory");
        let staging_root = paths.root.join("provider-staging");
        let session_path = staging_root.join(&session);
        std::fs::remove_dir(&session_path).expect("replace newly-created session directory");

        let target = std::env::temp_dir().join(format!(
            "swarmcraft-agent5-symlink-target-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&target).expect("create symlink target");
        symlink(&target, &session_path).expect("create staging-session symlink");

        let error = resolve_provider_staging_session(&session)
            .expect_err("symlinked provider staging session must fail closed");
        assert_eq!(error, "invalid provider staging session");

        std::fs::remove_file(&session_path).expect("remove staging-session symlink");
        std::fs::remove_dir_all(&target).expect("remove symlink target");
    }
}
'''
    path.write_text(text)
