/**
 * Exact named command registry from Rust Tauri 2 at issue #87 baseline.
 * Derived from apps/desktop/src-tauri/src command function signatures.
 *
 * Rust remains the authority for resource admission, membership, quorum,
 * fencing, snapshots, signed invites and hosting. 'unknown' response types
 * must be normalized and validated by a caller before presentation.
 */
export interface NativeCommandTable {
  initialize_node: { args: Record<string, never>; result: string };
  node_identity: { args: Record<string, never>; result: string };
  list_worlds: { args: Record<string, never>; result: string };
  minecraft_versions: { args: { includeSnapshots: boolean; refresh: boolean }; result: unknown };
  fabric_loader_versions: { args: { minecraftVersion: string; refresh: boolean }; result: unknown };
  validate_fabric_selection: { args: { minecraftVersion: string; fabricLoaderVersion: string }; result: void };
  create_world: { args: { name: string; minecraft: string; fabricLoader: string; compatibility: string; visibility: string }; result: string };
  import_world: { args: { source: string; name: string; minecraft: string; fabricLoader: string; visibility: string; serverMods: string[]; noServerMods: boolean }; result: string };
  join_world: { args: { invite: string }; result: string };
  leave_world: { args: { world: string }; result: string };
  create_invite: { args: { world: string; expiresMinutes: number; bootstrapAddrs: string[] }; result: string };
  world_status: { args: { world: string }; result: string };
  host_readiness: { args: { world: string }; result: string };
  world_mods_status: { args: { world: string }; result: string };
  world_mods_add: { args: { world: string; jarPath: string }; result: string };
  world_mods_remove: { args: { world: string; modId: string }; result: string };
  open_world_mods_folder: { args: { world: string }; result: string };
  world_compatibility: { args: { world: string }; result: string };
  world_conflicts: { args: { world: string }; result: string };
  set_background_seeding: { args: { world: string; enabled: boolean }; result: string };
  world_peers: { args: { world: string }; result: string };
  verify_world: { args: { world: string }; result: string };
  export_world: { args: { world: string; destination: string }; result: string };
  recover_world: { args: { world: string; snapshot: number; destination: string }; result: string };
  migration_capabilities: { args: Record<string, never>; result: string };
  migration_status: { args: { world: string }; result: string };
  manual_transfer_step: { args: { world: string; action: string; value: string | null }; result: string };
  wake_world: { args: { world: string }; result: string };
  configure_world_runtime: { args: { world: string; java: string; serverJar: string; modJar: string; acceptEula: boolean; gameEndpoint: string | null }; result: string };
  runtime_status: { args: { world: string }; result: string };
  runtime_plan: { args: { world: string }; result: string };
  runtime_install: { args: { world: string; acceptEula: boolean }; result: string };
  runtime_repair: { args: { world: string }; result: string };
  runtime_verify: { args: { world: string }; result: string };
  runtime_launch: { args: { world: string }; result: number | null };
  runtime_diagnostics: { args: { world: string }; result: string };
  connectivity_diagnostics: { args: Record<string, never>; result: string };
  canonicalize_modpack: { args: { request: Record<string, unknown> }; result: unknown };
  create_canonical_world: { args: { request: Record<string, unknown> }; result: unknown };
  modrinth_search: { args: { query: Record<string, unknown> }; result: unknown };
  modrinth_project: { args: { projectId: string }; result: unknown };
  modrinth_versions: { args: { projectId: string; filter: Record<string, unknown> }; result: unknown };
  modrinth_resolve: { args: { request: Record<string, unknown> }; result: unknown };
  modrinth_resolve_project: { args: { projectId: string; minecraftVersion: string; loader: string; environment: unknown }; result: unknown };
  modrinth_download: { args: { locator: Record<string, unknown>; stagingSession: string; maxBytes: unknown }; result: unknown };
  curseforge_provider_status: { args: Record<string, never>; result: unknown };
  curseforge_search: { args: { query: string; minecraft: string; loader: string; environment: string; index: number; pageSize: number }; result: unknown };
  curseforge_project: { args: { projectId: number }; result: unknown };
  curseforge_versions: { args: { projectId: number; minecraft: string; loader: string }; result: unknown };
  curseforge_resolve: { args: { fileId: number; minecraft: string; loader: string; environment: string }; result: unknown };
  curseforge_resolve_project: { args: { projectId: number; minecraft: string; loader: string; environment: string }; result: unknown };
  curseforge_download: { args: { fileId: number; stagingSession: string }; result: unknown };
  provider_staging_dir: { args: Record<string, never>; result: string };
  inspect_mod_artifact: { args: { path: string }; result: unknown };
  discovery_search: { args: { query: string | null }; result: unknown };
  discovery_resolve: { args: { world: string }; result: unknown };
  ensure_daemon_running: { args: { listen: string }; result: number };
  start_daemon: { args: { listen: string }; result: number };
  stop_daemon: { args: Record<string, never>; result: void };
  stop_host: { args: { world: string }; result: string };
  host_world: { args: { world: string; java: string; serverJar: string; modJar: string; acceptEula: boolean }; result: number };
}

export type NativeCommandName = keyof NativeCommandTable;
export type NativeCommandArgs<K extends NativeCommandName> = NativeCommandTable[K]['args'];
export type NativeCommandResult<K extends NativeCommandName> = NativeCommandTable[K]['result'];
export type NativeInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

/** Never mutate the native read-only core.invoke descriptor (PR #88). */
export function createNativeBridge(invoke: NativeInvoke | undefined) {
  return Object.freeze({
    invoke<K extends NativeCommandName>(command: K, args: NativeCommandArgs<K>): Promise<NativeCommandResult<K>> {
      if (typeof invoke !== 'function') {
        return Promise.reject(new Error('Tauri is unavailable. Open SwarmCraft as the installed desktop app.'));
      }
      return invoke<NativeCommandResult<K>>(command, args);
    },
  });
}

export function nativeBridge() {
  const core = window.__TAURI__?.core;
  return createNativeBridge(core?.invoke?.bind(core));
}
