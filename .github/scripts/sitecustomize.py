from pathlib import Path
import runpy

# Temporary integration-only bootstrap. The workflow runs this explicitly
# before the reconciliation driver and all workbench files are removed before handoff.
path = Path('.github/scripts/reconcile_runtime_player_journey.py')
text = path.read_text()
text = text.replace(
    '            "    /// Request a safe wake of a sleeping world.\\n",\n'
    '            "    /// Request a safe stop. Success is reported only after the Fabric save barrier, final checkpoint and durable sleep record complete.\\n"\n'
    '            "    Stop { world: String },\\n"\n'
    '            "    /// Request a safe wake of a sleeping world.\\n",',
    '            "    /// Request a safe wake of a sleeping world. Multi-member worlds remain blocked until a quorum transition exists.\\n",\n'
    '            "    /// Request a safe stop. Success is reported only after the Fabric save barrier, final checkpoint and durable sleep record complete.\\n"\n'
    '            "    Stop { world: String },\\n"\n'
    '            "    /// Request a safe wake of a sleeping world. Multi-member worlds remain blocked until a quorum transition exists.\\n",',
)
text = text.replace(
    '        create_anchor = "            storage.save_world_config(&world_config)?;"',
    '        create_anchor = "            storage.save_world_config(&config)?;"',
)
text = text.replace('paths.root.join("initial-world").join(world.to_hex())', 'paths.root.join("initial-world").join(world_id.to_hex())')
text = text.replace('                    world,\n                    snapshot_number: 1,', '                    world: world_id,\n                    snapshot_number: 1,')
text = text.replace(
    'OpenOptions::new().create(true).read(true).write(true).open(path)?;',
    'OpenOptions::new().create(true).read(true).write(true).truncate(false).open(path)?;',
)
text = text.replace('let _ = self.file.unlock();', 'let _ = FileExt::unlock(&self.file);')
text = text.replace('    reconcile_ci_packaging()\n', '')
path.write_text(text)

runpy.run_path('.github/scripts/finalize_manual_runtime.py', run_name='__main__')

installer_path = Path('crates/swarm-cli/src/runtime_installer.rs')
installer = installer_path.read_text()
installer = installer.replace('|config| manual_file_status(', '|config| Self::manual_file_status(')
installer = installer.replace('\n        manual_file_status(\n', '\n        Self::manual_file_status(\n')
if '    fn manual_file_status(\n' not in installer:
    anchor = '    fn server_mods_status(&self, world: WorldId) -> RuntimeComponentStatus {'
    if anchor not in installer:
        raise RuntimeError('RuntimeInstaller server_mods_status anchor moved')
    helper = '''    fn manual_file_status(
        kind: RuntimeComponentKind,
        path: &Path,
        ready_detail: &str,
    ) -> RuntimeComponentStatus {
        let ready = path.is_file();
        RuntimeComponentStatus {
            kind,
            state: if ready {
                RuntimeComponentState::Ready
            } else {
                RuntimeComponentState::Missing
            },
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

'''
    installer = installer.replace(anchor, helper + anchor, 1)
installer_path.write_text(installer)

runpy.run_path('.github/scripts/add_safe_stop_acceptance.py', run_name='__main__')
