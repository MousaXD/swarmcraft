import assert from 'node:assert/strict';
import test from 'node:test';
import { normalizeTransferProgress, registerTransferWizard } from '../src/transfer-wizard.js';

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
    'automaticTransferProgress', 'automaticTransferActions', 'startAutomaticTransfer',
    'manualSourceTransfer', 'manualTargetTransfer',
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

function progress(world, phase, currentAuthority = ALICE, currentEpoch = 4) {
  return {
    world_id: world, phase, current_authority: currentAuthority, current_epoch: currentEpoch,
    source_peer: phase === 'idle' ? null : ALICE,
    target_peer: phase === 'idle' ? null : BOB,
    source_epoch: phase === 'idle' ? null : 4,
    checkpoint_hash: ['idle', 'checkpoint_requested'].includes(phase) ? null : 'a'.repeat(64),
  };
}

test('one-click handoff submits the exact epoch and resumes from backend progress after reopening', async () => {
  let state = progress(WORLD_A, 'idle');
  const calls = [];
  const backend = signedBackend();
  backend.migration.capabilities.automaticTransfer = true;
  backend.migration.transferProgress = async () => state;
  backend.migration.startAutomaticTransfer = async (world, target, epoch) => {
    calls.push([world, target, epoch]);
    state = progress(world, 'checkpoint_requested');
    return state;
  };
  const env = fakeEnvironment(backend);
  try {
    const dialog = await env.open(WORLD_A);
    const button = env.elements.get('startAutomaticTransfer');
    assert.equal(button.disabled, false);
    assert.equal(env.elements.get('manualSourceTransfer').open, false);
    await env.step('startAutomaticTransfer');
    assert.deepEqual(calls, [[WORLD_A, BOB, 4]]);
    assert.equal(button.disabled, true);
    await env.step('startAutomaticTransfer');
    assert.equal(calls.length, 1);
    await dialog.close();
    state = progress(WORLD_A, 'committed');
    await env.open(WORLD_A);
    assert.match(env.elements.get('automaticTransferProgress').textContent, /committed/i);
    assert.equal(button.disabled, true);
    state = progress(WORLD_A, 'successor_observed', BOB, 5);
    await env.poll();
    assert.match(env.elements.get('automaticTransferProgress').textContent, /does not prove live quorum/i);
    assert.equal(env.elements.get('automaticTransferProgress').dataset.tone, 'neutral');
    assert.equal(button.disabled, true);
  } finally {
    env.restore();
  }
});

test('an in-flight idle poll cannot re-enable a submitted automatic handoff', async () => {
  let state = progress(WORLD_A, 'idle');
  let resolveOldPoll;
  let deferred = false;
  const backend = signedBackend();
  backend.migration.capabilities.automaticTransfer = true;
  backend.migration.transferProgress = async () => deferred
    ? new Promise(resolve => { resolveOldPoll = resolve; })
    : state;
  backend.migration.startAutomaticTransfer = async () => {
    deferred = false;
    state = progress(WORLD_A, 'prepared');
    return state;
  };
  const env = fakeEnvironment(backend);
  try {
    await env.open(WORLD_A);
    deferred = true;
    const oldPoll = env.poll();
    await Promise.resolve();
    await env.step('startAutomaticTransfer');
    resolveOldPoll(progress(WORLD_A, 'idle'));
    await oldPoll;
    assert.equal(env.elements.get('startAutomaticTransfer').disabled, true);
    assert.match(env.elements.get('automaticTransferProgress').textContent, /Final checkpoint recorded/);
  } finally {
    env.restore();
  }
});

test('durable transfer display rejects wrong-world, inconsistent generation and malformed checkpoint reports', () => {
  assert.throws(() => normalizeTransferProgress(progress(WORLD_B, 'prepared'), WORLD_A), /another world/);
  assert.throws(() => normalizeTransferProgress(progress(WORLD_A, 'committed', BOB, 5), WORLD_A), /source generation/);
  assert.throws(() => normalizeTransferProgress(progress(WORLD_A, 'successor_observed', BOB, 4), WORLD_A), /successor/);
  assert.throws(() => normalizeTransferProgress({ ...progress(WORLD_A, 'prepared'), checkpoint_hash: 'invalid' }, WORLD_A), /checkpoint/);
});
