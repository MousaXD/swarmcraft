import { describe, expect, it, vi } from 'vitest';
import { createNativeBridge } from '../src/lib/native-bridge';
import { initialStartupState, transitionStartup } from '../src/lib/startup-state';
import { parseWorldList } from '../src/lib/world-list';

describe('startup presentation contract', () => {
  it('never calls a failed startup ready without explicit retry', () => {
    const failed = transitionStartup(initialStartupState, { type: 'fail', detail: 'Missing CLI sidecar' });
    expect(transitionStartup(failed, { type: 'ready' }).phase).toBe('failed');
    expect(transitionStartup(failed, { type: 'network-degraded', detail: 'Offline' }).phase).toBe('failed');
    expect(transitionStartup(failed, { type: 'begin' }).phase).toBe('starting');
  });
  it('distinguishes ready and degraded', () => {
    expect(transitionStartup(initialStartupState, { type: 'ready' }).phase).toBe('ready');
    expect(transitionStartup(initialStartupState, { type: 'network-degraded', detail: 'Offline' }).phase).toBe('degraded');
  });
});

describe('native boundary', () => {
  it('fails closed when native IPC is missing', async () => {
    await expect(createNativeBridge(undefined).invoke('list_worlds', {})).rejects.toThrow('Tauri is unavailable');
  });
  it('passes exact command and args through without rewriting invoke', async () => {
    const invoke = vi.fn(async (_command: string, _args?: Record<string, unknown>) => 'scworld:abc Demo snapshots=2');
    const client = createNativeBridge(invoke);
    expect(await client.invoke('list_worlds', {})).toContain('Demo');
    expect(invoke).toHaveBeenCalledExactlyOnceWith('list_worlds', {});
  });
});

describe('world list contract', () => {
  it('handles empty and malformed lines, preserving actual canonical IDs', () => {
    expect(parseWorldList('')).toEqual([]);
    expect(parseWorldList('noise\nscworld:abc My World snapshots=3\nscworld:broken no-number')).toEqual([
      { id: 'scworld:abc', name: 'My World', snapshots: 3 },
    ]);
  });
});
