import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { createIdentityController } from '../src/lib/device-identity';

describe('Svelte native identity presentation', () => {
  it('accepts genuine Rust identity text and never invents initialization', async () => {
    const invoke = vi.fn(async () => 'Peer ID: scpeer:verified');
    const identity = createIdentityController(invoke);
    expect(get(identity.state).phase).toBe('checking');
    expect(get(identity.state).nativeVerifiedRefreshCount).toBe(0);
    await identity.refresh();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(get(identity.state)).toMatchObject({
      phase: 'ready', text: 'Peer ID: scpeer:verified', busy: false, error: '', nativeVerifiedRefreshCount: 1,
    });
  });

  it('reports unavailable and redacts native error details on failure or empty data', async () => {
    const identity = createIdentityController(async () => {
      throw new Error('secret=private signing material /path/to/private/identity');
    });
    await identity.refresh();
    const value = get(identity.state);
    expect(value.phase).toBe('unavailable');
    expect(value.nativeVerifiedRefreshCount).toBe(0);
    expect(value.error).toContain('Native identity lookup failed');
    expect(JSON.stringify(value)).not.toMatch(/secret|signing material|private\/identity/i);

    const empty = createIdentityController(async () => '');
    await empty.refresh();
    expect(get(empty.state).phase).toBe('unavailable');
    expect(get(empty.state).text).toBe('Device identity is unavailable.');
    expect(get(empty.state).nativeVerifiedRefreshCount).toBe(0);
  });

  it('coalesces duplicate refresh requests and keeps legacy startup updates current', async () => {
    let resolve!: (value: string) => void;
    const invoke = vi.fn(() => new Promise<string>(r => { resolve = r; }));
    const identity = createIdentityController(invoke);
    const first = identity.refresh();
    const second = identity.refresh();
    await Promise.resolve(); // allow native invocation to begin
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(get(identity.state).busy).toBe(true);
    expect(first).toBe(second);
    expect(identity.publishLegacy('Peer ID: scpeer:startup')).toBe(true);
    resolve('Peer ID: scpeer:stale');
    await first;
    expect(get(identity.state)).toMatchObject({
      phase: 'ready', text: 'Peer ID: scpeer:startup', busy: false, nativeVerifiedRefreshCount: 0,
    });
  });

  it('never represents initialize_node setup text as a verified identity', async () => {
    const identity = createIdentityController(async () => 'Peer ID: scpeer:real');
    identity.publishLegacy('Initialized SwarmCraft device', 'setup');
    expect(get(identity.state).phase).toBe('checking');
    expect(get(identity.state).text).toContain('Initialized');
    identity.publishLegacy('private identity read failure', 'unavailable');
    expect(get(identity.state).phase).toBe('unavailable');
    expect(get(identity.state).nativeVerifiedRefreshCount).toBe(0);
    expect(JSON.stringify(get(identity.state))).not.toContain('private identity read failure');
    await identity.refresh();
    expect(get(identity.state)).toMatchObject({ phase: 'ready', text: 'Peer ID: scpeer:real', nativeVerifiedRefreshCount: 1 });
  });

  it('can refresh after a failed request without leaving an endless spinner', async () => {
    let fail = true;
    const identity = createIdentityController(async () => {
      if (fail) throw Error('offline');
      return 'Peer ID: scpeer:recovered';
    });
    await identity.refresh();
    expect(get(identity.state).busy).toBe(false);
    expect(get(identity.state).nativeVerifiedRefreshCount).toBe(0);
    fail = false;
    await identity.refresh();
    expect(get(identity.state).phase).toBe('ready');
    expect(get(identity.state).nativeVerifiedRefreshCount).toBe(1);
  });
});
