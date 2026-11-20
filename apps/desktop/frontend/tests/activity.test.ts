import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { activityText, appendActivity, clearActivity, activityBridge } from '../src/lib/activity';

describe('reactive Activity log with legacy-compatible formatting', () => {
  it('replaces the initial status with the first real entry', () => {
    activityText.set('SwarmCraft is starting.');
    expect(appendActivity('12:00  Device\nReady')).toBe(true);
    expect(get(activityText)).toBe('12:00  Device\nReady');
  });
  it('appends multiple results in order and preserves each diagnostic entry', () => {
    activityText.set('SwarmCraft is starting.');
    appendActivity('12:00  Setup\nComplete');
    appendActivity('12:01  World status\ncanonical');
    expect(get(activityText)).toBe('12:00  Setup\nComplete\n\n12:01  World status\ncanonical');
  });
  it('clear is reactive, and a fresh event replaces the clearing marker', () => {
    activityBridge.clear();
    expect(get(activityText)).toBe('Activity cleared.');
    expect(activityBridge.append('12:02  Retry\nSucceeded')).toBe(true);
    expect(get(activityText)).toBe('12:02  Retry\nSucceeded');
  });
  it('ignores blank messages without erasing existing history', () => {
    activityText.set('Important entry');
    expect(appendActivity('   ')).toBe(false);
    expect(get(activityText)).toBe('Important entry');
  });
});
