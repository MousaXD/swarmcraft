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
# The GitHub Actions token used as the temporary integration workbench cannot
# push workflow-file changes. Apply packaging workflow edits separately through
# the repository connector, after the tested code commit lands.
text = text.replace('    reconcile_ci_packaging()\n', '')
path.write_text(text)

runpy.run_path('.github/scripts/finish_runtime_player_journey.py', run_name='__main__')
