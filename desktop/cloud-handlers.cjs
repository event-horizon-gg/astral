const path = require('node:path');
const engine = require('./engine.cjs');
const storage = require('./storage.cjs');
const settings = require('./settings.cjs');
const flavors = require('./flavors.cjs');
const localProfiles = require('./local-profiles.cjs');
const setupSync = require('./sync.cjs');
const syncedPackages = require('./library-sync.cjs');
const profilePolicy = require('./profile-policy.cjs');
const profileContext = require('./profile-context.cjs');
const clients = require('./clients.cjs');
const format = require('./setup-format.cjs');
const reviews = require('./reviews.cjs');
function register({ handle, runtime, state, persist, send, session, shell, busy }) {
  const data = runtime.data,
    request = session.request;
  const validSyncProfile = profileContext.validId;
  const syncProfile = () => profileContext.profileId(runtime.config);
  const syncScope = () => profileContext.syncScope(runtime.config);
  const profilePreviews = reviews.create({
    context: () => profileContext.selected(runtime.config),
    ttl: 300000,
    message: 'The setup changed. Review the profile again.',
  });
  handle('cloudInfo', async () => request('/cloud/' + syncScope()));
  const libraryPath = () => '/profiles/' + flavors.flavor(runtime.config.root) + '/library';
  handle('sharedLibrary', () => request(libraryPath()));
  handle('shareLibrary', async (revision) => {
    if (!(await storage.exists(path.join(runtime.config.root, 'Interface', 'AddOns'))))
      throw Error('Select your WoW installation folder first.');
    const { addons } = await engine.inventory(runtime.config.root, data);
    const seen = new Set();
    const library = [];
    for (const addon of addons) {
      const key = addon.folder.toLowerCase();
      if (seen.has(key) || !/^[a-z0-9_-]{1,100}$/.test(key)) continue;
      seen.add(key);
      library.push({ folder: addon.folder, name: addon.name.trim().slice(0, 100) || addon.folder });
    }
    return request(libraryPath(), { method: 'PUT', body: { addons: library, revision } });
  });
  const syncProfilesPath = () => '/sync-profiles/' + flavors.flavor(runtime.config.root);
  handle('syncProfiles', () => request(syncProfilesPath()));
  handle('createSyncProfile', (name) =>
    request(syncProfilesPath(), { method: 'POST', body: { name } }),
  );
  handle('renameSyncProfile', (id, name) => {
    if (!validSyncProfile(id)) throw Error('Unknown sync profile.');
    return request(syncProfilesPath() + '/' + id, { method: 'PATCH', body: { name } });
  });
  handle('deleteSyncProfile', (id) => {
    if (!profileContext.isCustom(id)) throw Error('Unknown custom profile.');
    if (id === syncProfile()) throw Error('Switch to another profile before deleting this one.');
    return request(syncProfilesPath() + '/' + id, { method: 'DELETE', body: { confirm: id } });
  });
  handle('setSyncProfile', async (value) => {
    if (!validSyncProfile(value)) throw Error('Unknown sync profile.');
    if (value === syncProfile()) return state();
    throw Error('Review the profile change before switching.');
  });
  handle('previewProfile', async (value, edits = {}) => {
    if (busy()) throw Error('Wait until the current operation finishes.');
    if (!validSyncProfile(value)) throw Error('Unknown sync profile.');
    if (await engine.gameRunning()) throw Error('Close WoW before changing profiles.');
    if (
      !['regular', 'handheld'].includes(value) &&
      value !== syncProfile() &&
      !(await localProfiles.read(data, runtime.config.root, runtime.config.account, value))
    ) {
      const { profiles } = await request(syncProfilesPath());
      if (!profiles.some((p) => p.id === value)) throw Error('Unknown sync profile.');
    }
    const reviewContext = profilePreviews.capture();
    const from = syncProfile(),
      root = runtime.config.root,
      account = runtime.config.account;
    const plan = await localProfiles.prepare({ data, root, account, from, to: value, edits });
    if (
      runtime.config.root !== root ||
      runtime.config.account !== account ||
      syncProfile() !== from
    )
      throw Error('The selected setup changed. Review the profile again.');
    const token = await profilePreviews.replace(
      { root, account, from, to: value, edits, fingerprint: plan.fingerprint },
      reviewContext,
    );
    return {
      token,
      target: value,
      initialized: plan.initialized,
      changes: plan.changes,
      changedSettings: plan.changedSettings,
      packages: plan.packages.queue,
      handheld: value === 'handheld' ? flavors.flavor(runtime.config.root) : null,
      missing: plan.packages.manual,
    };
  });
  handle('applyProfile', (token) =>
    profilePreviews.run(token, async (preview) => {
      const previous = structuredClone(runtime.config);
      await localProfiles.apply({
        data,
        ...preview,
        expected: preview.fingerprint,
        onProgress: send,
        running: engine.gameRunning,
        commit: async () => {
          if (preview.to !== preview.from) {
            runtime.config = profileContext.localChanges(runtime.config, preview.to);
          }
          try {
            await persist();
          } catch (error) {
            runtime.config = previous;
            throw error;
          }
        },
      });
      return state();
    }),
  );
  handle('revokeDevice', async (id) => {
    if (typeof id !== 'string' || !id.match(/^[A-Za-z0-9_-]{32,64}$/))
      throw Error('Invalid device.');
    const result = await request('/devices/' + id, { method: 'DELETE' });
    if (result.current) await session.clear();
    return state();
  });
  handle('restoreCloud', async (id, revision) => {
    if (await engine.gameRunning()) throw Error('Close WoW before restoring settings.');
    if (
      typeof id !== 'string' ||
      !id.match(/^[A-Za-z0-9_-]{43}$/) ||
      !Number.isSafeInteger(revision)
    )
      throw Error('Invalid cloud backup.');
    const root = runtime.config.root,
      account = runtime.config.account,
      type = syncScope();
    const old = await request('/profiles/' + type + '/history/' + id);
    const verified = format.inspect(old.payload);
    if (old.hash && old.hash !== verified.hash)
      throw Error('The cloud backup checksum does not match.');
    const captured = await settings.capture(root, account, data);
    const current = format.decode(captured.payload);
    const target = profilePolicy.apply(format.decode(old.payload), {
      root,
      profile: syncProfile(),
      inventory: await engine.inventory(root, data),
      currentInput: current.input,
    });
    const plan = await syncedPackages.plan({
      root,
      data,
      target: syncedPackages.required(target.library || [], target.addonSelection),
    });
    if (plan.manual.length)
      throw Error(
        'Connect or install these addons before restoring: ' +
          plan.manual.map((a) => a.name).join(', '),
      );
    await require('./setup-application.cjs').installPackages({
      root,
      account,
      data,
      plan,
      current: captured,
      onProgress: send,
    });
    send({});
    if (await engine.gameRunning()) throw Error('WoW started. Restore deferred.');
    const result = await request('/profiles/' + type + '/history/' + id + '/restore', {
      method: 'POST',
      body: { revision, hash: verified.hash },
    });
    await settings.apply(root, account, format.encode(target).payload, data, engine.gameRunning);
    runtime.config.syncProfilePending = false;
    runtime.config.lastHash = (await settings.capture(root, account, data)).hash;
    runtime.config.lastRemoteHash = verified.hash;
    runtime.config.syncRevision = result.revision;
    runtime.config.lastSync = new Date().toISOString();
    await persist();
    return state();
  });
  handle('deleteCloud', async (confirm) => {
    if (confirm !== 'DELETE') throw Error('Type DELETE to remove your cloud data.');
    await request('/account/data', { method: 'DELETE', body: { confirm } });
    await session.clear();
    runtime.config = clients.resetSync(runtime.config);
    await persist();
    return state();
  });
  handle('connect', async () => {
    await session.connect((url) => shell.openExternal(url));
    if (!runtime.config.syncProfilePending && !(await engine.gameRunning()))
      send({ kind: 'sync-ready', message: 'Battle.net connected. Preparing your setup.' });
    return state();
  });
  handle('sync', async (choice, expectedScope, expectedRevision) => {
    const scope = syncScope();
    if (expectedScope && expectedScope !== scope)
      throw Error('The sync profile changed. Try again.');
    if (choice === 'auto' && runtime.config.syncProfilePending) return state();
    if (await engine.gameRunning()) throw Error('Settings sync waits until WoW is closed.');
    try {
      for (const type of ['retail', 'forever']) {
        const otherRoot = path.join(
          path.dirname(runtime.config.root),
          type === 'retail' ? '_retail_' : '_forever_',
        );
        if (
          otherRoot === runtime.config.root ||
          !(await storage.exists(path.join(otherRoot, 'Interface', 'AddOns')))
        )
          continue;
        const saved = runtime.config.spaces?.[type];
        const accounts = await settings.accounts(otherRoot);
        const account =
          saved?.root === otherRoot && accounts.includes(saved.account)
            ? saved.account
            : accounts.length === 1
              ? accounts[0]
              : null;
        const result = await setupSync.collection({
          root: otherRoot,
          account,
          data,
          request,
          onProgress: send,
        });
        if (
          saved?.root === otherRoot &&
          saved.account === account &&
          saved.lastHash === result.beforeHash
        )
          saved.lastHash = result.afterHash;
      }
      const result = await setupSync.run({
        root: runtime.config.root,
        account: runtime.config.account,
        data,
        scope,
        profile: syncProfile(),
        lastHash: runtime.config.lastHash,
        lastRemoteHash: runtime.config.lastRemoteHash,
        choice,
        expectedRevision,
        device: runtime.config.device,
        request,
        onProgress: send,
      });
      if (result.conflict) return result;
      if (result.incomplete) return { ...result, state: await state() };
      if (result.libraryOnly)
        return {
          ...(await state()),
          syncMessage:
            'Addon collection shared. Open WoW once, then close it to sync this client’s settings.',
        };
      const { installed, ...updates } = result;
      Object.assign(runtime.config, updates);
      await persist();
    } finally {
      send({});
    }
    return state();
  });
  handle('disconnect', async () => {
    await session.disconnect();
    return state();
  });
}
module.exports = { register };
