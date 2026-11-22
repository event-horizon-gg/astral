const {
  app,
  BrowserWindow,
  ipcMain,
  dialog,
  shell,
  safeStorage,
  Tray,
  Menu,
  nativeImage,
} = require('electron');
const path = require('node:path'),
  fs = require('node:fs/promises'),
  os = require('node:os'),
  crypto = require('node:crypto');
const engine = require('./engine.cjs'),
  settings = require('./settings.cjs');
const storage = require('./storage.cjs');
const flavors = require('./flavors.cjs');
const updater = require('./updater.cjs'),
  projects = require('./projects.cjs');
const scheduler = require('./scheduler.cjs');
const addonActivity = require('./addon-activity.cjs');
const suites = require('./suites.cjs');
const profileContext = require('./profile-context.cjs');
const reviews = require('./reviews.cjs');
const clients = require('./clients.cjs');
const credentials = require('./credentials.cjs').create(safeStorage);
const { AppUpdates } = require('./app-updates.cjs');
let appUpdates, backgroundJobs;
const ackArgument = process.argv.indexOf('--astral-update-ack');
let updateAck = ackArgument >= 0 ? process.argv[ackArgument + 1] : null;
async function acknowledgeUpdate() {
  if (!/^[a-f0-9]{48}$/.test(updateAck || '')) return;
  await fs.writeFile(path.join(data, 'app-updates', 'ack-' + updateAck), 'ready', { mode: 0o600 });
  updateAck = null;
}
const scheduledLaunch = process.argv.includes('--daily-update');
let foregroundRequested = false;
let backgroundUpdates = {
  enabled: false,
  supported: ['darwin', 'win32'].includes(process.platform),
};
const previews = reviews.create({
  context: () => ({ root: path.resolve(config.root) }),
  ttl: 600000,
  message: 'This package review expired or belongs to another game client. Review it again.',
  dispose: (preview) => fs.rm(preview.temp, { recursive: true, force: true }),
});
let taskActive = false;
async function exclusive(fn) {
  if (taskActive) throw Error('Wait until the current operation finishes.');
  taskActive = true;
  try {
    return await fn();
  } finally {
    taskActive = false;
  }
}
let win,
  data,
  config,
  tray,
  releases = {},
  progress = {},
  lastError = null;
const send = (value) => {
  progress = value;
  win?.webContents.send('astral:progress', value);
};
async function persist() {
  await storage.save(path.join(data, 'config.json'), config);
}
const runtime = {
  get config() {
    return config;
  },
  set config(value) {
    config = value;
  },
  get data() {
    return data;
  },
};
const session = require('./cloud-session.cjs').create({
  configuration: () => config,
  directory: () => data,
  credentials,
  changed: () => win?.webContents.send('astral:changed'),
});
async function signalSyncReady(message) {
  if (config.syncProfilePending) return;
  try {
    await credentials.available();
    send({ kind: 'sync-ready', message });
  } catch (error) {
    send({ kind: 'sync-paused', message: error.message });
  }
}
async function reconcileAccount() {
  const accounts = await settings.accounts(config.root);
  if (!config.account && accounts.length === 1) {
    config = profileContext.accountChanged(config, accounts[0]);
    await persist();
  }
  return accounts;
}
async function state() {
  const accounts = await reconcileAccount();
  return require('./app-state.cjs').collect({
    config,
    data,
    accounts,
    identity: await session.identity(),
    backgroundUpdates,
    appVersion: app.getVersion(),
    appUpdates,
    releases,
    progress,
    lastError,
  });
}
const handle = require('./ipc.cjs').register({
  ipcMain,
  window: () => win,
  exclusive,
  clearError: () => {
    lastError = null;
  },
  failed: (error) => {
    lastError = error.message;
  },
});
async function initialize() {
  if (config) return;
  data = app.getPath('userData');
  await fs.mkdir(data, { recursive: true });
  config = await storage.json(path.join(data, 'config.json'), {});
  if (!config.service) config.service = 'https://astral-sync.astral-joaoseixas.workers.dev';
  if (!config.root) config.root = await engine.defaultRoot();
  if (!config.device) config.device = crypto.randomUUID();
  if (config.autoUpdates === undefined) config.autoUpdates = true;
  appUpdates = new AppUpdates({
    data,
    current: app.getVersion(),
    changed: () => win?.webContents.send('astral:changed'),
  });
  await fs.rm(path.join(data, 'previews'), { recursive: true, force: true });
  await persist();
}
async function configureSchedule() {
  if (!app.isPackaged) return;
  try {
    backgroundUpdates = await scheduler.ensure({
      executable: process.execPath,
      enabled: config.autoUpdates !== false,
    });
  } catch (error) {
    backgroundUpdates = {
      enabled: false,
      supported: true,
      error: 'Background scheduling needs attention: ' + error.message,
    };
  }
}
async function daily(force = false) {
  if (config.autoUpdates === false && !force) return;
  if (!force && Date.now() < (await updater.status(data)).nextAt) return;
  if (await engine.gameRunning()) {
    if (force) throw Error('Daily updates wait for WoW to close.');
    return;
  }
  return exclusive(async () => {
    const roots = [];
    const selectedType = flavors.flavor(config.root);
    for (const type of ['retail', 'forever']) {
      const root =
        type === selectedType
          ? config.root
          : config.spaces?.[type]?.root || path.join(path.dirname(config.root), '_' + type + '_');
      if (await storage.exists(path.join(root, 'Interface', 'AddOns'))) roots.push(root);
    }
    try {
      await updater.run({
        data,
        roots,
        force,
        ignored: config.updateIgnored,
        legacyRio:
          process.platform === 'darwin' &&
          (await storage.exists(
            path.join(
              os.homedir(),
              'Library',
              'LaunchAgents',
              'com.joaoseixas.astral-raiderio.plist',
            ),
          )),
        onProgress: send,
      });
    } finally {
      send({});
      win?.webContents.send('astral:changed');
    }
  });
}
async function main() {
  await initialize();
  await configureSchedule();
  handle('state', state);
  handle('checkAppUpdate', async () => {
    await appUpdates.check();
    return state();
  });
  handle('downloadAppUpdate', async () => {
    await appUpdates.download();
    return state();
  });
  handle('appUpdateChecks', async (enabled) => {
    config.appUpdateChecks = !!enabled;
    await persist();
    return state();
  });
  handle('installAppUpdate', async () => {
    if (!app.isPackaged)
      throw Error('App updates are available in the installed version of Astral.');
    if (appUpdates.active) throw Error('Wait for the update download to finish.');
    if (await engine.gameRunning()) throw Error('Close WoW before restarting Astral to update.');
    await persist();
    if (process.platform === 'darwin') {
      const target = path.resolve(app.getAppPath(), '../../..');
      const plan = await appUpdates.prepareMac(target);
      try {
        await scheduler.ensure({ executable: process.execPath, enabled: false });
        await appUpdates.launchMac(plan);
      } catch (error) {
        await fs.rm(plan.stage, { recursive: true, force: true });
        await configureSchedule();
        throw error;
      }
      // The detached installer owns restart and rollback. Chromium's normal quit
      // can wait indefinitely for a pending macOS Keychain request.
      app.exit(0);
    } else if (process.platform === 'win32') {
      await appUpdates.launchWindows(engine.gameRunning);
      app.quit();
    } else throw Error('App installation is supported on macOS and Windows.');
  });
  handle('discover', async (query) => {
    const projects = await require('./discovery.cjs').search(query);
    const inv = await engine.inventory(config.root, data);
    return {
      results: projects.map((project) => {
        const known =
          inv.addons.find((a) => a.repo?.toLowerCase() === project.repo.toLowerCase()) ||
          engine.catalog.find((a) => a.repo?.toLowerCase() === project.repo.toLowerCase());
        const installed = known && inv.addons.find((a) => a.id === known.id);
        return {
          ...project,
          ...(known && flavors.allowed(known, flavors.flavor(config.root)) ? known : {}),
          icon: installed?.icon || null,
          installed: !!installed,
          managed: !!known && flavors.allowed(known, flavors.flavor(config.root)),
        };
      }),
    };
  });
  handle('openLegacyBackups', async () => {
    const root =
      process.platform === 'win32'
        ? process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
        : path.join(os.homedir(), 'Library', 'Application Support');
    const folder = path.join(root, 'Astral RaiderIO', 'Backups');
    if (!(await storage.exists(folder))) throw Error('No earlier Raider.IO backups were found.');
    return shell.openPath(folder);
  });
  handle('openProvider', async (name) => {
    const urls = {
      GitHub: 'https://github.com',
      Wago: 'https://addons.wago.io',
      CurseForge: 'https://www.curseforge.com/wow',
    };
    if (!urls[name]) throw Error('Unknown provider.');
    return shell.openExternal(urls[name]);
  });
  handle('openProject', async (repo) => {
    if (!require('./github.cjs').validRepo(repo)) throw Error('Invalid GitHub project.');
    return shell.openExternal('https://github.com/' + repo + '/releases');
  });
  require('./cloud-handlers.cjs').register({
    handle,
    runtime,
    state,
    persist,
    send,
    session,
    shell,
    busy: () => taskActive,
  });
  handle('projectRelease', async (repo) => projects.releases(repo));
  handle('inspectProject', async (repo, assetId) => {
    if (taskActive) throw Error('Wait until the current operation finishes.');
    const reviewContext = previews.capture();
    await previews.clear();
    if (!Number.isSafeInteger(assetId)) throw Error('Invalid release package.');
    const preview = await projects.inspect(repo, assetId, config.root, data);
    const token = await previews.replace(preview, reviewContext);
    const missing = [];
    for (const folder of preview.definition.requires || [])
      if (!(await storage.exists(path.join(config.root, 'Interface', 'AddOns', folder))))
        missing.push(folder);
    return {
      token,
      repo: preview.repo,
      tag: preview.tag,
      details: preview.details,
      interface: preview.interface,
      assetName: preview.assetName,
      missing,
    };
  });
  handle('installProject', (token) =>
    previews.run(token, async (preview) => {
      try {
        send({
          kind: 'update',
          id: preview.definition.id,
          percent: 0,
          message: 'Installing ' + preview.definition.name,
        });
        await projects.install(preview, config.root, data, (p) =>
          send({
            kind: 'update',
            id: preview.definition.id,
            percent: p,
            message: 'Installing ' + preview.definition.name,
          }),
        );
        return await state();
      } finally {
        send({});
      }
    }),
  );
  handle('autoUpdates', async (enabled) => {
    config.autoUpdates = !!enabled;
    await persist();
    await configureSchedule();
    return state();
  });
  handle('autoIgnore', async (id, exclude) => {
    if (
      !(await engine.definitions(config.root, data)).some(
        (a) => a.id === id && (a.repo || a.bundled),
      )
    )
      throw Error('Unknown connected addon.');
    const type = flavors.flavor(config.root);
    config.updateIgnored = config.updateIgnored || {};
    const ids = new Set(config.updateIgnored[type] || []);
    exclude ? ids.add(id) : ids.delete(id);
    config.updateIgnored[type] = [...ids];
    await persist();
    return state();
  });
  handle('runDailyUpdates', async () => {
    await daily(true);
    return state();
  });
  handle('startup', async (enabled) => {
    config.startup = !!enabled;
    app.setLoginItemSettings({ openAtLogin: !!enabled, args: ['--background'] });
    await persist();
    return state();
  });
  handle('client', async (type) => {
    config = await clients.select(config, { type });
    releases = {};
    send({});
    await persist();
    return state();
  });
  handle('check', async () => {
    const all = await engine.inventory(config.root, data);
    for (const [i, a] of all.addons
      .filter((a) => (a.repo || a.bundled) && flavors.allowed(a, flavors.flavor(config.root)))
      .entries()) {
      send({
        kind: 'check',
        id: a.id,
        message: 'Checking ' + a.name,
        percent: Math.round((i / Math.max(1, all.addons.length)) * 100),
      });
      try {
        releases[a.id] = await engine.getRelease(a, data, true);
      } catch (e) {
        releases[a.id] = { error: e.message };
      }
    }
    send({});
    return state();
  });
  handle('update', async (id) => {
    const definition = (await engine.definitions(config.root, data)).find(
      (addon) => addon.id === id,
    );
    if (!definition?.repo && !definition?.bundled)
      throw Error('This addon needs a connected official source.');
    const inventory = await engine.inventory(config.root, data);
    const addon = inventory.addons.find((item) => item.id === id) || definition;
    try {
      const outcome = await require('./addon-operations.cjs').execute({
        addon,
        root: config.root,
        data,
        fresh: false,
        versionPolicy: addon.installed ? 'never-downgrade' : 'replace',
        onProgress: send,
      });
      releases[id] = outcome.release;
      return await state();
    } finally {
      send({});
    }
  });
  handle('installSuite', async (id) => {
    try {
      const result = await suites.install({ id, root: config.root, data, onProgress: send });
      return { ...(await state()), suiteResult: result };
    } finally {
      send({});
    }
  });
  handle('importZip', async (id) => {
    const addon = engine.catalog.find((a) => a.id === id);
    if (!addon || addon.local || !flavors.allowed(addon, flavors.flavor(config.root)))
      throw Error('ZIP updates are unavailable for this addon.');
    if (await engine.gameRunning()) throw Error('Close World of Warcraft before updating addons.');
    const root = config.root;
    const result = await dialog.showOpenDialog(win, {
      title: 'Update ' + addon.name + ' from ZIP',
      message:
        'Choose the addon ZIP from its official source. Astral backs up the current version and keeps your settings.',
      filters: [{ name: 'Addon ZIP', extensions: ['zip'] }],
      properties: ['openFile'],
    });
    if (result.canceled || !result.filePaths[0]) return { canceled: true };
    if (config.root !== root)
      throw Error('The selected game client changed. Choose the ZIP again.');
    try {
      send({ id, kind: 'update', percent: 0, message: 'Importing ' + addon.name });
      const installed = (await engine.inventory(root, data)).addons.find((a) => a.id === id);
      await addonActivity.track(data, root, installed || addon, 'import', () =>
        engine.importZip(id, root, data, result.filePaths[0], (percent) =>
          send({ id, kind: 'update', percent, message: 'Importing ' + addon.name }),
        ),
      );
      delete releases[id];
      return await state();
    } finally {
      send({});
    }
  });
  handle('restore', async (id) => {
    const backup = (await engine.backups(data, config.root)).find((b) => b.id === id);
    if (!backup) throw Error('Backup not found for this game installation.');
    const installed = (await engine.inventory(config.root, data)).addons.find(
      (a) => a.id === backup.addonId,
    );
    await addonActivity.track(
      data,
      config.root,
      installed || { id: backup.addonId, name: backup.name },
      'restore',
      async () => {
        await engine.restore(config.root, data, id);
        return { version: backup.version };
      },
    );
    return state();
  });
  handle('chooseRoot', async () => {
    const r = await dialog.showOpenDialog(win, {
      title: 'Select your WoW game folder',
      defaultPath: config.root,
      buttonLabel: 'Select folder',
      properties: ['openDirectory'],
    });
    if (!r.canceled && r.filePaths[0]) {
      config = await clients.select(config, { root: r.filePaths[0] });
      releases = {};
      send({});
      await persist();
    }
    return state();
  });
  handle('setAccount', async (value) => {
    if (!(await settings.accounts(config.root)).includes(value))
      throw Error('Unknown local account.');
    config = profileContext.accountChanged(config, value);
    await persist();
    return state();
  });
  require('./sharing-handlers.cjs').register({
    handle,
    runtime,
    state,
    persist,
    send,
    dialog,
    window: () => win,
    busy: () => taskActive,
  });
  handle('snapshot', async () => {
    if (await engine.gameRunning())
      throw Error('Close WoW so its latest settings are saved first.');
    const s = await settings.capture(config.root, config.account, data);
    return { files: s.files, bytes: s.bytes, hash: s.hash };
  });
  handle('openSource', async (id) => {
    const a = (await engine.definitions(config.root, data)).find((a) => a.id === id);
    if (a?.sourceUrl) return shell.openExternal(a.sourceUrl);
    if (a?.repo) return shell.openExternal('https://github.com/' + a.repo);
    if (a?.curseId)
      return shell.openExternal(
        'https://www.curseforge.com/wow/addons/' +
          (id === 'mrt' ? 'method-raid-tools' : 'handynotes'),
      );
  });
  handle('openBackups', async () => {
    await fs.mkdir(path.join(data, 'backups'), { recursive: true });
    return shell.openPath(path.join(data, 'backups'));
  });
  win = new BrowserWindow({
    show: !process.argv.includes('--background'),
    width: 1250,
    height: 840,
    minWidth: 980,
    minHeight: 680,
    title: 'Astral',
    backgroundColor: '#101114',
    titleBarStyle: 'hiddenInset',
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      additionalArguments: [
        '--astral-commands=' +
          encodeURIComponent(JSON.stringify(Object.keys(require('../shared/commands.json')))),
      ],
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
    },
  });
  win.on('close', (event) => {
    if (!app.isQuitting) {
      event.preventDefault();
      win.hide();
    }
  });
  const icon = nativeImage
    .createFromPath(path.join(__dirname, '../assets/icon.png'))
    .resize({ width: 18, height: 18 });
  tray = new Tray(icon);
  tray.setToolTip('Astral');
  tray.setContextMenu(
    Menu.buildFromTemplate([
      {
        label: 'Open Astral',
        click: () => {
          win.show();
          win.focus();
        },
      },
      { type: 'separator' },
      { label: 'Quit Astral', click: () => app.quit() },
    ]),
  );
  tray.on('click', () => {
    win.show();
    win.focus();
  });
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.webContents.on('will-navigate', (e, url) => {
    if (!url.startsWith('file://') && !url.startsWith('http://127.0.0.1:5173/')) e.preventDefault();
  });
  win.webContents.session.setPermissionRequestHandler((_, __, cb) => cb(false));
  if (process.argv.includes('--dev')) await win.loadURL('http://127.0.0.1:5173');
  else await win.loadFile(path.join(__dirname, '../dist/index.html'));
  await acknowledgeUpdate();
  backgroundJobs = require('./background-jobs.cjs').create({
    configuration: () => config,
    updates: appUpdates,
    gameRunning: engine.gameRunning,
    hasSession: session.exists,
    state,
    changed: () => win?.webContents.send('astral:changed'),
    syncReady: signalSyncReady,
    daily,
    busy: () => taskActive,
    failed: (job, error) => {
      if (job === 'daily') lastError = error.message;
      else console.warn('Astral background ' + job + ':', error.message);
    },
  });
  await backgroundJobs.start();
}
if (!app.requestSingleInstanceLock({ scheduled: scheduledLaunch, updateAck })) app.quit();
else {
  app.on('second-instance', (_event, args, _directory, extra) => {
    if (extra?.scheduled || args.includes('--daily-update')) {
      if (config && !taskActive)
        daily().catch((error) => {
          lastError = error.message;
        });
      return;
    }
    if (/^[a-f0-9]{48}$/.test(extra?.updateAck || '')) updateAck = extra.updateAck;
    if (!win) foregroundRequested = true;
    else {
      win.show();
      win.focus();
      acknowledgeUpdate().catch(() => {});
    }
  });
  app
    .whenReady()
    .then(async () => {
      if (!scheduledLaunch) return main();
      app.dock?.hide();
      await initialize();
      try {
        await daily();
      } catch (error) {
        console.error('Astral daily update failed:', error.message);
      }
      if (foregroundRequested) {
        await app.dock?.show();
        return main();
      }
      app.quit();
    })
    .catch((e) => {
      if (!scheduledLaunch) dialog.showErrorBox('Astral could not start', e.message);
      else console.error('Astral scheduled update could not start:', e.message);
      app.quit();
    });
  app.on('before-quit', () => {
    app.isQuitting = true;
    backgroundJobs?.stop();
  });
  app.on('activate', () => {
    win?.show();
    win?.focus();
  });
  app.on('window-all-closed', () => app.quit());
}
