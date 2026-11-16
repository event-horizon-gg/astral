const fs = require('node:fs');
const path = require('node:path');

const actions = new Set(['state', 'client']);
const profiles = [
  { id: 'regular', name: 'Regular' },
  { id: 'handheld', name: 'Handheld' },
  { id: '0123456789abcdef0123456789abcdef', name: 'Raid setup' },
];
const pages = {
  '--library': 'Addons',
  '--discover': 'Discover',
  '--account': 'Your setup',
  '--activity': 'Activity',
  '--sources': 'Sources',
  '--settings': 'Settings',
};
const dialogs = {
  '--profile-review': null,
  '--sync-conflict': { conflict: true },
  '--sync-missing': {
    incomplete: true,
    missing: [{ folder: 'ExampleAddon', name: 'Example Addon', reason: 'Manual install' }],
  },
};

function createReferenceFixture(app, argv) {
  const index = argv.indexOf('--ui-fixture');
  if (index === -1) return null;
  const flags = argv.slice(index + 2);
  const client = flags.find((flag) => flag === '--retail' || flag === '--forever');
  const theme = flags.find((flag) => flag === '--light' || flag === '--dark');
  const dialog = flags.find((flag) => Object.hasOwn(dialogs, flag));
  const viewport = flags.find((flag) => flag === '--compact' || flag === '--wide');
  const signedIn = flags.includes('--signed-in');
  const notice = flags.find((flag) => flag === '--notice' || flag === '--notice-error');
  const page = flags.find((flag) => Object.hasOwn(pages, flag));
  if (
    app.isPackaged ||
    !argv[index + 1] ||
    flags.length !==
      [client, theme, dialog, viewport, page, signedIn || null, notice].filter(Boolean).length
  )
    throw Error(
      'Use --ui-fixture <fixture-directory> [--retail | --forever] [--light | --dark] [--compact | --wide] [--library | --discover | --account | --activity | --sources | --settings] [--sync-conflict | --sync-missing | --profile-review] [--signed-in] [--notice | --notice-error].',
    );

  const base = fs.realpathSync(argv[index + 1]);
  const roots = new Set();
  const addonDirectories = new Set();
  for (const name of ['_retail_', '_forever_']) {
    const root = path.join(base, name);
    const addons = path.join(root, 'Interface', 'AddOns');
    try {
      if (fs.realpathSync(addons) === addons && fs.statSync(addons).isDirectory()) {
        roots.add(root);
        addonDirectories.add(addons);
      }
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  if (!roots.size) throw Error('The fixture must contain a client folder with Interface/AddOns.');
  const root = client ? path.join(base, '_' + client.slice(2) + '_') : roots.values().next().value;
  if (!roots.has(root)) throw Error('The selected client is missing from the fixture.');

  const data = fs.mkdtempSync(path.join(base, '.astral-electron-'));
  app.setPath('userData', data);
  app.setPath('sessionData', data);
  return {
    roots,
    accountName: signedIn ? 'AstralPlayer#1234' : null,
    toast: notice
      ? {
          message:
            notice === '--notice-error'
              ? 'The cloud backup changed. Sync again to review it.'
              : 'Cloud backup restored. Local backup saved.',
          error: notice === '--notice-error',
        }
      : null,
    response(name) {
      if (!signedIn) return undefined;
      if (name === 'syncProfiles') return { profiles };
      if (name === 'cloudInfo')
        return {
          devices: [
            {
              id: 'c'.repeat(43),
              label: 'Desktop Mac',
              current: true,
              created: 1794600000,
              last_seen: 1794690000,
            },
            {
              id: 'd'.repeat(43),
              label: 'ROG Ally',
              current: false,
              created: 1794500000,
              last_seen: 1794680000,
            },
          ],
          history: [
            {
              id: 'a'.repeat(43),
              revision: 2,
              created: '2026-11-14T20:15:00Z',
              bytes: 68981,
              files: 69,
              device_label: 'Desktop Mac',
              current: true,
            },
            {
              id: 'b'.repeat(43),
              revision: 1,
              created: '2026-11-13T18:30:00Z',
              bytes: 65536,
              files: 68,
              device_label: 'ROG Ally',
              current: false,
            },
          ],
          revision: 2,
          usage: { paused: false },
        };
      if (name === 'sharedLibrary') return { addons: null };
      return undefined;
    },
    root,
    profileReview: dialog === '--profile-review'
      ? structuredClone(require('../native/fixtures/profile-review.json')[path.basename(root) === '_forever_' ? 'forever' : 'retail'])
      : null,
    page: pages[page] || (dialog === '--profile-review' ? 'Your setup' : 'Addons'),
    theme: theme?.slice(2) || 'dark',
    viewport:
      viewport === '--compact'
        ? { width: 1000, height: 720 }
        : viewport === '--wide'
          ? { width: 1920, height: 1000 }
          : { width: 1250, height: 840 },
    dialog: dialogs[dialog] || null,
    allowsAddonDirectory: (directory) => addonDirectories.has(directory),
    assertAction(name) {
      if (!actions.has(name)) throw Error('This action is unavailable in the reference fixture.');
    },
  };
}

module.exports = { createReferenceFixture };
