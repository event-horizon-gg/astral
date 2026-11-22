import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';

const noop = () => {};
const addon = {
  id: 'fixture',
  name: 'Fixture addon',
  folder: 'Fixture',
  modules: ['Fixture'],
  category: 'UI',
  source: 'GitHub',
  repo: 'example/addon',
  version: '1',
  status: 'update',
  description: 'Addon description',
  release: { tag: '2', notes: 'Changed the UI.' },
};
const s = {
  root: '/temporary/_retail_',
  account: 'LOCAL',
  accounts: ['LOCAL'],
  flavor: 'retail',
  platform: 'darwin',
  loaded: true,
  clients: [
    { id: 'retail', label: 'Retail' },
    { id: 'forever', label: 'Forever' },
  ],
  addons: [addon],
  backups: [],
  providers: [{ name: 'GitHub', available: true }],
  catalog: [],
  signedIn: true,
  cloudConfigured: true,
  accountName: 'Player#1234',
  syncProfile: 'handheld',
  syncProfilePending: false,
  addonChoices: {
    rows: [
      {
        id: 'consoleport',
        name: 'ConsolePort',
        enabled: true,
        managed: true,
        managedReason: 'Enabled in Handheld only',
      },
    ],
  },
  updateIgnored: [],
  addonActivity: [],
  dailyUpdates: { runs: [] },
  autoUpdates: true,
  backgroundUpdates: {},
  appVersion: '0.8.0',
  appUpdates: { phase: 'available', latest: '0.8.1', notes: 'New version', enabled: true },
};

// Render every extracted screen with representative data, catching unbound
// identifiers and broken imports that a bundler alone does not report.
test('all extracted screens and dialogs render their actual DOM without desktop services', async (t) => {
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: { getItem: () => null, setItem: noop },
  });
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)),
    configFile: false,
    server: { middlewareMode: true, hmr: false, ws: false, watch: { ignored: ['**/*'] } },
    appType: 'custom',
  });
  t.after(async () => {
    await server.close();
    if (previousStorage) Object.defineProperty(globalThis, 'localStorage', previousStorage);
    else delete globalThis.localStorage;
  });
  const common = {
    s,
    client: { available: false },
    busy: false,
    call: noop,
    notify: noop,
    setDialog: noop,
    navigate: noop,
    setSelected: noop,
    check: noop,
    updateAll: noop,
  };
  const cases = [
    [
      'components/AppHeader.jsx',
      {
        ...common,
        page: 'Addons',
        theme: 'dark',
        setTheme: noop,
        setQuery: noop,
        onNavigate: noop,
        onCommand: noop,
        onShare: noop,
      },
      'Library',
    ],
    [
      'components/PageHeader.jsx',
      { ...common, page: 'Addons', updates: 1, api: true, progress: {} },
      'Update all',
    ],
    [
      'features/library/LibraryPage.jsx',
      {
        ...common,
        queueResult: null,
        filter: 'All addons',
        setFilter: noop,
        updates: 1,
        search: { current: null },
        query: '',
        setQuery: noop,
        filtered: [addon],
        selected: null,
        progress: {},
        update: noop,
      },
      'Fixture addon',
    ],
    [
      'features/library/AddonDrawer.jsx',
      {
        ...common,
        selected: addon,
        update: noop,
        importZip: noop,
        selectedBackup: null,
        selectedFailure: null,
      },
      'Addon details',
    ],
    [
      'features/discovery/DiscoveryPanel.jsx',
      { ...common, AddonIcon: () => null, onLibrary: noop },
      'Search',
    ],
    [
      'features/account/AccountPage.jsx',
      { ...common, time: () => 'Today' },
      'Connected with Battle.net',
    ],
    ['features/activity/ActivityPage.jsx', common, 'No backups yet'],
    ['features/settings/SettingsPage.jsx', common, 'Astral updates'],
    ['features/sources/SourcesPage.jsx', common, 'Visit GitHub'],
    [
      'components/AppDialogs.jsx',
      { ...common, command: true, setCommand: noop, dialog: null },
      'Profiles &amp; sync',
    ],
    [
      'components/AppDialogs.jsx',
      {
        ...common,
        command: false,
        setCommand: noop,
        dialog: {
          restore: { id: 'previous', name: 'Fixture addon', time: '2026-01-01T12:00:00Z' },
        },
      },
      'Restore previous',
    ],
    [
      'features/sharing/ShareSetup.jsx',
      {
        ...common,
        share: { mode: 'export', info: { characters: [], addonNames: ['Fixture addon'] } },
        onClose: noop,
      },
      'Export file',
    ],
    [
      'features/sharing/ShareSetup.jsx',
      {
        ...common,
        share: {
          mode: 'import',
          info: {
            token: 'preview',
            name: '<script>unsafe</script>',
            character: 'Realm/Source',
            characters: [{ value: 'Realm/Target', label: 'Target' }],
            addonList: [{ name: 'Fixture addon' }],
            includes: { settings: true, bindings: true },
          },
        },
        onClose: noop,
      },
      'Review import',
    ],
  ];
  for (const [module, props, expected] of cases) {
    const { default: Component } = await server.ssrLoadModule('/src/' + module);
    const html = renderToStaticMarkup(React.createElement(Component, props));
    assert.ok(html.includes(expected), `${module} did not render ${expected}`);
    assert.ok(!html.includes('<script>unsafe</script>'));
  }
});

const deferred = () => {
  let resolve;
  const promise = new Promise((value) => {
    resolve = value;
  });
  return { promise, resolve };
};

test('the actual app starts with no dialog, navigates all screens, and runs drawer/share actions through its bridge', async (t) => {
  const { JSDOM } = await import('jsdom');
  const { act } = await import('react');
  const dom = new JSDOM('<div id="root"></div>', { url: 'http://localhost/' });
  const names = [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'localStorage',
    'IS_REACT_ACT_ENVIRONMENT',
  ];
  const previous = new Map(
    names.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
  );
  for (const [name, value] of Object.entries({
    window: dom.window,
    document: dom.window.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  }))
    Object.defineProperty(globalThis, name, { configurable: true, value });
  Object.defineProperty(dom.window.HTMLElement.prototype, 'offsetParent', {
    configurable: true,
    get() {
      return this.hidden ? null : this.ownerDocument.body;
    },
  });
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)),
    configFile: false,
    server: { middlewareMode: true, hmr: false, ws: false, watch: { ignored: ['**/*'] } },
    appType: 'custom',
  });
  const { createRoot } = await import('react-dom/client');
  const { default: App } = await server.ssrLoadModule('/src/application/App.jsx');
  const root = createRoot(dom.window.document.getElementById('root'));
  t.after(async () => {
    await act(() => root.unmount());
    await server.close();
    dom.window.close();
    for (const name of names) {
      if (previous.get(name)) Object.defineProperty(globalThis, name, previous.get(name));
      else delete globalThis[name];
    }
  });
  let current = structuredClone(s);
  current.addons[0].canImport = true;
  current.addonActivity = [
    {
      id: 'failed',
      addonId: addon.id,
      name: addon.name,
      status: 'error',
      action: 'update',
      message: 'Try again',
      time: '2026-01-01T12:00:00Z',
    },
  ];
  const initial = deferred(),
    calls = [],
    events = {};
  let firstRead = true;
  const snapshot = () => structuredClone(current);
  let cloudInfo = async () => ({
    usage: { paused: false },
    devices: [],
    history: [],
    revision: 0,
  });
  const pendingSearch = deferred(),
    pendingPreview = deferred();
  const bridge = {
    state: () =>
      firstRead ? ((firstRead = false), initial.promise) : Promise.resolve(snapshot()),
    onProgress(callback) {
      events.progress = callback;
      return noop;
    },
    onChanged(callback) {
      events.changed = callback;
      return noop;
    },
    discover: () => pendingSearch.promise,
    previewProfile: () => pendingPreview.promise,
    cloudInfo: () => cloudInfo(),
    restoreCloud: async (...args) => {
      calls.push(['restoreCloud', ...args]);
      throw Error('Cloud revision changed');
    },
    sharedLibrary: async () => ({ addons: [], revision: 0 }),
    syncProfiles: async () => ({
      profiles: [
        { id: 'regular', name: 'Regular' },
        { id: 'handheld', name: 'Handheld' },
      ],
    }),
    update: async (id) => {
      calls.push(['update', id]);
      current.addons[0].version = '2';
      current.addons[0].status = 'current';
      return snapshot();
    },
    importZip: async (id) => {
      calls.push(['importZip', id]);
      return { canceled: true };
    },
    setupExportInfo: async () => ({ addonNames: ['Fixture addon'], characters: [] }),
    exportSetup: async (value) => {
      calls.push(['exportSetup', value]);
      return { exported: true };
    },
  };
  const get = (selector) => dom.window.document.querySelector(selector);
  const button = (label) =>
    [...dom.window.document.querySelectorAll('button')].find(
      (item) => item.textContent.trim() === label || item.getAttribute('aria-label') === label,
    );
  const click = async (label) => {
    const element = button(label);
    assert.ok(element, `Missing button ${label}`);
    await act(() => element.click());
  };
  const heading = () => get('h1')?.textContent;
  const tools = async (label) => {
    await click('Tools');
    await click(label);
  };
  await act(() => root.render(React.createElement(App, { api: bridge })));
  assert.equal(get('[role="dialog"]'), null);
  assert.match(get('main').textContent, /Loading addons/);
  await act(async () => initial.resolve(snapshot()));
  assert.equal(heading(), 'Library');
  await click('Fixture addon');
  assert.ok(get('aside[aria-label="Addon details"]'));
  await click('Update from ZIP');
  assert.deepEqual(calls.at(-1), ['importZip', addon.id]);
  assert.equal(heading(), 'Library');
  assert.ok(get('aside[aria-label="Addon details"]'));
  await click('Go to');
  await click('Settings');
  assert.equal(heading(), 'Settings');
  assert.equal(get('aside[aria-label="Addon details"]'), null);
  await tools('Sources');
  assert.equal(heading(), 'Sources');
  await tools('Activity & backups');
  assert.equal(heading(), 'Activity');
  await click('Retry');
  assert.deepEqual(calls.at(-1), ['update', addon.id]);
  await click('Close details');
  await click('Open library');
  assert.equal(heading(), 'Library');
  await click('Discover');
  assert.equal(heading(), 'Discover');

  const find = get('input[aria-label="Find addons"]');
  const setInputValue = Object.getOwnPropertyDescriptor(
    dom.window.HTMLInputElement.prototype,
    'value',
  ).set;
  await act(() => {
    setInputValue.call(find, 'old query');
    find.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  });
  await act(() =>
    get('form.finder-search').dispatchEvent(
      new dom.window.Event('submit', { bubbles: true, cancelable: true }),
    ),
  );
  await act(() => {
    setInputValue.call(find, '');
    find.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  });
  await act(async () =>
    pendingSearch.resolve({
      results: [{ name: 'Old project', repo: 'old/project', description: 'Earlier query' }],
    }),
  );
  assert.ok(!get('main').textContent.includes('Old project'));
  await click('Account menu');
  await click('Profiles & sync');
  assert.equal(heading(), 'Profiles & sync');
  assert.ok(get('input[aria-label="ConsolePort"]').disabled);

  current.addonChoices.rows.push({
    id: addon.id,
    name: addon.name,
    enabled: false,
    managed: false,
  });
  await act(async () => events.changed());
  await act(() => get('input[aria-label="Fixture addon"]').click());
  await click('Review changes (1)');
  current.addonChoices.rows[1].enabled = true;
  await act(async () => events.changed());
  await act(async () =>
    pendingPreview.resolve({
      token: 'stale-preview',
      changedSettings: [],
      changes: [],
      packages: [],
      missing: [],
    }),
  );
  assert.equal(get('[role="dialog"]'), null);
  cloudInfo = async () => ({
    usage: { paused: false },
    devices: [],
    revision: 10,
    history: [
      { id: 'backup', revision: 2, created: '2026-01-01T12:00:00Z', bytes: 20, files: 1 },
    ],
  });
  current.syncRevision = 1;
  await act(async () => events.changed());
  await click('Restore');
  assert.equal(get('[role="dialog"] h2').textContent, 'Restore version 2?');
  const pendingCloud = deferred();
  cloudInfo = () => pendingCloud.promise;
  current.syncRevision = 2;
  await act(async () => events.changed());
  assert.match(get('main').textContent, /Loading your cloud setup/);
  await click('Restore backup');
  assert.deepEqual(calls.at(-1), ['restoreCloud', 'backup', 10]);
  await act(async () =>
    pendingCloud.resolve({ usage: { paused: false }, devices: [], history: [], revision: 11 }),
  );
  current.root = '/temporary/other/_retail_';
  await act(async () => events.changed());
  assert.equal(get('[role="dialog"]'), null);

  await tools('Export setup');
  assert.equal(get('[role="dialog"] h2').textContent, 'Export setup');
  const form = get('form.share-setup');
  await act(async () =>
    form.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true })),
  );
  assert.equal(calls.at(-1)[0], 'exportSetup');
  assert.equal(get('[role="dialog"]'), null);
  assert.equal(heading(), 'Profiles & sync');
});
