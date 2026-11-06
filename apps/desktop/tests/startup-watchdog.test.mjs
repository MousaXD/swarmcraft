import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const bootstrap = readFileSync(new URL('../src/bootstrap.js', import.meta.url), 'utf8');

// Exercise the real early-loading watchdog, not a fake Tauri invoke or backend.
function surface() {
  const scheduled = new Map();
  const listeners = new Map();
  let timerId = 0;
  let banner = null;
  const element = (id, text = '') => ({
    id, textContent: text, hidden: false, className: '', dataset: {},
    children: [], attrs: {},
    setAttribute(name, value) { this.attrs[name] = value; },
    getAttribute(name) { return this.attrs[name]; },
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    remove() { if (this === banner) banner = null; },
    addEventListener(name, fn) { this.listeners ||= {}; this.listeners[name] = fn; },
  });
  const items = new Map([
    ['status', element('status', 'Starting…')],
    ['statusDot', element('statusDot')],
    ['nodeBadge', element('nodeBadge', 'Checking…')],
    ['worldCount', element('worldCount', 'Loading…')],
    ['worldCards', element('worldCards')],
  ]);
  items.get('worldCards').setAttribute('aria-busy', 'true');
  const document = {
    documentElement: { dataset: {} },
    body: { prepend(value) { banner = value; } },
    getElementById(id) { return id === 'startupFailure' ? banner : items.get(id) || null; },
    createElement(id) { return element(id); },
    createTextNode(text) { return { textContent: text }; },
    querySelector(selector) {
      return selector === '.topbar' ? { insertAdjacentElement(_, value) { banner = value; } } : null;
    },
    addEventListener(name, fn) { listeners.set(name, fn); },
  };
  const window = {
    addEventListener(name, fn) { listeners.set(name, fn); },
    location: { reload() { throw Error('Unexpected full reload'); } },
  };
  vm.runInNewContext(bootstrap, {
    window, document,
    setTimeout(callback) { const id = ++timerId; scheduled.set(id, callback); return id; },
    clearTimeout(id) { scheduled.delete(id); },
  }, { filename: 'bootstrap.js' });
  return { window, document, items, listeners, scheduled, get banner() { return banner; } };
}

test('startup never starts Ready and only reaches ready after explicit completion', () => {
  const ui = surface();
  assert.equal(ui.document.documentElement.dataset.swarmcraftStartup, 'loading');
  ui.window.__swarmcraftStartup.begin();
  assert.equal(ui.document.documentElement.dataset.swarmcraftStartup, 'starting');
  ui.window.__swarmcraftStartup.step('World list');
  ui.window.__swarmcraftStartup.ready();
  assert.equal(ui.document.documentElement.dataset.swarmcraftStartup, 'ready');
  assert.equal(ui.window.__swarmcraftStartup.phase, 'Ready');
  assert.equal(ui.scheduled.size, 0);
});

test('JavaScript error before module initialization removes permanent skeletons and offers retry', () => {
  const ui = surface();
  ui.listeners.get('DOMContentLoaded')();
  ui.listeners.get('error')({ message: 'module did not load' });
  assert.equal(ui.document.documentElement.dataset.swarmcraftStartup, 'failed');
  assert.equal(ui.items.get('status').textContent, 'Startup failed');
  assert.equal(ui.items.get('nodeBadge').textContent, 'Setup needs attention');
  assert.equal(ui.items.get('worldCount').textContent, 'Worlds unavailable');
  assert.equal(ui.items.get('worldCards').getAttribute('aria-busy'), 'false');
  assert.equal(ui.banner.hidden, false);
  let retried = 0;
  ui.window.swarmcraftRetryStartup = () => retried++;
  ui.banner.children.find(item => item.id === 'retryStartup').listeners.click();
  assert.equal(retried, 1);
});

test('watchdog deadline produces visible failure, not false readiness', () => {
  const ui = surface();
  ui.listeners.get('DOMContentLoaded')();
  assert.equal(ui.scheduled.size, 1);
  [...ui.scheduled.values()][0]();
  assert.equal(ui.document.documentElement.dataset.swarmcraftStartup, 'failed');
  assert.equal(ui.items.get('status').textContent, 'Startup failed');
  ui.window.__swarmcraftStartup.ready();
  assert.equal(ui.document.documentElement.dataset.swarmcraftStartup, 'failed');
});

test('backend failures redact token-looking errors on screen', () => {
  const ui = surface();
  ui.window.__swarmcraftStartup.begin();
  ui.window.__swarmcraftStartup.fail('backend', new Error('authorization: Bearer private-key-secret'));
  const display = ui.banner.children.map(c => c.textContent || '').join(' ');
  assert.doesNotMatch(display, /private-key-secret/);
  assert.match(display, /sanitized native startup diagnostics/);
});
