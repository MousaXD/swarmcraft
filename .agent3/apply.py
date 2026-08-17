from pathlib import Path
import shutil

ROOT = Path('.')


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f'{path}: expected exactly one replacement, found {count}: {old[:120]!r}')
    target.write_text(text.replace(old, new, 1))


parts = sorted((ROOT / '.agent3').glob('part[0-9][0-9]'))
if len(parts) != 7:
    raise RuntimeError(f'expected 7 server_mods.rs parts, found {len(parts)}')
server_mods = ''.join(part.read_text() for part in parts)
(ROOT / 'crates/swarm-cli/src/server_mods.rs').write_text(server_mods)

replace_once(
    'crates/swarm-cli/src/lib.rs',
    'pub mod authority_permit;\npub mod migration;\n',
    'pub mod authority_permit;\npub mod migration;\npub mod server_mods;\n',
)

replace_once(
    'crates/swarm-cli/src/main.rs',
    'use swarm_cli::migration::{self, RuntimeLaunchConfig, TransferPrepareResult};\n',
    'use swarm_cli::migration::{self, RuntimeLaunchConfig, TransferPrepareResult};\nuse swarm_cli::server_mods;\n',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '        compatibility: String,\n        /// private, unlisted, or public.\n',
    '        compatibility: String,\n        /// Exact third-party Fabric server mods to bind into this world runtime profile.\n        #[arg(long = "server-mod")]\n        server_mod: Vec<PathBuf>,\n        /// private, unlisted, or public.\n',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '    /// Inspect the canonical execution compatibility manifest and authority eligibility.\n    Compatibility { world: String },\n',
    '''    /// Inspect the canonical execution compatibility manifest and authority eligibility.\n    Compatibility { world: String },\n    /// Inspect local third-party server mods against the canonical world runtime profile.\n    ModsStatus {\n        world: String,\n        #[arg(long)]\n        json: bool,\n    },\n    /// Add the exact locally supplied JAR for a canonical server-mod requirement.\n    ModsAdd { world: String, jar: PathBuf },\n    /// Remove a locally installed third-party server mod by Fabric mod ID.\n    ModsRemove { world: String, mod_id: String },\n    /// Print the persistent per-world third-party server mods directory.\n    ModsPath { world: String },\n''',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '        WorldCommand::Create { name, minecraft, fabric_loader, compatibility, visibility } => {\n',
    '        WorldCommand::Create { name, minecraft, fabric_loader, compatibility, server_mod, visibility } => {\n',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''            let legacy_hash =\n                Hash32::from_domain_bytes(b"swarmcraft/legacy-compatibility/v1\\0", compatibility.as_bytes());\n            let manifest = RuntimeCompatibilityManifestV1 {\n''',
    '''            let legacy_hash =\n                Hash32::from_domain_bytes(b"swarmcraft/legacy-compatibility/v1\\0", compatibility.as_bytes());\n            let mut required_server_mods = vec![ArtifactRequirementV1 {\n                artifact_id: "swarmcraft.legacy-compatibility".into(),\n                version: "1".into(),\n                artifact_hash: legacy_hash,\n                side: ArtifactSideV1::Server,\n                provider_hint: None,\n            }];\n            required_server_mods.extend(server_mods::requirements_from_jars(&server_mod)?);\n            let manifest = RuntimeCompatibilityManifestV1 {\n''',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''                required_server_mods: vec![ArtifactRequirementV1 {\n                    artifact_id: "swarmcraft.legacy-compatibility".into(),\n                    version: "1".into(),\n                    artifact_hash: legacy_hash,\n                    side: ArtifactSideV1::Server,\n                    provider_hint: None,\n                }],\n''',
    '                required_server_mods,\n',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''            sign_world_config(&identity, &mut config)?;\n            storage.save_world_config(&config)?;\n            println!("Created world: {name}");\n''',
    '''            sign_world_config(&identity, &mut config)?;\n            storage.save_world_config(&config)?;\n            for source in &server_mod {\n                server_mods::add_local_mod(paths, world_id, &config.compatibility, source)?;\n            }\n            println!("Created world: {name}");\n''',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''                    let fingerprint = config.compatibility_fingerprint()?;\n                    println!("Compatibility fingerprint: {fingerprint}");\n''',
    '''                    let fingerprint = config.compatibility_fingerprint()?;\n                    let mod_readiness = server_mods::evaluate_world_mods(paths, world, &config.compatibility)?;\n                    println!("Compatibility fingerprint: {fingerprint}");\n''',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''                    println!("Datapacks: {}", config.compatibility.datapacks.len());\n                    println!("Genesis match: {}", fingerprint == metadata.genesis.compatibility_fingerprint);\n''',
    '''                    println!("Datapacks: {}", config.compatibility.datapacks.len());\n                    println!("Genesis match: {}", fingerprint == metadata.genesis.compatibility_fingerprint);\n                    println!("Server mods ready: {}", mod_readiness.ready);\n''',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''                    if eligible && fingerprint == metadata.genesis.compatibility_fingerprint {\n                        println!("Authority eligibility: Compatible");\n                    } else {\n                        println!("Authority eligibility: Replica only: not authority eligible");\n                    }\n''',
    '''                    if eligible && fingerprint == metadata.genesis.compatibility_fingerprint && mod_readiness.ready {\n                        println!("Authority eligibility: Compatible");\n                    } else if !mod_readiness.ready {\n                        println!("Authority eligibility: Replica only: server mods missing or incompatible");\n                    } else {\n                        println!("Authority eligibility: Replica only: not authority eligible");\n                    }\n''',
)
replace_once(
    'crates/swarm-cli/src/main.rs',
    '''        WorldCommand::Seed { world, enabled } => {\n''',
    '''        WorldCommand::ModsStatus { world, json } => {\n            let world = parse_world(&world)?;\n            let config = storage\n                .load_world_config(world)\n                .context("canonical runtime profile is not yet synchronized")?;\n            let status = server_mods::evaluate_world_mods(paths, world, &config.compatibility)?;\n            if json {\n                println!("{}", serde_json::to_string_pretty(&status)?);\n            } else {\n                println!("Server mods directory: {}", status.mods_dir.display());\n                println!("Ready: {}", status.ready);\n                println!("Canonical requirements: {}", status.required.len());\n                for required in &status.required {\n                    println!(\n                        "required {} {} {} {:?}",\n                        required.mod_id, required.version, required.artifact_hash, required.component_kind\n                    );\n                }\n                for installed in &status.installed {\n                    println!(\n                        "installed {} {} {} {:?}",\n                        installed.mod_id, installed.version, installed.artifact_hash, installed.environment\n                    );\n                }\n                for issue in &status.issues {\n                    println!("issue {:?}: {}", issue.kind, issue.message);\n                }\n            }\n        }\n        WorldCommand::ModsAdd { world, jar } => {\n            let world = parse_world(&world)?;\n            let config = storage\n                .load_world_config(world)\n                .context("canonical runtime profile is not yet synchronized")?;\n            let installed = server_mods::add_local_mod(paths, world, &config.compatibility, &jar)?;\n            println!(\n                "Installed {} {} with artifact hash {}",\n                installed.mod_id, installed.version, installed.artifact_hash\n            );\n        }\n        WorldCommand::ModsRemove { world, mod_id } => {\n            let world = parse_world(&world)?;\n            storage.load_world(world)?;\n            let removed = server_mods::remove_local_mod(paths, world, &mod_id)?;\n            if removed.is_empty() {\n                println!("No local server mod with id {mod_id} was installed.");\n            } else {\n                println!("Removed {} from local server mods.", removed[0].display());\n            }\n        }\n        WorldCommand::ModsPath { world } => {\n            let world = parse_world(&world)?;\n            storage.load_world(world)?;\n            println!("{}", server_mods::mods_dir(paths, world).display());\n        }\n        WorldCommand::Seed { world, enabled } => {\n''',
)

replace_once(
    'crates/swarm-cli/src/migration.rs',
    'use crate::authority_permit::PermitWatch;\n',
    'use crate::{authority_permit::PermitWatch, server_mods};\n',
)
replace_once(
    'crates/swarm-cli/src/migration.rs',
    '''    storage.verify_snapshot(&latest)?;\n    verify_snapshot_signature(&latest)?;\n    ensure_authority_generation(storage, &identity, &epoch)?;\n\n    let runtime = paths.root.join("runtime").join(options.world.to_hex());\n''',
    '''    storage.verify_snapshot(&latest)?;\n    verify_snapshot_signature(&latest)?;\n    ensure_authority_generation(storage, &identity, &epoch)?;\n    let world_config = storage\n        .load_world_config(options.world)\n        .context("canonical runtime profile is not synchronized")?;\n    let mod_readiness = server_mods::evaluate_world_mods(paths, options.world, &world_config.compatibility)?;\n    if !mod_readiness.ready {\n        let details = mod_readiness\n            .issues\n            .iter()\n            .map(|issue| issue.message.as_str())\n            .collect::<Vec<_>>()\n            .join("; ");\n        bail!("local device is not server-mod ready for authority runtime: {details}");\n    }\n\n    let runtime = paths.root.join("runtime").join(options.world.to_hex());\n''',
)
replace_once(
    'crates/swarm-cli/src/migration.rs',
    '''    fs::copy(&options.mod_jar, runtime.join("mods/swarmcraft-fabric.jar"))\n        .with_context(|| format!("cannot install Fabric bridge from {}", options.mod_jar.display()))?;\n    fs::write(runtime.join("eula.txt"), "eula=true\\n")?;\n''',
    '''    fs::copy(&options.mod_jar, runtime.join("mods/swarmcraft-fabric.jar"))\n        .with_context(|| format!("cannot install Fabric bridge from {}", options.mod_jar.display()))?;\n    server_mods::install_verified_user_mods(\n        paths,\n        options.world,\n        &world_config.compatibility,\n        &runtime.join("mods"),\n    )?;\n    fs::write(runtime.join("eula.txt"), "eula=true\\n")?;\n''',
)

shutil.rmtree(ROOT / '.agent3')
workflow = ROOT / '.github/workflows/agent3-apply.yml'
if workflow.exists():
    workflow.unlink()
