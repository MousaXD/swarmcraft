from pathlib import Path
import sys

# Temporary bootstrap for the integration-only reconciliation driver.  It runs
# before the driver is loaded on CPython and makes its text anchors match the
# actual integrated CLI, then the resulting temporary files are removed before
# the final release-candidate handoff.
if sys.argv and sys.argv[0].endswith("reconcile_runtime_player_journey.py"):
    path = Path(sys.argv[0])
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
    path.write_text(text)
