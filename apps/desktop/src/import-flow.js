import './catalog-selectors.js';
import './provider-contract-bridge.js';
import './launcher-controller.js';
import './player-copy.js';

function invalid(field, message) {
  const error = new Error(message);
  error.field = field;
  throw error;
}

function normalizeServerMods(value) {
  const values = Array.isArray(value) ? value : String(value || '').split(/\r?\n/);
  return values.map((item) => String(item).trim()).filter(Boolean);
}

export function createImportRequest(input = {}) {
  const source = String(input.source || '').trim();
  const name = String(input.name || '').trim();
  const minecraft = String(input.minecraft || '').trim();
  const fabricLoader = String(input.fabricLoader || '').trim();
  const visibility = String(input.visibility || '').trim();
  const serverMods = normalizeServerMods(input.serverMods);
  const noServerMods = input.noServerMods === true;

  if (!source) invalid('importSource', 'Minecraft world folder is required.');
  if (!name) invalid('importName', 'World display name is required.');
  if (!minecraft) invalid('importMinecraft', 'Exact Minecraft version is required.');
  if (!fabricLoader) invalid('importLoader', 'Exact Fabric Loader version is required.');
  if (!visibility) invalid('importVisibility', 'Visibility is required.');
  if (!serverMods.length && !noServerMods) {
    invalid(
      'importNoServerMods',
      'Add every required third-party server mod JAR, or explicitly confirm that no third-party server mods are required.',
    );
  }
  if (serverMods.length && noServerMods) {
    invalid('importServerMods', 'Choose either required server mod JARs or the no-third-party-server-mods confirmation, not both.');
  }

  return {
    source,
    name,
    minecraft,
    fabricLoader,
    visibility,
    serverMods,
    noServerMods,
  };
}

export function parseImportResult(raw) {
  let result = raw;
  if (!result || typeof result !== 'object') {
    try {
      result = JSON.parse(String(raw || ''));
    } catch (error) {
      throw new Error(`Import result was not valid JSON: ${error}`);
    }
  }
  const worldId = String(result.world_id ?? result.worldId ?? '').trim();
  if (!worldId) throw new Error('Import result did not include a world_id.');
  return { worldId, result };
}
