/**
 * Typed incremental bridge for Svelte controllers. Rust is the only authority.
 * This never mutates window.__TAURI__, especially its native read-only invoke.
 * Any command not listed here must continue via the existing backend adapter
 * until its exact Rust payload/error contract is checked and ported.
 */
export interface NativeCommandTable {
  initialize_node: { args: Record<string, never>; result: string };
  node_identity: { args: Record<string, never>; result: string };
  list_worlds: { args: Record<string, never>; result: string };
  create_world: {
    args: {
      name: string;
      minecraft: string;
      fabricLoader: string;
      compatibility: string;
      visibility: string;
    };
    result: string;
  };
  join_world: { args: { invite: string }; result: string };
  leave_world: { args: { world: string }; result: string };
  create_invite: {
    args: { world: string; expiresMinutes: number; bootstrapAddrs: string[] };
    result: string;
  };
  world_status: { args: { world: string }; result: string };
  host_readiness: { args: { world: string }; result: string };
  migration_capabilities: { args: Record<string, never>; result: string };
}

export type NativeCommandName = keyof NativeCommandTable;
export type NativeCommandArgs<K extends NativeCommandName> = NativeCommandTable[K]['args'];
export type NativeCommandResult<K extends NativeCommandName> = NativeCommandTable[K]['result'];
export type NativeInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

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
  // Read a reference only. NEVER assign to the native core or its invoke slot.
  const core = window.__TAURI__?.core;
  return createNativeBridge(core?.invoke?.bind(core));
}
