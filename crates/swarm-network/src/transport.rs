use anyhow::{Context, Result};
use libp2p::identity::Keypair;
use std::{fs, io::Write, path::Path};

pub fn generate_transport_key() -> Keypair {
    Keypair::generate_ed25519()
}

pub fn load_or_create_transport_key(path: &Path) -> Result<Keypair> {
    if path.exists() {
        let bytes = fs::read(path).with_context(|| format!("cannot read transport key {}", path.display()))?;
        return Keypair::from_protobuf_encoding(&bytes).context("invalid persisted libp2p transport key");
    }

    let parent = path.parent().context("transport key path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    let key = generate_transport_key();
    let encoded = key.to_protobuf_encoding().context("cannot encode libp2p transport key")?;
    let tmp = path.with_extension("key.tmp");

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?
    };

    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .with_context(|| format!("cannot create {}", tmp.display()))?;

    file.write_all(&encoded)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path).with_context(|| format!("cannot persist transport key {}", path.display()))?;
    sync_parent(parent)?;
    Ok(key)
}

fn sync_parent(parent: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn persisted_transport_identity_is_stable() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("identity/transport.key");
        let first = load_or_create_transport_key(&path).unwrap();
        let first_peer = first.public().to_peer_id();
        drop(first);
        let second = load_or_create_transport_key(&path).unwrap();
        assert_eq!(first_peer, second.public().to_peer_id());
    }
}
