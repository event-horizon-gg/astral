import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { createRequire } from 'node:module';
import { create as reviewStore } from '../desktop/reviews.cjs';
import { register as registerIPC } from '../desktop/ipc.cjs';
import { create as cloudSession } from '../desktop/cloud-session.cjs';
import { create as backgroundJobs } from '../desktop/background-jobs.cjs';
import context from '../desktop/profile-context.cjs';
const require = createRequire(import.meta.url),
  commands = require('../shared/commands.json');

test('sandbox preload exposes the declared commands without requiring local files', async () => {
  let exposed;
  const sent = [];
  const electron = {
    contextBridge: {
      exposeInMainWorld: (_, value) => {
        exposed = value;
      },
    },
    ipcRenderer: {
      invoke: async (...args) => {
        sent.push(args);
        return 'result';
      },
      on() {},
      removeListener() {},
    },
  };
  const sandbox = {
    require: (name) => {
      assert.equal(name, 'electron');
      return electron;
    },
    process: {
      argv: [
        'electron',
        '--astral-commands=' + encodeURIComponent(JSON.stringify(Object.keys(commands))),
      ],
    },
  };
  vm.runInNewContext(
    await fs.readFile(new URL('../desktop/preload.cjs', import.meta.url), 'utf8'),
    sandbox,
  );
  for (const name of Object.keys(commands)) assert.equal(await exposed[name]('value'), 'result');
  assert.deepEqual(
    sent.map(([channel]) => channel).sort(),
    Object.keys(commands)
      .map((name) => 'astral:' + name)
      .sort(),
  );
  assert.deepEqual(
    sent.find(([channel]) => channel === 'astral:applySetupImport'),
    ['astral:applySetupImport', 'value'],
  );
  assert.ok(Object.isFrozen(exposed));
});

test('IPC rejects other windows/frames and routes declared mutations through the exclusive gate', async () => {
  const handlers = new Map(),
    contents = { mainFrame: {} },
    errors = [];
  let guarded = 0,
    calls = 0;
  const register = registerIPC({
    ipcMain: { handle: (name, fn) => handlers.set(name, fn) },
    window: () => ({ webContents: contents }),
    exclusive: async (fn) => {
      guarded++;
      return fn();
    },
    clearError() {},
    failed: (error) => errors.push(error.message),
  });
  register('applyProfile', async () => {
    calls++;
    return 'done';
  });
  register('cloudInfo', async () => 'data');
  assert.throws(() => register('madeUp', () => {}), /Undeclared/);
  const apply = handlers.get('astral:applyProfile');
  await assert.rejects(apply({ sender: {}, senderFrame: contents.mainFrame }), /Untrusted/);
  await assert.rejects(apply({ sender: contents, senderFrame: {} }), /Untrusted/);
  assert.equal(calls, 0);
  assert.equal(await apply({ sender: contents, senderFrame: contents.mainFrame }), 'done');
  await handlers.get('astral:cloudInfo')({ sender: contents, senderFrame: contents.mainFrame });
  assert.equal(guarded, 1);
  register('restore', () => {
    throw Error('restore failed');
  });
  await assert.rejects(
    handlers.get('astral:restore')({ sender: contents, senderFrame: contents.mainFrame }),
    /restore failed/,
  );
  assert.deepEqual(errors, ['restore failed']);
});

test('reviews expire, bind to their context and dispose owned files on replacement and failure', async () => {
  let selected = { root: '/retail', account: 'A', profile: 'regular' },
    now = 100;
  const disposed = [];
  const reviews = reviewStore({
    context: () => selected,
    ttl: 50,
    now: () => now,
    message: 'Review again',
    dispose: async (value) => disposed.push(value.id),
  });
  const first = await reviews.replace({ id: 1 });
  const second = await reviews.replace({ id: 2 });
  assert.deepEqual(disposed, [1]);
  await assert.rejects(reviews.get(first), /Review again/);
  selected = { ...selected, profile: 'handheld' };
  await assert.rejects(reviews.get(second), /Review again/);
  assert.deepEqual(disposed, [1, 2]);
  const third = await reviews.replace({ id: 3 });
  now = 151;
  await assert.rejects(reviews.get(third), /Review again/);
  const fourth = await reviews.replace({ id: 4 });
  await assert.rejects(
    reviews.run(fourth, async () => {
      throw Error('install failed');
    }),
    /install failed/,
  );
  await assert.rejects(reviews.get(fourth), /Review again/);
  assert.deepEqual(disposed, [1, 2, 3, 4]);
});

test('a review prepared before a context change is rejected and its resources disposed', async () => {
  let selected = { root: '/retail' };
  const disposed = [];
  const reviews = reviewStore({
    context: () => selected,
    ttl: 100,
    message: 'Changed',
    dispose: async (value) => disposed.push(value),
  });
  const scope = reviews.capture();
  selected = { root: '/forever' };
  await assert.rejects(reviews.replace('prepared archive', scope), /Changed/);
  assert.deepEqual(disposed, ['prepared archive']);
});

test('cloud session owns encrypted credential persistence, authorization and logout cleanup', async (t) => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-session-'));
  t.after(() => fs.rm(data, { recursive: true, force: true }));
  const calls = [],
    opened = [];
  const response = (value, status = 200) => ({ ok: status < 400, status, json: async () => value });
  const session = cloudSession({
    configuration: () => ({ service: 'https://astral.example' }),
    directory: () => data,
    credentials: {
      available: async () => {},
      encrypt: async (value) => 'encrypted:' + value,
      decrypt: async (value) => value.slice(10),
    },
    changed() {},
    sleep: async (ms) => assert.equal(ms, 2000),
    request: async (url, options) => {
      calls.push([url, options]);
      if (url.endsWith('/auth/device'))
        return response({ id: 'device', url: 'https://astral.example/auth/device' });
      if (url.endsWith('/auth/device/poll')) return response({ token: 'secret', name: 'Player' });
      if (url.endsWith('/auth/logout')) return response({ error: 'offline' }, 503);
      return response({ ready: true });
    },
  });
  await session.connect(async (url) => opened.push(url));
  assert.deepEqual(opened, ['https://astral.example/auth/device']);
  assert.deepEqual(await session.identity(), { name: 'Player' });
  assert.match(await fs.readFile(path.join(data, 'session.json'), 'utf8'), /encrypted:secret/);
  await session.request('/cloud/retail');
  assert.equal(calls.at(-1)[1].headers.Authorization, 'Bearer secret');
  await assert.rejects(session.disconnect(), /offline/);
  assert.equal(await session.exists(), false);
});

function fakeTimers() {
  const entries = [];
  const add = (callback, delay, repeat) => {
    const entry = { callback, delay, repeat, cleared: false, unref() {} };
    entries.push(entry);
    return entry;
  };
  return {
    entries,
    setTimeout: (callback, delay) => add(callback, delay, false),
    setInterval: (callback, delay) => add(callback, delay, true),
    clearTimeout: (entry) => {
      entry.cleared = true;
    },
    clearInterval: (entry) => {
      entry.cleared = true;
    },
  };
}

test('background jobs honor pending profiles, WoW close, successful-check cadence and stop lifecycle', async () => {
  const timers = fakeTimers(),
    messages = [],
    errors = [];
  let running = true,
    pending = false,
    changed = 0,
    checks = 0;
  const jobs = backgroundJobs({
    timers,
    configuration: () => ({ syncProfilePending: pending }),
    updates: { active: false, status: { checkedAt: 1 }, check: async () => checks++ },
    now: () => 86400002,
    gameRunning: async () => running,
    hasSession: async () => true,
    state: async () => ({ signedIn: true }),
    changed: () => changed++,
    syncReady: async (message) => messages.push(message),
    daily: async () => {},
    busy: () => false,
    failed: (name, error) => errors.push([name, error.message]),
  });
  await jobs.start();
  assert.equal(messages.length, 0);
  running = false;
  await timers.entries.find((entry) => entry.repeat && entry.delay === 15000).callback();
  assert.equal(changed, 1);
  assert.match(messages[0], /WoW closed/);
  pending = true;
  await timers.entries.find((entry) => entry.repeat && entry.delay === 300000).callback();
  assert.equal(messages.length, 1);
  await timers.entries.find((entry) => entry.delay === 10000).callback();
  assert.equal(checks, 1);
  jobs.stop();
  assert.ok(timers.entries.every((entry) => entry.cleared));
  for (const entry of timers.entries) await entry.callback();
  assert.equal(checks, 1);
  assert.deepEqual(errors, []);
});

test('profile identity keeps legacy Regular scope and selected setup isolation', () => {
  const regular = { root: '/wow/_retail_', account: 'A', syncProfile: 'regular' };
  assert.equal(context.syncScope(regular), 'retail');
  assert.equal(context.syncScope({ ...regular, syncProfile: 'handheld' }), 'retail-handheld');
  assert.equal(context.syncScope({ ...regular, root: '/wow/_forever_' }), 'forever');
  assert.deepEqual(context.selected(regular), {
    root: regular.root,
    account: 'A',
    profile: 'regular',
  });
  assert.equal(context.validId('unregistered-name'), false);
  assert.equal(context.validId('a'.repeat(32)), true);
  const pending = context.localChanges({
    ...regular,
    lastHash: 'old',
    lastRemoteHash: 'remote',
    lastSync: 'date',
    syncRevision: 3,
  });
  assert.equal(pending.lastHash, null);
  assert.equal(pending.syncRevision, 0);
  assert.equal(pending.syncProfilePending, true);
  assert.equal(regular.syncProfilePending, undefined);
});

test('concurrent review preparation retires the older result and consumption owns its resource', async () => {
  const disposed = [],
    failures = [];
  const reviews = reviewStore({
    context: () => 'same',
    ttl: 1000,
    message: 'stale',
    dispose: async (value) => {
      disposed.push(value);
      if (value === 'active') throw Error('cleanup failed');
    },
    cleanupFailed: (error) => failures.push(error.message),
  });
  const older = reviews.replace('older'),
    latest = reviews.replace('latest');
  await assert.rejects(older, /stale/);
  const token = await latest;
  assert.deepEqual(disposed, ['older']);
  let finish;
  const applied = reviews.run(
    token,
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await Promise.resolve();
  const next = await reviews.replace('active');
  assert.deepEqual(disposed, ['older']);
  await assert.rejects(reviews.get(token), /stale/);
  finish('done');
  assert.equal(await applied, 'done');
  await assert.rejects(
    reviews.run(next, () => {
      throw Error('original failure');
    }),
    /original failure/,
  );
  assert.deepEqual(disposed, ['older', 'latest', 'active']);
  assert.deepEqual(failures, ['cleanup failed']);
});

test('Electron composition registers the complete command contract and serves scoped state without native apps', async (t) => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-main-'));
  t.after(() => fs.rm(data, { recursive: true, force: true }));
  const root = path.join(data, '_retail_'),
    account = path.join(root, 'WTF', 'Account', 'ONE', 'SavedVariables');
  await fs.mkdir(account, { recursive: true });
  const addon = path.join(root, 'Interface', 'AddOns', 'ConsolePort');
  await fs.mkdir(addon, { recursive: true });
  await fs.writeFile(path.join(addon, 'ConsolePort.toc'), '## Title: ConsolePort\n## Version: 1\n');
  await fs.writeFile(
    path.join(data, 'config.json'),
    JSON.stringify({ root, service: 'https://astral.example' }),
  );
  const handlers = new Map(),
    failures = [];
  let boot, contents, windowOptions;
  class FakeWindow {
    constructor(options) {
      windowOptions = options;
      this.webContents = contents = {
        mainFrame: {},
        send() {},
        on() {},
        setWindowOpenHandler() {},
        session: { setPermissionRequestHandler() {} },
      };
    }
    on() {}
    async loadFile() {}
    async loadURL() {}
    show() {}
    hide() {}
    focus() {}
  }
  const electron = {
    app: {
      isPackaged: false,
      getPath: () => data,
      getVersion: () => '0.8.0',
      requestSingleInstanceLock: () => true,
      on() {},
      quit() {},
      whenReady: () => ({ then: (callback) => (boot = Promise.resolve().then(callback)) }),
    },
    BrowserWindow: FakeWindow,
    ipcMain: { handle: (channel, handler) => handlers.set(channel, handler) },
    dialog: { showErrorBox: (...args) => failures.push(args) },
    shell: {},
    safeStorage: {},
    Tray: class {
      setToolTip() {}
      setContextMenu() {}
      on() {}
    },
    Menu: { buildFromTemplate: (value) => value },
    nativeImage: { createFromPath: () => ({ resize: () => ({}) }) },
  };
  const main = new URL('../desktop/main.cjs', import.meta.url),
    nativeRequire = createRequire(main);
  vm.runInNewContext(await fs.readFile(main, 'utf8'), {
    require: (name) =>
      name === 'electron'
        ? electron
        : name === './background-jobs.cjs'
          ? { create: () => ({ start: async () => {}, stop() {} }) }
          : nativeRequire(name),
    __dirname: path.dirname(main.pathname),
    process: {
      argv: ['electron'],
      platform: process.platform,
      execPath: process.execPath,
      env: {},
    },
    console,
    setTimeout,
    setInterval,
    clearTimeout,
    clearInterval,
    URL,
    AbortSignal,
    fetch,
  });
  await boot;
  assert.deepEqual(failures, []);
  assert.equal(windowOptions.webPreferences.sandbox, true);
  assert.deepEqual(
    JSON.parse(
      decodeURIComponent(
        windowOptions.webPreferences.additionalArguments[0].slice('--astral-commands='.length),
      ),
    ),
    Object.keys(commands),
  );
  assert.deepEqual(
    [...handlers.keys()].sort(),
    Object.keys(commands)
      .map((name) => 'astral:' + name)
      .sort(),
  );
  const event = { sender: contents, senderFrame: contents.mainFrame };
  let bridge;
  vm.runInNewContext(
    await fs.readFile(new URL('../desktop/preload.cjs', import.meta.url), 'utf8'),
    {
      require: (name) => {
        assert.equal(name, 'electron');
        return {
          contextBridge: {
            exposeInMainWorld: (_, value) => {
              bridge = value;
            },
          },
          ipcRenderer: {
            invoke: (channel, ...args) => handlers.get(channel)(event, ...args),
            on() {},
            removeListener() {},
          },
        };
      },
      process: { argv: ['electron', ...windowOptions.webPreferences.additionalArguments] },
    },
  );
  assert.deepEqual(
    Object.keys(bridge)
      .filter((name) => !name.startsWith('on'))
      .sort(),
    Object.keys(commands).sort(),
  );
  const current = await bridge.state();
  assert.equal(current.root, root);
  assert.equal(current.account, 'ONE');
  assert.equal(current.profileDefinitions[0].id, 'regular');
  assert.equal(current.addonChoices.rows.find((row) => row.id === 'consoleport').managed, true);
  assert.equal(
    JSON.parse(await fs.readFile(path.join(data, 'config.json'), 'utf8')).account,
    'ONE',
  );
  const exportInfo = await bridge.setupExportInfo();
  assert.equal(exportInfo.addons, undefined);
  assert.ok(exportInfo.addonNames.includes('ConsolePort'));
});
