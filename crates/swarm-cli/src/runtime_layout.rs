use std::path::PathBuf;
use swarm_core::DataPaths;
use swarm_protocol::WorldId;

pub const RUNTIME_LOCK_SCHEMA_VERSION: u16 = 1;

pub fn managed_runtimes_root(paths: &DataPaths) -> PathBuf {
    paths.root.join("runtimes")
}

pub fn managed_java_root(paths: &DataPaths, major: u32) -> PathBuf {
    managed_runtimes_root(paths).join("java").join(major.to_string())
}

pub fn managed_minecraft_server(paths: &DataPaths, minecraft: &str) -> PathBuf {
    managed_runtimes_root(paths).join("minecraft").join(minecraft).join("server.jar")
}

pub fn managed_fabric_server(paths: &DataPaths, minecraft: &str, loader: &str) -> PathBuf {
    managed_runtimes_root(paths)
        .join("fabric")
        .join(minecraft)
        .join(loader)
        .join("fabric-server-launch.jar")
}

pub fn managed_fabric_api(paths: &DataPaths, version: &str) -> PathBuf {
    managed_runtimes_root(paths)
        .join("fabric-api")
        .join(version)
        .join(format!("fabric-api-{version}.jar"))
}

pub fn managed_swarmcraft_fabric(paths: &DataPaths, version: &str) -> PathBuf {
    managed_runtimes_root(paths)
        .join("swarmcraft-fabric")
        .join(version)
        .join(format!("swarmcraft-fabric-{version}.jar"))
}

pub fn managed_world_root(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("runtime-components").join(world.to_hex())
}

pub fn managed_world_server_dir(paths: &DataPaths, world: WorldId) -> PathBuf {
    managed_world_root(paths, world).join("server")
}

pub fn managed_world_mods_dir(paths: &DataPaths, world: WorldId) -> PathBuf {
    managed_world_root(paths, world).join("mods")
}

pub fn managed_world_config_dir(paths: &DataPaths, world: WorldId) -> PathBuf {
    managed_world_root(paths, world).join("config")
}

pub fn runtime_lock_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    managed_world_root(paths, world).join("runtime-lock.json")
}

pub fn runtime_install_lock_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    managed_world_root(paths, world).join("install.lock")
}
