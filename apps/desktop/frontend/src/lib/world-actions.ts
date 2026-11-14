/**
 * Stageable Svelte world-action coordinator using the exact existing
 * app.js/backend-adapter.js command contracts. It does not infer world
 * authority, signed membership, Fabric readiness or hosting eligibility.
 */
export type WorldVisibility = 'private' | 'unlisted' | 'public';
export type CompatibilityProfile = string;

export interface NewWorldInput {
  name: string;
  minecraft: string;
  fabricLoader: string;
  compatibility: CompatibilityProfile;
  visibility: WorldVisibility;
}
export interface ImportWorldInput {
  source: string;
  name: string;
  minecraft: string;
  fabricLoader: string;
  visibility: WorldVisibility;
  serverMods: readonly string[] | string;
  noServerMods: boolean;
}
export interface NativeImportRequest {
  source: string;
  name: string;
  minecraft: string;
  fabricLoader: string;
  visibility: WorldVisibility;
  serverMods: string[];
  noServerMods: boolean;
}
export type ImportResult = { worldId: string; result: Record<string, unknown> };

export interface WorldActionsBackend {
  createWorld(request: NewWorldInput): Promise<string>;
  importWorld(request: NativeImportRequest): Promise<string | Record<string, unknown>>;
  joinWorld(invite: string): Promise<string>;
  createInvite(request: { world: string; expiresMinutes: number; bootstrapAddrs: string[] }): Promise<string>;
}
export class WorldFormError extends Error {
  constructor(public readonly field: string, message: string) {
    super(message);
    this.name = 'WorldFormError';
  }
}
function required(value: unknown, field: string, message: string): string {
  const result = String(value ?? '').trim();
  if (!result) throw new WorldFormError(field, message);
  return result;
}
export function validateCreate(input: NewWorldInput): NewWorldInput {
  return {
    name: required(input.name, 'createName', 'World name is required.'),
    minecraft: required(input.minecraft, 'createMinecraft', 'Minecraft version is required.'),
    fabricLoader: required(input.fabricLoader, 'createLoader', 'Fabric Loader version is required.'),
    compatibility: required(input.compatibility, 'createCompatibility', 'Compatibility profile is required.'),
    visibility: input.visibility,
  };
}
export function validateImport(input: ImportWorldInput): NativeImportRequest {
  const serverMods = (Array.isArray(input.serverMods)
    ? input.serverMods.map(String)
    : String(input.serverMods ?? '').split(/\r?\n/))
    .map(v => v.trim()).filter(Boolean);
  const result = {
    source: required(input.source, 'importSource', 'Minecraft world folder is required.'),
    name: required(input.name, 'importName', 'World display name is required.'),
    minecraft: required(input.minecraft, 'importMinecraft', 'Exact Minecraft version is required.'),
    fabricLoader: required(input.fabricLoader, 'importLoader', 'Exact Fabric Loader version is required.'),
    visibility: required(input.visibility, 'importVisibility', 'Visibility is required.') as WorldVisibility,
    serverMods,
    noServerMods: input.noServerMods === true,
  };
  if (!result.serverMods.length && !result.noServerMods) {
    throw new WorldFormError('importNoServerMods', 'Add every required third-party server mod JAR, or explicitly confirm that no third-party server mods are required.');
  }
  if (result.serverMods.length && result.noServerMods) {
    throw new WorldFormError('importServerMods', 'Choose either required server mod JARs or the no-third-party-server-mods confirmation, not both.');
  }
  // Never synthesize EULA acceptance, verified modpack or runtime identity.
  return result;
}
export function parseImportedWorld(raw: unknown): ImportResult {
  let result = raw;
  if (typeof raw !== 'object' || raw === null) {
    try {
      result = JSON.parse(String(raw ?? ''));
    } catch {
      throw new Error('Import result was not valid JSON.');
    }
  }
  if (!result || typeof result !== 'object' || Array.isArray(result)) {
    throw new Error('Import result was not an object.');
  }
  const obj = result as Record<string, unknown>;
  const worldId = String(obj.world_id ?? obj.worldId ?? '').trim();
  if (!worldId) throw new Error('Import result did not include a world_id.');
  return { worldId, result: obj };
}
export function parseCreatedWorld(raw: unknown): string {
  const match = String(raw ?? '').match(/World ID:\s*(scworld:[^\s]+)/);
  if (!match) throw new Error('World creation did not return a canonical World ID.');
  return match[1];
}

export function createWorldActions(backend: WorldActionsBackend) {
  const active = new Set<string>();
  async function once<T>(action: string, execute: () => Promise<T>): Promise<T> {
    if (active.has(action)) throw new Error(action + ' is already running. Wait for the current operation to finish.');
    active.add(action);
    try {
      return await execute();
    } finally {
      active.delete(action);
    }
  }
  return Object.freeze({
    create(input: NewWorldInput): Promise<string> {
      const request = validateCreate(input);
      return once('Create world', async () => parseCreatedWorld(await backend.createWorld(request)));
    },
    import(input: ImportWorldInput): Promise<ImportResult> {
      const request = validateImport(input);
      return once('Import world', async () => parseImportedWorld(await backend.importWorld(request)));
    },
    join(invite: string): Promise<string> {
      const value = required(invite, 'joinInvite', 'A signed invite is required.');
      // Canonical membership and signed-invite validation remain in Rust.
      return once('Join world', () => backend.joinWorld(value));
    },
    invite(world: string, expiresMinutes: number, bootstrapAddrs: readonly string[]): Promise<string> {
      const id = required(world, 'inviteWorld', 'Select a world before creating an invite.');
      if (!Number.isSafeInteger(expiresMinutes) || expiresMinutes <= 0) {
        throw new WorldFormError('inviteMinutes', 'Choose a valid invite expiry.');
      }
      return once('Create invite', () => backend.createInvite({
        world: id,
        expiresMinutes,
        bootstrapAddrs: bootstrapAddrs.map(a => a.trim()).filter(Boolean),
      }));
    },
  });
}
