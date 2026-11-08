/// <reference types="svelte" />
/// <reference types="vite/client" />

declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
      };
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
