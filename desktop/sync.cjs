const engine = require('./engine.cjs');
const settings = require('./settings.cjs');
const packages = require('./library-sync.cjs');
const setup = require('./setup-application.cjs');
const policy = require('./profile-policy.cjs');
const { encode } = require('./setup-format.cjs');
const { flavor } = require('./flavors.cjs');
const { syncAction } = require('./sync-state.cjs');
const comparison = require('./setup-comparison.cjs');
const os = require('node:os');
async function run({
  root,
  account,
  data,
  scope,
  profile = 'regular',
  lastHash,
  lastRemoteHash,
  choice,
  expectedRevision,
  expectedLocalHash,
  device,
  request,
  onProgress = () => {},
  api = engine,
  store = settings,
  library = packages,
}) {
  if (await api.gameRunning()) throw Error('Sync waits until WoW is closed.');
  const route = '/profiles/' + scope + '/current';
  const libraryRoute = '/profiles/' + flavor(root) + '/library';
  let local = account ? await store.capture(root, account, data) : null;
  if (expectedLocalHash !== undefined && expectedLocalHash !== local?.hash)
    throw Error('This computer’s setup changed. Sync again to review it.');
  const remote = local ? await request(route) : null;
  if (remote && expectedRevision !== undefined && expectedRevision !== remote.revision)
    throw Error('The cloud backup changed. Sync again to review it.');
  if (choice === 'cloud' && !remote?.revision) throw Error('No cloud backup is available.');
  const action = local
    ? syncAction(local, remote, lastHash, choice, lastRemoteHash)
    : 'library';
  if (action === 'conflict') {
    if (remote.hash && store.inspect(remote.payload).hash !== remote.hash)
      throw Error('The cloud backup checksum does not match.');
    const current = store.decode(local.payload);
    const effective = policy.apply(store.decode(remote.payload), {
      root,
      profile,
      inventory: await api.inventory(root, data),
      currentInput: current.input,
    });
    return {
      conflict: true,
      scope,
      revision: remote.revision,
      localHash: local.hash,
      context: { root, account, profile },
      localFiles: local.files,
      remoteTime: remote.updated,
      comparison: {
        profile,
        client: flavor(root),
        local: {
          device: os.hostname(),
          files: local.files,
          capturedAt: new Date().toISOString(),
        },
        remote: {
          device: remote.deviceLabel || null,
          updated: remote.updated || null,
          revision: remote.revision,
        },
        rows: comparison.compare(current, effective),
      },
    };
  }
  let target;
  if (local) {
    if (action === 'download') {
      if (remote.hash && store.inspect(remote.payload).hash !== remote.hash)
        throw Error('The cloud backup checksum does not match.');
      target = store.decode(remote.payload);
    } else target = store.decode(local.payload);
    target = policy.apply(target, {
      root,
      profile,
      inventory: await api.inventory(root, data),
      currentInput: store.decode(local.payload).input,
    });
  }
  // Migrate the selected profile's older explicit list into the per-client
  // collection. Additions merge; switching profile never removes packages.
  const legacy =
    scope !== flavor(root) ? await request('/profiles/' + scope + '/library') : null;
  const shared = await library.publish({
    request,
    route: libraryRoute,
    local: library.merge(await library.shareable(root, data, api), legacy?.addons || []),
  });
  if (!local) return { libraryOnly: true, libraryRevision: shared.revision };
  const plan = await library.plan({
    root,
    data,
    shared: shared.addons || [],
    target: library.required(target.library || [], target.addonSelection),
    api,
  });
  await setup.installPackages({
    root,
    account,
    data,
    plan,
    current: local,
    api,
    store,
    library,
    onProgress,
  });
  if (plan.manual.length)
    return {
      incomplete: true,
      scope,
      missing: plan.manual,
      installed: plan.queue.map((a) => a.name),
    };
  if (await api.gameRunning()) throw Error('WoW started. Settings sync is paused.');
  if (remote.revision) {
    const latest = await request(route);
    if (latest.revision !== remote.revision || latest.hash !== remote.hash)
      throw Error('The cloud backup changed during installation. Sync again.');
  }
  const encoded = encode(target);
  if (encoded.hash !== local.hash || plan.queue.length || action === 'download')
    await store.apply(root, account, encoded.payload, data, api.gameRunning);
  local = await store.capture(root, account, data);
  let revision = remote.revision || 0,
    remoteHash = remote.hash || null;
  if (
    action === 'upload' ||
    (action === 'unchanged' && local.hash !== lastHash && local.hash !== remote.hash)
  ) {
    const result = await request(route, {
      method: 'PUT',
      body: { payload: local.payload, hash: local.hash, files: local.files, revision, device },
    });
    revision = result.revision;
    remoteHash = local.hash;
  }
  await library.publish({
    request,
    route: libraryRoute,
    local: await library.shareable(root, data, api),
  });
  return {
    lastHash: local.hash,
    lastRemoteHash: remoteHash,
    syncRevision: revision,
    lastSync: new Date().toISOString(),
    syncProfilePending: false,
    installed: plan.queue.map((a) => a.name),
  };
}
async function collection({
  root,
  account,
  data,
  request,
  onProgress = () => {},
  api = engine,
}) {
  if (await api.gameRunning()) throw Error('Sync waits until WoW is closed.');
  const route = '/profiles/' + flavor(root) + '/library';
  const shared = await packages.publish({
    request,
    route,
    local: await packages.shareable(root, data, api),
  });
  if (!account) return { libraryOnly: true };
  const before = await settings.capture(root, account, data);
  const plan = await packages.plan({ root, data, shared: shared.addons || [], api });
  await setup.installPackages({ root, account, data, plan, current: before, api, onProgress });
  await packages.publish({ request, route, local: await packages.shareable(root, data, api) });
  return {
    beforeHash: before.hash,
    afterHash: (await settings.capture(root, account, data)).hash,
  };
}
module.exports = { run, collection };
