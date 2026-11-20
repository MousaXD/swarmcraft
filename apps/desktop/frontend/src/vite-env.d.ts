/// <reference types="svelte" />
/// <reference types="vite/client" />

declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
      };
    };
    __swarmcraftSvelteCreateOwned?: boolean;
    __swarmcraftSvelteJoinOwned?: boolean;
    __swarmcraftSvelteImportOwned?: boolean;
    __swarmcraftSvelteInviteOwned?: boolean;
    __swarmcraftSvelteDiscoveryOwned?: boolean;
    __swarmcraftSvelteLeaveOwned?: boolean;
    __swarmcraftSvelteIdentityOwned?: boolean;
    __swarmcraftSvelteConnectivityOwned?: boolean;
    __swarmcraftSvelteConnectivity?: { refresh(): Promise<void> };
    __swarmcraftSvelteNetworkingOwned?: boolean;
    __swarmcraftSvelteWorldsOwned?: boolean;
    __swarmcraftSvelteWorlds?: {
      refresh(): Promise<import('./lib/world-list-state').NativeWorldEntry[]>;
      setSelected(id: string): void;
      setConnectivity(label: string): void;
    };
    __swarmcraftRefreshWorldsFromSvelte?: () => Promise<void>;
    __swarmcraftSelectWorldFromSvelte?: (id: string) => boolean;
    __swarmcraftAfterSvelteNetworkingChange?: (outcome: 'started' | 'stopped' | 'unknown') => Promise<void>;
    __swarmcraftSvelteIdentity?: { publish(value: string, source?: import('./lib/device-identity').IdentitySource): boolean };
    __swarmcraftResetSvelteLeave?: () => void;
    __swarmcraftAfterSvelteLeave?: () => Promise<void>;
    __swarmcraftResetSvelteInvite?: () => void;
    __swarmcraftAfterSvelteImport?: (worldId: string) => Promise<void>;
    __swarmcraftAfterSvelteJoin?: (worldId: string) => Promise<void>;
    __swarmcraftAfterSvelteCreate?: (worldId: string) => Promise<void>;
    __swarmcraftSvelteActivity?: {
      append(entry: string): boolean;
      clear(): void;
    };
    __swarmcraftStartup?: {
      fail(kind: string, error: unknown): void;
      begin(): void;
      ready(status?: { degraded?: boolean }): void;
      readonly failed: boolean;
    };
  }
}
export {};
