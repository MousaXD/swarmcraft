import { writable, type Readable } from 'svelte/store';

/**
 * The Svelte candidate reads only registered Tauri/Rust commands. World lists,
 * status, and compatibility are native observations, never authority claims.
 */
export interface NativeWorldEntry {
  id: string;
  name: string;
  snapshots: number;
  status: Record<string, string>;
  compatibility: Record<string, string>;
}
export interface WorldListState {
  phase: 'loading' | 'ready' | 'unavailable';
  worlds: NativeWorldEntry[];
  busy: boolean;
  error: string;
  verifiedReadCount: number;
}
export interface NativeWorldListBackend {
  listWorlds(): Promise<string>;
  worldStatus(world: string): Promise<string>;
  worldCompatibility(world: string): Promise<string>;
}
export interface WorldListController {
  state: Readable<WorldListState>;
  refresh(): Promise<NativeWorldEntry[]>;
}

const LIMIT = 256;
const DEFAULT_READ_TIMEOUT_MS = 18_000;

function parseFields(value: unknown): Record<string, string> {
  const fields: Record<string, string> = Object.create(null);
  for (const line of String(value ?? '').split(/\r?\n/)) {
    const position = line.indexOf(':');
    if (position <= 0) continue;
    const key = line.slice(0, position).trim();
    if (!key || key.length > 80 || key === '__proto__' || key === 'constructor' || key === 'prototype') continue;
    fields[key] = line.slice(position + 1).trim().slice(0, 1024);
  }
  return fields;
}

export function parseNativeWorldList(raw: unknown): NativeWorldEntry[] {
  if (typeof raw !== 'string') throw new Error('Native world-list response was not text.');
  const entries: NativeWorldEntry[] = [];
  const seen = new Set<string>();
  for (const rawLine of raw.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line.startsWith('scworld:')) continue;
    const match = line.match(/^(scworld:[^\s]+)\s+(.+?)\s+snapshots=(\d+)$/);
    if (!match) throw new Error('Native world-list record was malformed.');
    const [, id, name, count] = match;
    const snapshots = Number(count);
    if (!Number.isSafeInteger(snapshots) || seen.has(id) || name.length > 512) {
      throw new Error('Native world-list record failed validation.');
    }
    seen.add(id);
    entries.push({ id, name, snapshots, status: Object.create(null), compatibility: Object.create(null) });
    if (entries.length > LIMIT) throw new Error('Native world-list result exceeds the bounded UI limit.');
  }
  // An empty native list is legitimate. Never synthesize fixture worlds.
  return entries;
}

function bounded<T>(read: () => Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([
    Promise.resolve().then(read),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('Native world-list read exceeded its timeout.')), timeoutMs);
    }),
  ]).finally(() => {
    if (timer !== undefined) clearTimeout(timer);
  });
}

export function createNativeWorldListController(
  backend: NativeWorldListBackend,
  timeoutMs = DEFAULT_READ_TIMEOUT_MS,
): WorldListController {
  const state = writable<WorldListState>({
    phase: 'loading', worlds: [], busy: true, error: '', verifiedReadCount: 0,
  });
  let request: Promise<NativeWorldEntry[]> | null = null;
  let verifiedReadCount = 0;
  let generation = 0;

  return {
    state: { subscribe: state.subscribe },
    refresh() {
      if (request) return request;
      const version = ++generation;
      state.update(s => ({ ...s, busy: true, error: '' }));
      const work = async (): Promise<NativeWorldEntry[]> => {
        const entries = parseNativeWorldList(await bounded(() => backend.listWorlds(), timeoutMs));
        // Do not serially block startup on every status read. Either Rust
        // source can independently be unavailable for one offline replica.
        // Limit spawned Rust/CLI reads to four worlds at once. Unbounded
        // Promise.all over imported libraries could open hundreds of sidecars.
        const detailed: NativeWorldEntry[] = new Array(entries.length);
        let nextIndex = 0;
        await Promise.all(Array.from({ length: Math.min(4, entries.length) }, async () => {
          for (;;) {
            if (version !== generation) return;
            const index = nextIndex++;
            if (index >= entries.length) return;
            const entry = entries[index];
            const [status, compatibility] = await Promise.allSettled([
              bounded(() => backend.worldStatus(entry.id), timeoutMs),
              bounded(() => backend.worldCompatibility(entry.id), timeoutMs),
            ]);
            detailed[index] = {
              ...entry,
              status: status.status === 'fulfilled' ? parseFields(status.value) : Object.create(null),
              compatibility: compatibility.status === 'fulfilled' ? parseFields(compatibility.value) : Object.create(null),
            };
          }
        }));
        if (version === generation) {
          verifiedReadCount += 1;
          state.set({ phase: 'ready', worlds: detailed, busy: false, error: '', verifiedReadCount });
        }
        return detailed;
      };
      // Bound the entire refresh too: 256 local worlds could otherwise
      // queue bounded per-item metadata reads for much longer than startup.
      // A late Rust callback must never turn a timed-out list into Ready.
      const task = bounded(work, timeoutMs * 2).catch(() => {
        ++generation;
        const error = 'Could not read local worlds. Check the SwarmCraft installation and try Refresh.';
        // Sidecar errors may include local paths or private data: never expose them.
        state.set({ phase: 'unavailable', worlds: [], busy: false, error, verifiedReadCount });
        throw new Error(error);
      });
      const settled = task.finally(() => {
        if (request === settled) request = null;
      });
      request = settled;
      return settled;
    },
  };
}

export function worldSafetyTone(value: unknown): 'safe' | 'warning' | 'danger' | 'neutral' {
  const safety = String(value ?? '').toLowerCase();
  if (safety.includes('conflict')) return 'danger';
  if (safety.includes('solo') || safety.includes('degraded')) return 'warning';
  if (safety.includes('canonical') || safety.includes('quorum')) return 'safe';
  return 'neutral';
}
