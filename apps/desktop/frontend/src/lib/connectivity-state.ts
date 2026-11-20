import { writable, type Readable } from 'svelte/store';

export type NativeConnectivityPhase = 'unchecked' | 'checking' | 'checked' | 'unavailable';
export type NativeConnectivityTone = 'neutral' | 'safe' | 'warning' | 'danger';
export interface NativeConnectivityState {
  phase: NativeConnectivityPhase;
  label: string;
  detail: string;
  tone: NativeConnectivityTone;
  busy: boolean;
  verifiedReadCount: number;
}
export interface NativeConnectivityController {
  state: Readable<NativeConnectivityState>;
  refresh(): Promise<void>;
  invalidate(detail?: string): void;
}

const labels: Record<string, { label: string; detail: string; tone: NativeConnectivityTone }> = {
  nat_status_unknown: { label: 'Network path not verified', detail: 'Rust has not established a usable path.', tone: 'warning' },
  direct_reachable: { label: 'Direct path available', detail: 'Rust reports direct reachability. This is not hosting authorization.', tone: 'safe' },
  hole_punched: { label: 'NAT traversal succeeded', detail: 'Rust reports a direct traversed path, not hosting eligibility.', tone: 'safe' },
  relay_connected: { label: 'Relay path available', detail: 'Rust reports a usable relay. This does not grant hosting eligibility.', tone: 'safe' },
  relay_required: { label: 'Relay required', detail: 'A relay is needed for reliable peer connectivity.', tone: 'warning' },
  private_unreachable: { label: 'Inbound peer path unavailable', detail: 'No usable inbound peer path is reported.', tone: 'danger' },
  bootstrap_unavailable: { label: 'Discovery unavailable', detail: 'Rust could not reach bootstrap discovery.', tone: 'danger' },
  no_viable_path: { label: 'No viable peer path', detail: 'Direct, NAT-traversed and relayed paths are unavailable.', tone: 'danger' },
};

export function parseNativeConnectivity(raw: unknown): Pick<NativeConnectivityState, 'label' | 'detail' | 'tone'> {
  const value: unknown = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Connectivity response is not an object.');
  }
  const source = value as Record<string, unknown>;
  const state = source.state ?? source.connectivity_state ?? source.connectivityState ??
    source.path_state ?? source.pathState ?? source.status;
  if (typeof state !== 'string') throw new Error('Connectivity state was missing.');
  const normalized = state.trim().toLowerCase().replace(/[\s-]+/g, '_');
  // Render only allowlisted status labels. Raw JSON may contain peer addresses,
  // filesystem paths, token-like error text or diagnostic arguments.
  return Object.prototype.hasOwnProperty.call(labels, normalized) ? labels[normalized] : {
    label: 'Unknown native network state',
    detail: 'Rust returned an unsupported state. Retry or inspect sanitized diagnostics.',
    tone: 'warning',
  };
}

export function createNativeConnectivityController(
  read: () => Promise<unknown>,
  timeoutMs = 12_000,
): NativeConnectivityController {
  const state = writable<NativeConnectivityState>({
    phase: 'unchecked', label: 'Not checked',
    detail: 'Check the native network state. Connectivity never grants hosting consent.',
    tone: 'neutral', busy: false, verifiedReadCount: 0,
  });
  let inflight: Promise<void> | null = null;
  let verifiedReadCount = 0;
  let generation = 0;
  return {
    state: { subscribe: state.subscribe },
    invalidate(detail = 'The previous network status may be stale. Check native connectivity again.') {
      ++generation;
      inflight = null;
      state.set({
        phase: 'unchecked', label: 'Not checked',
        detail, tone: 'neutral', busy: false, verifiedReadCount,
      });
    },
    refresh() {
      if (inflight) return inflight;
      const version = ++generation;
      state.set({
        phase: 'checking', label: 'Checking native network…',
        detail: 'Reading the local Rust connectivity diagnostic.',
        tone: 'neutral', busy: true, verifiedReadCount,
      });
      let timer: ReturnType<typeof setTimeout> | undefined;
      const operation = Promise.race([
        Promise.resolve().then(read),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error('Native connectivity check timed out.')), timeoutMs);
        }),
      ]).then(raw => {
        const view = parseNativeConnectivity(raw);
        if (version !== generation) return;
        verifiedReadCount += 1;
        state.set({ phase: 'checked', ...view, busy: false, verifiedReadCount });
      }).catch(() => {
        if (version !== generation) return;
        state.set({
          phase: 'unavailable', label: 'Native diagnostics unavailable',
          detail: 'The Rust network diagnostic could not complete. Check networking and retry.',
          tone: 'warning', busy: false, verifiedReadCount,
        });
      }).finally(() => {
        if (timer !== undefined) clearTimeout(timer);
      });
      const finished = operation.finally(() => {
        if (inflight === finished) inflight = null;
      });
      inflight = finished;
      return finished;
    },
  };
}
