import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import App from '../src/App.svelte';
import WorldsView from '../src/components/WorldsView.svelte';
import Dialogs from '../src/components/Dialogs.svelte';

describe('Svelte component rendering and accessibility', () => {
  it('renders the whole desktop shell and six real task views', () => {
    const html = render(App).body;
    for (const id of ['worldsView', 'createPanel', 'importPanel', 'joinPanel', 'activityView', 'diagnosticsView']) {
      expect(html).toContain('id="' + id + '"');
    }
    expect(html).toContain('aria-label="Primary navigation"');
    expect(html).toContain('id="nodeBadge"');
    expect(html).toContain('id="status"');
  });
  it('renders honest initial starting/unknown/empty states, not false readiness', () => {
    const html = render(WorldsView).body;
    expect(html).toContain('id="worldCount"');
    expect(html).toContain('Loading');
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain('id="worldLoadError"');
  });
  it('retains native modal dialogs with consequence-aware confirmations', () => {
    const html = render(Dialogs).body;
    for (const id of ['inviteDialog', 'sleepDialog', 'leaveDialog', 'confirmSleep', 'confirmLeave']) {
      expect(html).toContain('id="' + id + '"');
    }
    expect(html).toContain('signed invite');
    expect(html).toContain('Stop this world?');
    expect(html).toContain('Leave this world?');
  });
  it('makes activity output keyboard-readable and explicitly read-only', () => {
    const html = render(App).body;
    expect(html).toContain('id="output"');
    expect(html).toContain('role="textbox"');
    expect(html).toContain('aria-readonly="true"');
    expect(html).toContain('aria-multiline="true"');
  });
});
