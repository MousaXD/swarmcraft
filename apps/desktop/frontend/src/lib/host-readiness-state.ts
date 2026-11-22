import { writable, type Readable } from 'svelte/store';

/** This is a view-only native Rust host-readiness report, not authorization. */
export interface NativeReadinessView {
  phase: 'unchecked' | 'checking' | 'ready' | 'unavailable';
  world: string;
  title: string;
  detail: string;
  tone: 'neutral' | 'safe' | 'warning' | 'danger';
  busy: boolean;
  verifiedReadCount: number;
}
export interface NativeHostReadinessController {
  state: Readable<NativeReadinessView>;
  select(world: string): Promise<void>;
  clear(): void;
}

const UNKNOWN = 'SwarmCraft has not verified whether this device can safely shut down.';
const STATES: Record<string, [string, string, NativeReadinessView['tone']]> = {
  safe: ['Safe to shut down this PC', 'Another ready device can take over this world.', 'safe'],
  sleeping: ['Safe to shut down this PC', 'This world is durably stopped and its latest checkpoint is saved.', 'safe'],
  blocked_by_runtime: ['Keep this PC on', 'A successor is missing its Minecraft runtime.', 'warning'],
  blocked_by_mods: ['Keep this PC on', 'A successor is missing verified server mods.', 'warning'],
  syncing: ['Wait before shutting down', 'Another device is still syncing the latest world state.', 'warning'],
  world_will_stop: ['World will go offline', 'No other ready host is currently reachable.', 'warning'],
  blocked_by_quorum: ['World will go offline', 'A safe successor quorum is not available.', 'warning'],
  conflict: ['Host handoff unavailable', 'Conflicting world histories need attention.', 'danger'],
  degraded_safety: ['Keep this PC on', 'Safe host takeover has not been proven.', 'warning'],
  not_current_host: ['Shutdown safety not proven', 'This device is not the current host; takeover safety is unverified.', 'warning'],
  unknown: ['Checking shutdown safety…', UNKNOWN, 'neutral'],
};

export function parseNativeHostReadiness(response: unknown): Pick<NativeReadinessView, 'title' | 'detail' | 'tone'> {
  let raw: unknown = response;
  if (typeof raw === 'string') raw = JSON.parse(raw);
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new Error('Invalid Rust host readiness envelope.');
  }
  const body = raw as Record<string, unknown>;
  if (typeof body.state !== 'string') throw new Error('Missing native host readiness state.');
  const state = body.state.trim().toLowerCase().replace(/[\s-]+/g, '_');
  if (!Object.prototype.hasOwnProperty.call(STATES, state)) {
    return { title: 'Shutdown safety not verified', detail: UNKNOWN, tone: 'warning' };
  }
  const [title, detail, tone] = STATES[state];
  // Rust safe_to_shutdown must be affirmative; status strings alone are not
  // enough to assert the actual shutdown/recovery safety contract.
  if ((state === 'safe' || state === 'sleeping') && body.safe_to_shutdown !== true && body.safeToShutdown !== true) {
    return { title: 'Shutdown safety not verified', detail: UNKNOWN, tone: 'warning' };
  }
  // A negative authoritative flag overrides contradictory state labels.
  if (body.safe_to_shutdown === false || body.safeToShutdown === false) {
    if (state === 'safe' || state === 'sleeping') {
      return { title: 'Shutdown safety not verified', detail: UNKNOWN, tone: 'warning' };
    }
  }
  // Do not reflect arbitrary peer IDs, diagnostic errors or paths contained
  // in body.detail or body.peers into a privileged safety UI.
  return { title, detail, tone };
}

export function createNativeHostReadinessController(
  invoke: (world: string) => Promise<unknown>,
  timeoutMs = 12_000,
): NativeHostReadinessController {
  const initial = (): NativeReadinessView => ({
    phase: 'unchecked', world: '', title: 'Checking shutdown safety…', detail: UNKNOWN,
    tone: 'neutral', busy: false, verifiedReadCount: 0,
  });
  const state = writable<NativeReadinessView>(initial());
  let version = 0;
  let selectedWorld = '';
  let pending: Promise<void> | null = null;
  let verifiedReadCount = 0;

  function clearState() {
    version += 1;
    selectedWorld = '';
    pending = null;
    state.set({ ...initial(), verifiedReadCount });
  }

  return {
    state: { subscribe: state.subscribe },
    clear: clearState,
    select(world) {
      if (!/^scworld:[^\s]+$/.test(world)) {
        clearState();
        return Promise.resolve();
      }
      if (world === selectedWorld && pending) return pending;
      const epoch = ++version;
      selectedWorld = world;
      state.set({
        phase: 'checking', world,
        title: 'Checking shutdown safety…',
        detail: 'Waiting for the current Rust host-readiness report.',
        tone: 'neutral', busy: true, verifiedReadCount,
      });
      let timer: ReturnType<typeof setTimeout> | undefined;
      const work = Promise.race([
        Promise.resolve().then(() => invoke(world)),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error('Rust host readiness timed out')), timeoutMs);
        }),
      ]).then(result => {
        const mapped = parseNativeHostReadiness(result);
        if (epoch !== version || selectedWorld !== world) return;
        verifiedReadCount += 1;
        state.set({
          phase: 'ready', world, ...mapped, busy: false, verifiedReadCount,
        });
      }).catch(() => {
        if (epoch !== version || selectedWorld !== world) return;
        state.set({
          phase: 'unavailable', world, title: 'Shutdown safety unavailable',
          detail: UNKNOWN, tone: 'warning', busy: false, verifiedReadCount,
        });
      }).finally(() => {
        if (timer !== undefined) clearTimeout(timer);
      });
      const completion = work.finally(() => {
        if (pending === completion) pending = null;
      });
      pending = completion;
      return completion;
    },
  };
}
