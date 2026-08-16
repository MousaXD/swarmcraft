from pathlib import Path
import re

OLD = '0.1.0'
NEW = '0.2.0'

def replace_required(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        raise SystemExit(f'missing version anchor in {path}: {old!r}')
    p.write_text(text.replace(old, new))

replace_required('Cargo.toml', 'version = "0.1.0"', 'version = "0.2.0"')
replace_required('apps/desktop/package.json', '"version": "0.1.0"', '"version": "0.2.0"')
replace_required('apps/desktop/src-tauri/tauri.conf.json', '"version": "0.1.0"', '"version": "0.2.0"')
replace_required('minecraft/fabric/gradle.properties', 'mod_version=0.1.0', 'mod_version=0.2.0')

# Update explicit SwarmCraft-owned package versions if any crate is not inheriting
# workspace.package.version. Do not touch third-party dependency requirements.
for cargo in Path('.').glob('**/Cargo.toml'):
    if 'target' in cargo.parts:
        continue
    text = cargo.read_text()
    if cargo == Path('Cargo.toml'):
        continue
    if re.search(r'^version\s*=\s*"0\.1\.0"\s*$', text, re.M):
        text = re.sub(r'^version\s*=\s*"0\.1\.0"\s*$', 'version = "0.2.0"', text, flags=re.M)
        cargo.write_text(text)

# Cargo.lock embeds versions for workspace packages. Update only packages whose
# names are owned by SwarmCraft; dependency package versions are untouched.
lock = Path('Cargo.lock')
text = lock.read_text()
owned = {
    'swarm-cli', 'swarm-consensus', 'swarm-core', 'swarm-ipc',
    'swarm-network', 'swarm-protocol', 'swarm-storage', 'swarmcraft-desktop'
}
blocks = text.split('[[package]]')
for i in range(1, len(blocks)):
    block = blocks[i]
    m = re.search(r'\nname = "([^"]+)"', block)
    if m and m.group(1) in owned:
        block = re.sub(r'\nversion = "0\.1\.0"', '\nversion = "0.2.0"', block, count=1)
        blocks[i] = block
lock.write_text('[[package]]'.join(blocks))

# Protocol version is intentionally not changed here.
Path(__file__).unlink()
