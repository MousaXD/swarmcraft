import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { createDesktopSession, type DesktopSessionBackend } from '../src/lib/desktop-session';

function backend(overrides: Partial<DesktopSessionBackend> = {}): DesktopSessionBackend {
  return {
    initializeNode: vi.fn(async () => 'SwarmCraft initialized'),
    nodeIdentity: vi.fn(async () => 'Peer ID: scpeer:abc'),
    ensureDaemonRunning: vi.fn(async () => 123),
    listWorlds: vi.fn(async () => 'scworld:a Alpha snapshots=2\nscworld:b Beta snapshots=1'),
    ...overrides,
  };
}
describe('native session startup', () => {
  it('is ready only after initialization, identity, daemon and world list', async () => {
    const native = backend();
    const desktop = createDesktopSession(native);
    expect(get(desktop.state).startup.phase).toBe('starting');
    await desktop.start();
    expect(get(desktop.state)).toMatchObject({
      startup: { phase: 'ready' },
      identity: 'Peer ID: scpeer:abc',
      worldsPhase: 'loaded',
      selectedWorldId: 'scworld:a',
    });
    expect(native.ensureDaemonRunning).toHaveBeenCalledWith('');
  });
  it('marks networking failure degraded without hiding locally accessible worlds', async () => {
    const desktop = createDesktopSession(backend({
      ensureDaemonRunning: async () => { throw new Error('offline'); },
    }));
    await desktop.start();
    const state = get(desktop.state);
    expect(state.startup.phase).toBe('degraded');
    expect(state.worlds.length).toBe(2);
    expect(state.networkWarning).toMatch(/replication/);
  });
  it('does not claim ready when identity fails or world-list loading fails', async () => {
    const badIdentity = createDesktopSession(backend({ nodeIdentity: async () => { throw new Error('identity unavailable'); } }));
    await badIdentity.start();
    expect(get(badIdentity.state).startup.phase).toBe('failed');
    expect(get(badIdentity.state).worldsPhase).toBe('idle');

    const badWorlds = createDesktopSession(backend({ listWorlds: async () => { throw new Error('sidecar missing'); } }));
    await badWorlds.start();
    expect(get(badWorlds.state).startup.phase).toBe('failed');
    expect(get(badWorlds.state).worldsPhase).toBe('failed');
  });
  it('explicit retry recovers only after actual backend success', async () => {
    let fail = true;
    const desktop = createDesktopSession(backend({
      listWorlds: async () => {
        if (fail) throw new Error('filesystem unavailable');
        return 'scworld:restored Restored snapshots=5';
      },
    }));
    await desktop.start();
    expect(get(desktop.state).startup.phase).toBe('failed');
    fail = false;
    await desktop.start();
    expect(get(desktop.state).startup.phase).toBe('ready');
    expect(get(desktop.state).selectedWorldId).toBe('scworld:restored');
  });
  it('retains valid world selection and ignores stale out-of-order list refresh', async () => {
    let resolveFirst!: (value: string) => void;
    const first = new Promise<string>((resolve) => { resolveFirst = resolve; });
    let calls = 0;
    const desktop = createDesktopSession(backend({
      listWorlds: async () => (++calls === 2 ? first : 'scworld:a Alpha snapshots=2\nscworld:b Beta snapshots=1'),
    }));
    await desktop.start();
    desktop.selectWorld('scworld:b');
    const stale = desktop.refreshWorlds();
    const newer = desktop.refreshWorlds();
    await newer;
    resolveFirst('scworld:stale Stale snapshots=3');
    await stale;
    expect(get(desktop.state).selectedWorldId).toBe('scworld:b');
    expect(get(desktop.state).worlds.map(w => w.id)).not.toContain('scworld:stale');
  });
  it('never treats a missing daemon or stale refresh as authority eligibility', async () => {
    const desktop = createDesktopSession(backend({ ensureDaemonRunning: async () => null }));
    await desktop.start();
    expect(get(desktop.state).startup.phase).toBe('degraded');
    expect(Object.keys(get(desktop.state))).not.toContain('authorityEligible');
  });
});
