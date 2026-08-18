from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def read(path: str) -> str:
    return (ROOT / path).read_text()


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content)


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise RuntimeError(f"missing manual-runtime anchor: {label}")
    return text.replace(old, new, 1)


def installer_manual_status() -> None:
    path = "crates/swarm-cli/src/runtime_installer.rs"
    text = read(path)

    old = """        let mut components = vec![
            self.java_status(required_java_major, lock.as_ref(), manual.as_ref(), lock_compatible),
            self.artifact_status(
                RuntimeComponentKind::MinecraftServer,
                "minecraft_server",
                lock.as_ref(),
                lock_compatible,
                Some(managed_world_server_dir(self.paths, world).join("server.jar")),
            ),
            self.artifact_status(
                RuntimeComponentKind::FabricLoader,
                "fabric_loader",
                lock.as_ref(),
                lock_compatible,
                None,
            ),
            self.artifact_status(
                RuntimeComponentKind::FabricApi,
                "fabric_api",
                lock.as_ref(),
                lock_compatible,
                Some(managed_world_mods_dir(self.paths, world).join("fabric-api.jar")),
            ),
            self.swarmcraft_status(world, lock.as_ref(), lock_compatible),
        ];

        let directories_ready = [
            managed_world_root(self.paths, world),
            managed_world_server_dir(self.paths, world),
            managed_world_mods_dir(self.paths, world),
            managed_world_config_dir(self.paths, world),
        ]
        .iter()
        .all(|path| path.is_dir());
"""
    if old in text:
        new = """        let manual_only = manual.as_ref().filter(|_| lock.is_none());
        let mut components = vec![
            self.java_status(required_java_major, lock.as_ref(), manual.as_ref(), lock_compatible),
            manual_only.map_or_else(
                || self.artifact_status(
                    RuntimeComponentKind::MinecraftServer,
                    "minecraft_server",
                    lock.as_ref(),
                    lock_compatible,
                    Some(managed_world_server_dir(self.paths, world).join("server.jar")),
                ),
                |config| manual_file_status(
                    RuntimeComponentKind::MinecraftServer,
                    &config.server_jar,
                    "manual Fabric server launcher is configured; live Fabric verification proves the exact Minecraft runtime before host readiness",
                ),
            ),
            manual_only.map_or_else(
                || self.artifact_status(
                    RuntimeComponentKind::FabricLoader,
                    "fabric_loader",
                    lock.as_ref(),
                    lock_compatible,
                    None,
                ),
                |config| manual_file_status(
                    RuntimeComponentKind::FabricLoader,
                    &config.server_jar,
                    "manual Fabric launcher is configured; loader compatibility is proven by the live Fabric handshake",
                ),
            ),
            manual_only.map_or_else(
                || self.artifact_status(
                    RuntimeComponentKind::FabricApi,
                    "fabric_api",
                    lock.as_ref(),
                    lock_compatible,
                    Some(managed_world_mods_dir(self.paths, world).join("fabric-api.jar")),
                ),
                |config| manual_file_status(
                    RuntimeComponentKind::FabricApi,
                    &config.mod_jar,
                    "manual SwarmCraft Fabric integration is configured; release packaging embeds Fabric API and live verification remains required",
                ),
            ),
            manual_only.map_or_else(
                || self.swarmcraft_status(world, lock.as_ref(), lock_compatible),
                |config| self.manual_swarmcraft_status(world, config),
            ),
        ];

        let directories_ready = manual_only.is_some()
            || [
                managed_world_root(self.paths, world),
                managed_world_server_dir(self.paths, world),
                managed_world_mods_dir(self.paths, world),
                managed_world_config_dir(self.paths, world),
            ]
            .iter()
            .all(|path| path.is_dir());
"""
        text = text.replace(old, new, 1)

    if "fn manual_swarmcraft_status(" not in text:
        anchor = "    fn server_mods_status(&self, world: WorldId) -> RuntimeComponentStatus {"
        block = """    fn manual_swarmcraft_status(
        &self,
        world: WorldId,
        config: &RuntimeLaunchConfig,
    ) -> RuntimeComponentStatus {
        let expected = self
            .storage
            .load_world_config(world)
            .ok()
            .map(|world_config| world_config.compatibility.fabric_adapter_version)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        if expected != env!("CARGO_PKG_VERSION") {
            return RuntimeComponentStatus {
                kind: RuntimeComponentKind::SwarmcraftFabric,
                state: RuntimeComponentState::Incompatible,
                version: Some(expected.clone()),
                path: Some(config.mod_jar.clone()),
                managed: false,
                detail: Some(format!(
                    "world requires SwarmCraft Fabric adapter {expected}, but this application build provides {}",
                    env!("CARGO_PKG_VERSION")
                )),
            };
        }
        manual_file_status(
            RuntimeComponentKind::SwarmcraftFabric,
            &config.mod_jar,
            "manual SwarmCraft Fabric integration is configured; host readiness still requires the live authenticated Fabric handshake",
        )
    }

"""
        text = replace_once(text, anchor, block + anchor, "manual SwarmCraft status")

    if "fn manual_file_status(" not in text:
        anchor = "fn platform_components_ready(status: &RuntimeStatus) -> bool {"
        block = """fn manual_file_status(
    kind: RuntimeComponentKind,
    path: &Path,
    ready_detail: &str,
) -> RuntimeComponentStatus {
    let ready = path.is_file();
    RuntimeComponentStatus {
        kind,
        state: if ready { RuntimeComponentState::Ready } else { RuntimeComponentState::Missing },
        version: None,
        path: Some(path.to_path_buf()),
        managed: false,
        detail: Some(if ready {
            ready_detail.to_owned()
        } else {
            format!("manual runtime file is missing: {}", path.display())
        }),
    }
}

"""
        text = replace_once(text, anchor, block + anchor, "manual file status helper")

    write(path, text)


def runtime_verify_boundary() -> None:
    path = "crates/swarm-cli/src/runtime_main.rs"
    text = read(path)
    text = text.replace(
        "use swarm_network::ServerModsReadinessV1;",
        "use swarm_network::{HostRuntimeReadinessV1, ServerModsReadinessV1};",
        1,
    )
    old = """            if status.ready {
                let config = migration::load_runtime_config(&paths, world)?;
                let descriptor = storage.load_world_descriptor(world)?;
                host_readiness::record_runtime_verified(&paths, world, &config, descriptor.compatibility_fingerprint)?;
                let world_config = storage.load_world_config(world)?;
"""
    if old in text:
        new = """            if status.ready {
                let config = migration::load_runtime_config(&paths, world)?;
                let descriptor = storage.load_world_descriptor(world)?;
                if status.manual_configuration {
                    let live = host_readiness::local_runtime_readiness(
                        &paths,
                        world,
                        descriptor.compatibility_fingerprint,
                    )?;
                    if live != HostRuntimeReadinessV1::Ready {
                        anyhow::bail!(
                            "manual Advanced runtime is configured but not authoritatively verified; launch it through the shared SwarmCraft runtime once so the authenticated Fabric compatibility handshake can prove this exact configuration"
                        );
                    }
                } else {
                    host_readiness::record_runtime_verified(
                        &paths,
                        world,
                        &config,
                        descriptor.compatibility_fingerprint,
                    )?;
                }
                let world_config = storage.load_world_config(world)?;
"""
        text = text.replace(old, new, 1)
    write(path, text)


def manual_regression_test() -> None:
    path = "crates/swarm-cli/tests/runtime_setup_hardening.rs"
    text = read(path)
    if "RuntimeInstaller" not in text.split("\n", 20).__str__():
        text = text.replace(
            "use swarm_cli::migration::{",
            "use swarm_cli::runtime_installer::RuntimeInstaller;\nuse swarm_cli::migration::{",
            1,
        )
    old = 'host = os.environ["SWARMCRAFT_IPC_HOST"]\n'
    if old in text and 'if "-version" in sys.argv:' not in text:
        text = text.replace(
            'import os\nimport socket\nimport time\n\nhost = os.environ["SWARMCRAFT_IPC_HOST"]\n',
            'import os\nimport socket\nimport sys\nimport time\n\nif "-version" in sys.argv:\n    print(\'openjdk version "25.0.1"\', file=sys.stderr)\n    raise SystemExit(0)\n\nhost = os.environ["SWARMCRAFT_IPC_HOST"]\n',
            1,
        )
    if "manual_advanced_config_is_launchable_without_being_reclassified_as_missing_managed_runtime" not in text:
        text += """

#[test]
fn manual_advanced_config_is_launchable_without_being_reclassified_as_missing_managed_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fixture(temp.path().join("peer-manual-inspect"));
    let (java, server, bridge) = manual_runtime_files(temp.path(), "26.1.2", "0.19.3");
    let config = RuntimeLaunchConfig {
        java,
        server_jar: server,
        mod_jar: bridge,
        accept_eula: true,
        game_endpoint: Some("127.0.0.1:25565".into()),
    };
    save_runtime_config(&fixture.paths, fixture.world, &config).unwrap();

    let installer = RuntimeInstaller::new(&fixture.paths, &fixture.storage);
    let status = installer.inspect(fixture.world).unwrap();
    assert!(status.manual_configuration);
    assert!(status.ready, "valid manual runtime should be launchable without automatic managed re-resolution: {status:?}");
    assert!(status.components.iter().filter(|component| {
        matches!(
            component.kind,
            swarm_cli::runtime_installer::RuntimeComponentKind::MinecraftServer
                | swarm_cli::runtime_installer::RuntimeComponentKind::FabricLoader
                | swarm_cli::runtime_installer::RuntimeComponentKind::FabricApi
                | swarm_cli::runtime_installer::RuntimeComponentKind::SwarmcraftFabric
        )
    }).all(|component| !component.managed));

    let readiness = swarm_cli::host_readiness::local_runtime_readiness(
        &fixture.paths,
        fixture.world,
        fixture.storage.load_world_descriptor(fixture.world).unwrap().compatibility_fingerprint,
    )
    .unwrap();
    assert_eq!(readiness, swarm_network::HostRuntimeReadinessV1::Unverified);
}
"""
    write(path, text)


def main() -> None:
    installer_manual_status()
    runtime_verify_boundary()
    manual_regression_test()


if __name__ == '__main__':
    main()
