import assert from 'node:assert/strict';
import test from 'node:test';
import { registerTransferWizard } from '../src/transfer-wizard.js';

const ALICE = `scpeer:${'a'.repeat(64)}`;
const BOB = `scpeer:${'b'.repeat(64)}`;
const WORLD_A = `scworld:${'1'.repeat(64)}`;
const WORLD_B = `scworld:${'2'.repeat(64)}`;

function fakeElement() {
  const handlers = new Map();
  return {
    dataset: {}, value: '', textContent: '', hidden: false, disabled: false,
    open: false, children: [], attributes: {},
    addEventListener(type, fn) {
      const registered = handlers.get(type) || [];
      registered.push(fn);
      handlers.set(type, registered);
    },
    async fire(type) {
      const event = { preventDefault() {}, stopImmediatePropagation() {} };
      for (const handler of handlers.get(type) || []) await handler(event);
    },
    setAttribute(key, value) { this.attributes[key] = value; },
    removeAttribute(key) { delete this.attributes[key]; },
    querySelector() { return null; },
    replaceChildren() { this.children = []; this.value = ''; },
    append(child) {
      this.children.push(child);
      if (!this.value && child.value) this.value = child.value;
    },
    showModal() { this.open = true; },
    close() { this.open = false; return this.fire('close'); },
  };
}

function fakeEnvironment(backend) {
  const names = [
    'world', 'transferHost', 'closeTransferDialog', 'doneTransferDialog',
    'transferWizardError', 'transferDialogTitle', 'transferRoleNote',
    'transferSourceFlow', 'transferTargetFlow', 'transferTargetPeer',
    'transferTargetHint', 'prepareTransfer', 'commitTransfer', 'observeTransfer',
    'acceptTransfer', 'activateTransfer', 'sourceTransferComplete',
    'preparedTransferToken', 'acceptedTransferToken', 'committedTransferToken',
    'successorEpochToken', 'incomingPreparedTransfer', 'outgoingAcceptedTransfer',
    'incomingCommittedTransfer', 'outgoingEpochTransfer',
    'preparedTransferWrap', 'committedTransferWrap', 'acceptedTransferWrap',
    'epochTransferWrap', 'copyPreparedTransfer', 'copyCommittedTransfer',
    'copyAcceptedTransfer', 'copyEpochTransfer',
  ];
  const elements = new Map(names.map(name => [name, fakeElement()]));
  const beforeDocument = globalThis.document;
  const beforeWindow = globalThis.window;
  const beforeSetInterval = globalThis.setInterval;
  const beforeClearInterval = globalThis.clearInterval;
  const intervals = new Map();
  let nextTimer = 1;
  const body = { append(element) { elements.set(element.id, element); } };
  globalThis.document = {
    readyState: 'complete', body,
    getElementById(name) { return elements.get(name) || null; },
    createElement() { return fakeElement(); },
  };
  globalThis.window = { __swarmcraftInvalidateTransientSafety() {} };
  globalThis.setInterval = (fn) => {
    const id = nextTimer++;
    intervals.set(id, fn);
    return id;
  };
  globalThis.clearInterval = (id) => intervals.delete(id);
  registerTransferWizard(backend);
  return {
    elements,
    async open(world) {
      elements.get('world').value = world;
      await elements.get('transferHost').fire('click');
      return elements.get('transferDialog');
    },
    async step(id) { await elements.get(id).fire('click'); },
    async poll() {
      const fn = [...intervals.values()].at(-1);
      assert.ok(fn, 'expected one active observation timer');
      await fn();
    },
    restore() {
      globalThis.document = beforeDocument;
      globalThis.window = beforeWindow;
      globalThis.setInterval = beforeSetInterval;
      globalThis.clearInterval = beforeClearInterval;
    },
  };
}

function signedBackend({ prepare, getStatus } = {}) {
  return {
    nodeIdentity: async () => `Peer ID: ${ALICE}`,
    worldStatus: async (world) => getStatus?.(world) || `Authority: ${ALICE}\nEpoch: 4`,
    worldPeers: async () => `${ALICE} authority_eligible=true banned=false\n${BOB} authority_eligible=true banned=false`,
    hostReadiness: async () => ({ safeToShutdown: false }),
    migration: {
      capabilities: { transfer: true },
      refreshCapabilities: async () => {},
      transferPrepare: prepare || (async () => 'signed-prepared'),
      transferObserve: async () => 'signed-observed',
    },
  };
}

test('a slow signed token from an old dialog cannot appear in another world', async () => {
  let finishOld;
  const calls = [];
  const backend = signedBackend({
    prepare: (world) => {
      calls.push(world);
      return new Promise((resolve) => { finishOld = resolve; });
    },
  });
  const env = fakeEnvironment(backend);
  try {
    const oldDialog = await env.open(WORLD_A);
    const oldStep = env.step('prepareTransfer');
    await Promise.resolve();
    assert.deepEqual(calls, [WORLD_A]);
    await oldDialog.close();
    await env.open(WORLD_B);
    finishOld('OLD-WORLD-SIGNED-TOKEN');
    await oldStep;
    assert.equal(env.elements.get('preparedTransferToken').value, '');
    assert.equal(env.elements.get('preparedTransferWrap').hidden, true);
    assert.equal(env.elements.get('transferDialog').dataset.world, WORLD_B);
    assert.equal(env.elements.get('transferWizardError').textContent, '');
  } finally {
    env.restore();
  }
});

test('poll cannot call an old host or same epoch a successful successor', async () => {
  let current = `Authority: ${ALICE}\nEpoch: 4`;
  const env = fakeEnvironment(signedBackend({ getStatus: () => current }));
  try {
    await env.open(WORLD_A);
    await env.step('prepareTransfer');
    env.elements.get('successorEpochToken').value = 'signed-successor-epoch';
    await env.step('observeTransfer');
    const detail = env.elements.get('sourceTransferComplete');
    assert.match(detail.textContent, /quorum propagation/i);
    await env.poll();
    assert.doesNotMatch(detail.textContent, /selected successor at epoch/i);
    current = `Authority: ${BOB}\nEpoch: 4`;
    await env.poll();
    assert.doesNotMatch(detail.textContent, /selected successor at epoch/i);
    current = `Authority: ${BOB}\nEpoch: 5`;
    await env.poll();
    assert.match(detail.textContent, /selected successor at epoch 5/);
    assert.match(detail.textContent, /independent verification/);
    assert.notEqual(detail.dataset.tone, 'safe');
  } finally {
    env.restore();
  }
});
