import { nativeBridge } from './native-bridge';
import { createWorldActions, type WorldActionsBackend } from './world-actions';

/**
 * Compatibility backend for individually migrated Svelte journeys.
 * Exact Tauri invoke command names and camelCase arguments originate from the
 * existing registered Rust commands; no renderer-side policy decisions.
 * Never assign to native .invoke or keep a signing key in the webview.
 */
export function nativeWorldBackend(): WorldActionsBackend {
  const native = nativeBridge();
  return {
    createWorld(request) {
      return native.invoke('create_world', request);
    },
    importWorld(request) {
      return native.invoke('import_world', request);
    },
    joinWorld(invite) {
      return native.invoke('join_world', { invite });
    },
    createInvite(request) {
      return native.invoke('create_invite', request);
    },
  };
}

export function nativeWorldActions() {
  return createWorldActions(nativeWorldBackend());
}
