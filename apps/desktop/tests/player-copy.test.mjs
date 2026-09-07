import test from 'node:test';
import assert from 'node:assert/strict';
import { applyPlayerCopy, DURABLE_STOP_COPY } from '../src/player-copy.js';

test('Stop-world copy describes durable save, checkpoint, and sleeping semantics', () => {
  const paragraph = { textContent: '' };
  const documentRef = {
    querySelector(selector) {
      assert.equal(selector, '#sleepDialog .dialog-card > div:first-child p');
      return paragraph;
    },
  };

  assert.equal(applyPlayerCopy(documentRef), true);
  assert.equal(paragraph.textContent, DURABLE_STOP_COPY);
  assert.match(paragraph.textContent, /save barrier/);
  assert.match(paragraph.textContent, /canonical checkpoint/);
  assert.match(paragraph.textContent, /durable sleeping state/);
  assert.doesNotMatch(paragraph.textContent, /does not itself create a durable sleeping migration state/i);
});
