use std::{
    fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};
use swarm_consensus::AuthorityGeneration;
use swarm_core::DataPaths;
use swarm_protocol::WorldId;

pub const PERMIT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
pub const PERMIT_TIMEOUT: Duration = Duration::from_secs(6);
pub const PERMIT_START_TIMEOUT: Duration = Duration::from_secs(15);

pub fn permit_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex()).join("authority.permit")
}

pub fn refresh_permit(
    paths: &DataPaths,
    world: WorldId,
    generation: AuthorityGeneration,
    heartbeat: u64,
) -> io::Result<()> {
    let path = permit_path(paths, world);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{} {} {}\n", generation.epoch, generation.fencing_token, heartbeat))
}

pub fn clear_permit(paths: &DataPaths, world: WorldId) -> io::Result<()> {
    let path = permit_path(paths, world);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
pub struct PermitWatch {
    expected: AuthorityGeneration,
    last_heartbeat: Option<u64>,
    last_refresh: Option<Instant>,
}

impl PermitWatch {
    pub fn new(expected: AuthorityGeneration) -> Self {
        Self { expected, last_heartbeat: None, last_refresh: None }
    }

    pub fn observe(&mut self, paths: &DataPaths, world: WorldId, now: Instant) -> io::Result<bool> {
        let value = fs::read_to_string(permit_path(paths, world))?;
        let Some((generation, heartbeat)) = parse_permit(&value) else {
            return Ok(false);
        };
        if generation != self.expected {
            return Ok(false);
        }
        match self.last_heartbeat {
            None => self.last_heartbeat = Some(heartbeat),
            Some(previous) if previous != heartbeat => {
                self.last_heartbeat = Some(heartbeat);
                self.last_refresh = Some(now);
            }
            Some(_) => {}
        }
        Ok(self.is_fresh(now))
    }

    pub fn is_fresh(&self, now: Instant) -> bool {
        self.last_refresh.is_some_and(|refresh| now.duration_since(refresh) < PERMIT_TIMEOUT)
    }
}

fn parse_permit(value: &str) -> Option<(AuthorityGeneration, u64)> {
    let mut fields = value.split_whitespace();
    let epoch = fields.next()?.parse().ok()?;
    let fencing_token = fields.next()?.parse().ok()?;
    let heartbeat = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some((AuthorityGeneration { epoch, fencing_token }, heartbeat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_permit_never_becomes_live() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([7; 32]);
        let generation = AuthorityGeneration { epoch: 4, fencing_token: 9 };
        refresh_permit(&paths, world, generation, 1).unwrap();
        let start = Instant::now();
        let mut watch = PermitWatch::new(generation);
        assert!(!watch.observe(&paths, world, start).unwrap());
        assert!(!watch.observe(&paths, world, start + Duration::from_secs(1)).unwrap());
    }

    #[test]
    fn changing_exact_generation_becomes_live_then_expires() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([8; 32]);
        let generation = AuthorityGeneration { epoch: 11, fencing_token: 22 };
        let start = Instant::now();
        let mut watch = PermitWatch::new(generation);
        refresh_permit(&paths, world, generation, 1).unwrap();
        assert!(!watch.observe(&paths, world, start).unwrap());
        refresh_permit(&paths, world, generation, 2).unwrap();
        assert!(watch.observe(&paths, world, start + Duration::from_secs(1)).unwrap());
        assert!(!watch.is_fresh(start + Duration::from_secs(8)));
    }

    #[test]
    fn different_generation_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([9; 32]);
        let expected = AuthorityGeneration { epoch: 2, fencing_token: 2 };
        refresh_permit(&paths, world, AuthorityGeneration { epoch: 3, fencing_token: 3 }, 1).unwrap();
        let mut watch = PermitWatch::new(expected);
        assert!(!watch.observe(&paths, world, Instant::now()).unwrap());
    }
}
