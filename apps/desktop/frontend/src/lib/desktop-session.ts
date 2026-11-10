import { get, writable, type Readable } from 'svelte/store';
import { parseWorldList } from './world-list';
import { worldModel, type WorldModel } from './world-state';
import { initialStartupState, transitionStartup, type StartupState } from './startup-state';

/**
 * The Svelte replacement's local session coordinator.
 *
 * This module ONLY owns presentation state. Backend commands still perform
 * identity, membership, consensus, hosting, safety, and recovery decisions.
 * It is intentionally not attached to the legacy controller during migration:
 * two startup coordinators would double-execute native commands.
 */
export interface DesktopSessionBackend {
  initializeNode(): Promise<string>;
  nodeIdentity(): Promise<string>;
  ensureDaemonRunning(listen: string): Promise<unknown>;
  listWorlds(): Promise<string>;
  worldStatus(world: string): Promise<string>;
  worldCompatibility(world: string): Promise<string>;
}

export type WorldsPhase = 'idle' | 'loading' | 'loaded' | 'failed';

export interface DesktopSessionState {
  startup: StartupState;
  identity: string | null;
  networkWarning: string | null;
  worldsPhase: WorldsPhase;
  worlds: WorldModel[];
  worldsError: string | null;
  selectedWorldId: string | null;
}

const initialState = (): DesktopSessionState => ({
  startup: initialStartupState,
  identity: null,
  networkWarning: null,
  worldsPhase: 'idle',
  worlds: [],
  worldsError: null,
  selectedWorldId: null,
});

function detail(error: unknown): string {
  // Do not echo raw native error messages that may include signed
  // invitations, command arguments, bearer tokens, or private data.
  const raw = String(error instanceof Error ? error.message : error);
  if (/authorization|bearer|private.?key|password|ipc.?token|invite.?token|secret/i.test(raw)) {
    return 'A native operation failed. Check the sanitized diagnostics and retry.';
  }
  return raw.slice(0, 320);
}

async function within<T>(label: string, maxMs: number, operation: () => Promise<T>): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation(),
      new Promise<T>((_, reject) => {
        timeout = setTimeout(() => reject(new Error(label + ' timed out. Retry or restart SwarmCraft.')), maxMs);
      }),
    ]);
  } finally {
    if (timeout) clearTimeout(timeout);
  }
}

export interface DesktopSession {
  readonly state: Readable<DesktopSessionState>;
  start(): Promise<void>;
  refreshWorlds(): Promise<void>;
  selectWorld(id: string): void;
}

/** Keeps stale native completions from claiming a new session is Ready. */
export function createDesktopSession(backend: DesktopSessionBackend, listen = ''): DesktopSession {
  const mutable = writable<DesktopSessionState>(initialState());
  let generation = 0;
  let worldsRequest = 0;
  let starting: Promise<void> | null = null;
  const current = (g: number) => g === generation;
  const change = (fn: (state: DesktopSessionState) => DesktopSessionState) => mutable.update(fn);

  async function refreshWorldsFor(g: number): Promise<void> {
    const request = ++worldsRequest;
    if (!current(g)) return;
    change(s => ({ ...s, worldsPhase: 'loading', worldsError: null }));
    try {
      const raw = await within('World list', 45000, () => backend.listWorlds());
      if (!current(g) || request !== worldsRequest) return;
      const summaries = parseWorldList(raw);
      // Status and compatibility are always requested from Rust. Parallelism is
      // bounded to avoid stampeding the local sidecar on large world lists.
      const worlds: WorldModel[] = [];
      for (let i = 0; i < summaries.length; i += 4) {
        const batch = await Promise.all(summaries.slice(i, i + 4).map(async (summary) => {
          const results = await Promise.allSettled([
            within('World status', 15000, () => backend.worldStatus(summary.id)),
            within('World compatibility', 15000, () => backend.worldCompatibility(summary.id)),
          ]);
          return worldModel(summary, results as [PromiseSettledResult<string>, PromiseSettledResult<string>]);
        }));
        if (!current(g) || request !== worldsRequest) return;
        worlds.push(...batch);
      }
      change(s => ({
        ...s,
        worldsPhase: 'loaded',
        worlds,
        worldsError: null,
        selectedWorldId: worlds.some(w => w.id === s.selectedWorldId)
          ? s.selectedWorldId
          : (worlds[0]?.id ?? null),
      }));
    } catch (error) {
      if (!current(g) || request !== worldsRequest) return;
      change(s => ({
        ...s,
        worldsPhase: 'failed',
        worldsError: detail(error),
        // Keep previously displayed worlds as stale context; never claim a
        // successful refresh or silently reclassify them as canonical.
      }));
      throw error;
    }
  }

  async function doStartup(g: number): Promise<void> {
    try {
      const initialized = await within('Device setup', 30000, () => backend.initializeNode());
      if (!current(g)) return;
      if (!String(initialized).trim()) throw new Error('Device initialization returned an empty result.');
      const daemon = await within('Networking service', 10000, () => backend.ensureDaemonRunning(listen))
        .then(pid => pid !== null && pid !== undefined && String(pid).trim() !== '')
        .catch(() => false);
      if (!current(g)) return;
      if (!daemon) {
        change(s => ({
          ...s,
          networkWarning: 'Networking is unavailable. Local worlds remain usable, but replication and automatic migration may be unavailable.',
        }));
      }
      const identity = await within('Device identity', 15000, () => backend.nodeIdentity());
      if (!current(g)) return;
      if (!String(identity).trim()) throw new Error('Device identity could not be verified.');
      change(s => ({ ...s, identity }));
      await refreshWorldsFor(g);
      if (!current(g)) return;
      if (get(mutable).worldsPhase !== 'loaded') {
        throw new Error('The local world list could not be read.');
      }
      change(s => ({
        ...s,
        startup: transitionStartup(
          s.startup,
          daemon
            ? { type: 'ready' }
            : { type: 'network-degraded', detail: 'Networking needs attention' },
        ),
      }));
    } catch (error) {
      if (!current(g)) return;
      change(s => ({
        ...s,
        startup: transitionStartup(s.startup, { type: 'fail', detail: detail(error) }),
      }));
    }
  }

  return {
    state: { subscribe: mutable.subscribe },
    start() {
      if (starting) return starting;
      const g = ++generation;
      ++worldsRequest;
      mutable.set(initialState());
      // Starting must be cleared after the native work settles, including
      // failure, so the explicit Retry action can initiate a fresh attempt.
      const run = doStartup(g);
      const completion = run.finally(() => {
        if (starting === completion) starting = null;
      });
      starting = completion;
      return completion;
    },
    refreshWorlds() {
      return refreshWorldsFor(generation);
    },
    selectWorld(id) {
      change(s => s.worlds.some(w => w.id === id) ? { ...s, selectedWorldId: id } : s);
    },
  };
}
