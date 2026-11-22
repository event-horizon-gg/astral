const path = require('node:path');
const os = require('node:os');
const engine = require('./engine.cjs');
const storage = require('./storage.cjs');
const settings = require('./settings.cjs');
const flavors = require('./flavors.cjs');
const updater = require('./updater.cjs');
const suites = require('./suites.cjs');
const addonActivity = require('./addon-activity.cjs');
const addonSelection = require('./addon-selection.cjs');
const versions = require('./addon-versions.cjs');
const profilePolicy = require('./profile-policy.cjs');
const profileContext = require('./profile-context.cjs');
async function collect({
  config,
  data,
  accounts,
  identity: session,
  backgroundUpdates,
  appVersion,
  appUpdates,
  releases,
  progress,
  lastError,
}) {
  const inventory = await engine.inventory(config.root, data);
  const type = flavors.flavor(config.root);
  const parent = path.dirname(config.root);
  const clients = [];
  for (const [id, folder, label] of [
    ['retail', '_retail_', 'Retail'],
    ['forever', '_forever_', 'Forever'],
  ]) {
    const root = path.join(parent, folder);
    if (await storage.exists(root)) clients.push({ id, root, label });
  }
  const artwork = new Map(inventory.addons.filter((a) => a.icon).map((a) => [a.id, a.icon]));
  let addonChoices = { rows: [] };
  if (config.account) {
    try {
      const base = await settings.accountRoot(config.root, config.account);
      addonChoices.rows = addonSelection
        .rows(await addonSelection.capture(config.root, base), inventory.addons)
        .map((row) => ({
          ...row,
          ...profilePolicy.choiceCapability(row.id, profileContext.profileId(config)),
        }));
    } catch (error) {
      addonChoices.error = error.message;
    }
  }
  for (const client of clients.filter((c) => c.root !== config.root)) {
    const other = await engine.inventory(client.root, data);
    for (const addon of other.addons)
      if (addon.icon && !artwork.has(addon.id)) artwork.set(addon.id, addon.icon);
  }
  return {
    ...inventory,
    addonChoices,
    flavor: type,
    clients,
    catalog: engine.catalog
      .filter((a) => !a.local && flavors.allowed(a, type))
      .map((a) => ({
        ...a,
        icon: artwork.get(a.id) || null,
        installed: inventory.addons.some((i) => i.id === a.id),
      })),
    root: config.root,
    account: config.account,
    accounts,
    cloudConfigured: !!config.service,
    signedIn: !!session,
    startup: !!config.startup,
    autoUpdates: config.autoUpdates !== false,
    backgroundUpdates,
    appVersion,
    appUpdates: { ...appUpdates?.status, enabled: config.appUpdateChecks !== false },
    legacyBackups: await storage.exists(
      path.join(
        process.platform === 'win32'
          ? process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
          : path.join(os.homedir(), 'Library', 'Application Support'),
        'Astral RaiderIO',
        'Backups',
      ),
    ),
    updateIgnored: config.updateIgnored?.[type] || [],
    dailyUpdates: await updater.status(data),
    suite: type === 'forever' ? { id: 'forever', addons: suites.FOREVER } : null,
    addonActivity: await addonActivity.list(data, config.root),
    legacyRio: await storage.exists(
      path.join(os.homedir(), 'Library', 'LaunchAgents', 'com.joaoseixas.astral-raiderio.plist'),
    ),
    accountName: session?.name || '',
    machine: os.hostname(),
    platform: process.platform,
    running: await engine.gameRunning().catch(() => null),
    backups: await engine.backups(data, config.root),
    progress,
    lastError,
    lastSync: config.lastSync || null,
    syncRevision: config.syncRevision || 0,
    syncProfile: profileContext.profileId(config),
    profileDefinitions: profileContext.defaults,
    syncProfilePending: !!config.syncProfilePending,
    providers: [
      {
        name: 'GitHub',
        status: 'Connected',
        detail: 'Official packaged releases',
        available: true,
      },
      {
        name: 'Wago',
        status: 'Access pending',
        detail: 'Manager API access has not been granted',
        available: false,
      },
      {
        name: 'CurseForge',
        status: 'Access pending',
        detail: 'Requires an approved third-party key',
        available: false,
      },
    ],
    addons: inventory.addons.map((a) => ({
      ...a,
      release: releases[a.id] || null,
      status:
        releases[a.id] && !releases[a.id].error
          ? versions.needsUpdate(a, releases[a.id])
            ? 'update'
            : 'current'
          : a.status,
    })),
  };
}
module.exports = { collect };
