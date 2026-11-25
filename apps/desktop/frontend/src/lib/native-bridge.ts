/**
 * Exact named command registry from Rust Tauri 2 at issue #87 baseline.
 * Derived from apps/desktop/src-tauri/src command function signatures.
 *
 * Rust remains the authority for resource admission, membership, quorum,
 * fencing, snapshots, signed invites and hosting. 'unknown' response types
 * must be normalized and validated by a caller before presentation.
 */
export interface HostingContributionReadiness {
  world_id: string;
  consented: boolean;
  selected_heap_mib: number;
  physical_mib: number | null;
  available_mib: number | null;
  estimated_native_overhead_mib: number;
  client_headroom_mib: number;
  os_headroom_mib: number;
  daemon_headroom_mib: number;
  may_host_now: boolean;
  reason: string;
  hard_process_memory_cap_guaranteed: boolean;
  process_memory?: null |
    { mode: 'heap_only' } |
    { mode: 'linux_cgroup_configured'; limit_mib: number } |
    { mode: 'linux_cgroup_active'; limit_mib: number; current_bytes: number; peak_bytes: number | null } |
    { mode: 'unavailable'; detail: string };
  checkpoint_state?: 'no_pending_checkpoint' | 'stopping_at_checkpoint' | 'checkpoint_blocked' | null;
  consent_revision: string | null;
}

/** Read-only, freshness-checked native status, not a renderer-issued route grant. */
export interface NativeGameRouteStatus {
  world_id: string;
  state: 'verified' | 'waiting_for_quorum' | 'syncing_checkpoint' | 'restoring_runtime' | 'verifying_route' | 'blocked' | 'unavailable';
  quorum_verified: boolean;
  hosting_consent_verified: boolean;
  server_ready: boolean;
  route_verified: boolean;
  local_gateway: string | null;
  authority_peer_id: string | null;
  epoch: number | null;
  fencing_token: number | null;
  snapshot_hash: string | null;
  verified_at_unix_ms: number | null;
  valid_until_unix_ms: number | null;
  diagnostic?: 'gateway_unavailable' | 'peer_link_unavailable' | 'host_proof_unavailable' | 'route_check_timed_out' | 'authority_changed';
  recent_session?: { state: 'checking_route' | 'opening_stream' | 'route_refused' | 'route_timed_out' | 'stream_admission_failed' | 'stream_interrupted' | 'session_ended' | 'overloaded'; age_ms: number };
}

export interface NativeCommandTable {
  initialize_node: { args: Record<string, never>; result: string };
  node_identity: { args: Record<string, never>; result: string };
  list_worlds: { args: Record<string, never>; result: string };
  minecraft_versions: { args: { includeSnapshots: boolean; refresh: boolean }; result: unknown };
  fabric_loader_versions: { args: { minecraftVersion: string; refresh: boolean }; result: unknown };
  validate_fabric_selection: { args: { minecraftVersion: string; fabricLoaderVersion: string }; result: void };
  create_world: { args: { name: string; minecraft: string; fabricLoader: string; compatibility: string; visibility: string; region?: string; language?: string }; result: string };
  pick_import_folder: { args: Record<string, never>; result: string | null };
  preview_import_world: { args: { source: string; name: string; minecraft: string; fabricLoader: string; visibility: string; serverMods: string[]; noServerMods: boolean }; result: string };
  import_world: { args: { source: string; name: string; minecraft: string; fabricLoader: string; visibility: string; serverMods: string[]; noServerMods: boolean; expectedCompatibility?: string }; result: string };
  join_world: { args: { invite: string; hostCandidate: boolean }; result: string };
  leave_world: { args: { world: string }; result: string };
  create_invite: { args: { world: string; expiresMinutes: number; bootstrapAddrs: string[] }; result: string };
  inspect_invite: { args: { invite: string }; result: string };
  revoke_invite: { args: { world: string; invite: string; inviteId: string; confirmRevocation: boolean }; result: string };
  world_status: { args: { world: string }; result: string };
  host_readiness: { args: { world: string }; result: string };
  /** Select exactly one local loopback gateway; this does NOT verify a route. */
  game_route_select: { args: { world: string }; result: string };
  /** Fresh signed-quorum, consent and Fabric proof from the live daemon. */
  game_route_status: { args: { world: string }; result: NativeGameRouteStatus };
  client_profile: { args: { world: string; modsDir: string | null }; result: string };
  hosting_contribution_get: { args: { world: string }; result: HostingContributionReadiness };
  hosting_contribution_set: { args: { world: string; heapMib: number }; result: HostingContributionReadiness };
  world_mods_status: { args: { world: string }; result: string };
  world_mods_acquire: { args: { world: string }; result: string };
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
  recover_copy: { args: { world: string; checkpoint: string; confirmNewIdentity: boolean; acknowledgedConflicts?: string }; result: string };
  recovery_review: { args: { world: string }; result: string };
  recovery_export: { args: { world: string; checkpoint: string; acknowledgedConflicts?: string; destination: string }; result: string };
  conflict_export: { args: { world: string; branch: string; acknowledgedConflicts: string; evidenceOnly: boolean; destination: string }; result: string };
  migration_capabilities: { args: Record<string, never>; result: string };
  migration_status: { args: { world: string }; result: string };
  manual_transfer_step: { args: { world: string; action: string; value: string | null }; result: string };
  start_automatic_transfer: { args: { world: string; target: string; sourceEpoch: number }; result: string };
  transfer_progress: { args: { world: string }; result: string };
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
  support_history: { args: { world: string | null }; result: unknown };
  support_recording: { args: { world: string | null; enabled: boolean; confirm: boolean }; result: unknown };
  support_clear_history: { args: { world: string | null; confirm: boolean }; result: unknown };
  support_preview: { args: { world: string | null; confirm: boolean }; result: unknown };
  support_save: { args: { token: string; confirm: boolean }; result: string | null };
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
  discovery_search: { args: { query: string | null; region?: string | null; language?: string | null }; result: unknown };
  discovery_resolve: { args: { world: string }; result: unknown };
  directory_policy: { args: Record<string, never>; result: unknown };
  directory_hide: { args: { world: string; enabled: boolean }; result: unknown };
  directory_report: { args: { world: string; configHash: string; reason: string }; result: unknown };
  directory_dismiss_report: { args: { report: string }; result: unknown };
  world_listing: { args: { world: string; paused: boolean | null }; result: unknown };
  friend_settings: { args: Record<string, never>; result: unknown };
  friend_add: { args: { peer: string; publicKey: string; label: string }; result: unknown };
  friend_remove: { args: { peer: string }; result: unknown };
  friend_share_presence: { args: { peer: string; enabled: boolean }; result: unknown };
  friend_presence_mode: { args: { enabled: boolean }; result: unknown };
  friend_probe: { args: { peer: string }; result: unknown };
  friend_service_status: { args: Record<string, never>; result: unknown };
  friend_service_start: { args: Record<string, never>; result: unknown };
  friend_service_stop: { args: Record<string, never>; result: unknown };
  ensure_daemon_running: { args: { listen: string }; result: number };
  start_daemon: { args: { listen: string }; result: number };
  stop_daemon: { args: Record<string, never>; result: void };
  stop_host: { args: { world: string }; result: string };
  host_world: { args: { world: string; java: string; serverJar: string; modJar: string; acceptEula: boolean }; result: number | null };
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
