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
