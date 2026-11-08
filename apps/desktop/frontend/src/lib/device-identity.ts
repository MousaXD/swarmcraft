import { writable, type Readable } from 'svelte/store';

export type IdentityPhase = 'checking' | 'ready' | 'unavailable';
export interface IdentityState {
  phase: IdentityPhase;
  text: string;
  busy: boolean;
  error: string;
}
export type IdentitySource = 'setup' | 'identity' | 'unavailable';
export interface IdentityController {
  state: Readable<IdentityState>;
  publishLegacy(value: unknown, source?: IdentitySource): boolean;
  refresh(): Promise<void>;
}

/**
 * Renderer-only identity presentation. Rust is authoritative for node identity
 * and initialization. Never print raw native errors (they may contain secrets).
 */
export function createIdentityController(readNative: () => Promise<string>): IdentityController {
  const state = writable<IdentityState>({
    phase: 'checking', text: 'Checking identity…', busy: false, error: '',
  });
  let request: Promise<void> | null = null;
  let generation = 0;

  function present(value: unknown): IdentityState {
    const text = typeof value === 'string' ? value.trim() : '';
    return text
      ? { phase: 'ready', text, busy: false, error: '' }
      : {
          phase: 'unavailable', text: 'Device identity is unavailable.',
          busy: false, error: 'Set up this device and refresh its identity.',
        };
  }

  return {
    state: { subscribe: state.subscribe },
    publishLegacy(value, source = 'identity') {
      ++generation;
      // initialize_node can produce setup text, not a verified node_identity.
      if (source === 'setup') {
        state.set({ phase: 'checking', text: String(value || 'Device setup complete. Reading identity…'), busy: false, error: '' });
      } else if (source === 'unavailable') {
        state.set(present(''));
      } else {
        state.set(present(value));
      }
      return true;
    },
    refresh() {
      if (request) return request;
      const version = ++generation;
      state.update(s => ({ ...s, busy: true, error: '' }));
      const task = Promise.resolve().then(readNative).then(value => {
        if (version === generation) state.set(present(value));
      }).catch(() => {
        if (version !== generation) return;
        state.set({
          phase: 'unavailable', text: 'Device identity could not be read.',
          busy: false, error: 'Native identity lookup failed. Check device setup and retry.',
        });
      });
      request = task.finally(() => {
        if (request === completion) request = null;
      });
      const completion = request;
      return completion;
    },
  };
}
