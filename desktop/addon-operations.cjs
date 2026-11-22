const engine = require('./engine.cjs');
const activity = require('./addon-activity.cjs');
const { newer } = require('./addon-versions.cjs');
const talents = require('./talent-builds.cjs');

async function execute({
  addon,
  root,
  data,
  release,
  fresh = true,
  versionPolicy = 'replace',
  verb = 'Updating',
  action = 'update',
  onProgress = () => {},
  api = engine,
  track = activity.track,
  refreshTalents = talents.refresh,
  now = Date.now(),
}) {
  let status = 'updated',
    refreshed = false;
  const result = await track(data, root, addon, action, async () => {
    if (await api.gameRunning())
      throw Object.assign(Error('WoW started. Close it before continuing installation.'), {
        code: 'WOW_RUNNING',
      });
    onProgress({ id: addon.id, kind: 'update', percent: 0, message: 'Checking ' + addon.name });
    release ||= await api.getRelease(addon, data, fresh);
    const upgrade = versionPolicy === 'replace' ? true : newer(addon, release);
    if (upgrade === null && versionPolicy === 'newer') {
      status = 'review';
      return { unchanged: true, version: addon.version };
    }
    if (upgrade === false) {
      status = 'current';
      refreshed = addon.id === 'talents' && (await refreshTalents(root, api, now));
      if (refreshed) status = 'updated';
      return { unchanged: !refreshed, version: addon.version };
    }
    const installed = await api.install(addon, root, data, release, (percent) =>
      onProgress({ id: addon.id, kind: 'update', percent, message: verb + ' ' + addon.name }),
    );
    if (addon.id === 'talents') await refreshTalents(root, api, now);
    return installed;
  });
  return { release, result, status, refreshed };
}
module.exports = { execute };
