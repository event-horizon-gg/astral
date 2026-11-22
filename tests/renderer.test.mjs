import test from 'node:test';
import assert from 'node:assert/strict';
import { createApplicationClient } from '../src/application/client.js';
import { createRequestScope } from '../src/application/requests.js';
import { navigateState, selectedAddon } from '../src/application/navigation.js';
import { readProfileNames, saveProfileNames } from '../src/features/account/profile-cache.js';
import { registerDialog } from '../src/components/dialog-behavior.js';
import { JSDOM } from 'jsdom';

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const state = (root) => ({ root, accounts: [], addons: [], flavor: 'retail' });

test('renderer commands use their declared result rather than metadata shape', async () => {
  const values = [], metadata = state('metadata-that-looks-like-state');
  const client = createApplicationClient({ setupExportInfo: async () => metadata, check: async () => state('game') }, { state: (value) => values.push(value) });
  assert.equal(await client.execute('setupExportInfo'), metadata);
  assert.deepEqual(values, []);
  await client.execute('check');
  assert.deepEqual(values, [state('game')]);
});

test('sync wrappers publish only their actual state and preserve the decision context', async () => {
  const values = [], issues = [], result = { incomplete: true, state: state('game'), scope: 'retail', revision: 7 };
  const client = createApplicationClient({ sync: async () => result }, { state: (value) => values.push(value), syncIssue: (...args) => issues.push(args) });
  await client.execute('sync', 'auto');
  assert.deepEqual(values, [result.state]);
  assert.deepEqual(issues, [[result, ['auto']]]);
});

test('an old state refresh cannot replace the result of a completed action', async () => {
  const old = deferred(), values = [];
  const client = createApplicationClient({ state: () => old.promise, client: async () => state('new-installation') }, { state: (value) => values.push(value) });
  const refresh = client.refresh();
  await client.execute('client', 'forever');
  old.resolve(state('old-installation'));
  assert.equal(await refresh, undefined);
  assert.deepEqual(values, [state('new-installation')]);
});

test('failed reads retain the last good state and normalize IPC errors once', async () => {
  const values = [], errors = [];
  const client = createApplicationClient({ state: async () => { throw Error("Error invoking remote method 'astral:state': Error: Folder offline"); } }, { state: (value) => values.push(value), error: (message) => errors.push(message) });
  await assert.rejects(client.refresh(), /Folder offline/);
  assert.deepEqual(values, []);
  await client.execute('state');
  assert.deepEqual(errors, ['Folder offline']);
  assert.deepEqual(values, []);
});

test('busy remains active until concurrent commands have both finished', async () => {
  const first = deferred(), second = deferred(), busy = [];
  const client = createApplicationClient({ snapshot: () => first.promise, setupExportInfo: () => second.promise }, { busy: (value) => busy.push(value) });
  const a = client.execute('snapshot'), b = client.execute('setupExportInfo');
  first.resolve({ files: 1 }); await a;
  assert.equal(busy.at(-1), true);
  second.resolve({ addonNames: [] }); await b;
  assert.equal(busy.at(-1), false);
});

test('scoped requests discard older refreshes and late failures after navigation', async () => {
  const scope = createRequestScope(), old = deferred(), fresh = deferred(), values = [];
  const a = scope.run(() => old.promise, (value) => values.push(value));
  const b = scope.run(() => fresh.promise, (value) => values.push(value));
  fresh.resolve('current'); await b;
  old.resolve('earlier'); await a;
  assert.deepEqual(values, ['current']);
  const late = deferred(), pending = scope.run(() => late.promise, (value) => values.push(value));
  scope.invalidate(); late.reject(Error('Old account error')); await pending;
  assert.deepEqual(values, ['current']);
});

test('all navigation paths clear the old details and quick-find selection', () => {
  const current = { page: 'Addons', selectedId: 'old-addon', command: true };
  const next = navigateState(current, 'Settings');
  assert.equal(next.page, 'Settings'); assert.equal(next.selectedId, null); assert.equal(next.command, false);
  assert.equal(selectedAddon([{ id: 'different' }], current.selectedId), null);
  assert.equal(navigateState(current, 'not-a-page'), current);
});

test('profile names stay bound to their local account and unavailable storage is optional', () => {
  const values = new Map(), storage = { getItem: (key) => values.get(key) || null, setItem: (key, value) => values.set(key, value) };
  assert.ok(saveProfileNames(storage, '/game', 'FIRST', [{ id: 'regular', name: 'Desktop' }]));
  assert.deepEqual(readProfileNames(storage, '/game', 'FIRST'), [{ id: 'regular', name: 'Desktop' }]);
  assert.equal(readProfileNames(storage, '/game', 'SECOND'), null);
  const unavailable = { getItem() { throw Error('Blocked'); }, setItem() { throw Error('Blocked'); } };
  assert.equal(readProfileNames(unavailable, '/game', 'FIRST'), null);
  assert.equal(saveProfileNames(unavailable, '/game', 'FIRST', []), false);
});

function document() {
  const dom = new JSDOM('<button id="opener">Open</button><div id="first"><button id="a">A</button><button disabled>Disabled</button><input id="b"></div><div id="second"><button id="c">C</button></div>');
  // jsdom has no layout; expose only the visibility property queried by the dialog.
  Object.defineProperty(dom.window.HTMLElement.prototype, 'offsetParent', { configurable: true, get() { return this.hidden ? null : this.ownerDocument.body; } });
  const get = (id) => dom.window.document.getElementById(id);
  const key = (name, shiftKey = false) => {
    const event = new dom.window.KeyboardEvent('keydown', { key: name, shiftKey, bubbles: true, cancelable: true });
    dom.window.document.activeElement.dispatchEvent(event); return event;
  };
  return { dom, get, key };
}

test('dialog keyboard boundaries skip disabled controls and restore the opener', () => {
  const { dom, get, key } = document();
  get('opener').focus();
  let closed = false;
  const cleanup = registerDialog(dom.window, get('first'), () => { closed = true; });
  assert.equal(dom.window.document.activeElement, get('a'));
  assert.equal(key('Tab', true).defaultPrevented, true);
  assert.equal(dom.window.document.activeElement, get('b'));
  assert.equal(key('Tab').defaultPrevented, true);
  assert.equal(dom.window.document.activeElement, get('a'));
  key('Escape'); assert.ok(closed);
  cleanup(); assert.equal(dom.window.document.activeElement, get('opener'));
  dom.window.close();
});

test('only the top dialog handles Escape and closing it resumes the underlying dialog', () => {
  const { dom, get, key } = document(), calls = [];
  get('opener').focus();
  const first = registerDialog(dom.window, get('first'), () => calls.push('first'));
  const second = registerDialog(dom.window, get('second'), () => calls.push('second'));
  key('Escape'); assert.deepEqual(calls, ['second']);
  second(); assert.equal(dom.window.document.activeElement, get('a'));
  key('Escape'); assert.deepEqual(calls, ['second', 'first']);
  first(); dom.window.close();
});

test('closing a dialog does not refocus a removed opener or leak a keyboard handler', () => {
  const { dom, get, key } = document();
  get('opener').focus();
  let calls = 0;
  const cleanup = registerDialog(dom.window, get('first'), () => calls++);
  get('opener').remove(); cleanup();
  key('Escape'); assert.equal(calls, 0);
  dom.window.close();
});


test('canceling the ZIP chooser does not publish cancellation metadata as app state', async () => {
  const states = [], errors = [];
  const client = createApplicationClient({ importZip: async () => ({ canceled: true }) }, {
    state: (value) => states.push(value), error: (error) => errors.push(error),
  });
  assert.equal(await client.execute('importZip', 'fixture'), undefined);
  assert.deepEqual(states, []);
  assert.deepEqual(errors, []);
});
