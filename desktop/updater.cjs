const storage = require('./storage.cjs');
const path = require('node:path');
const engine = require('./engine.cjs');
const talentBuilds = require('./talent-builds.cjs');
const { allowed, flavor } = require('./flavors.cjs');
const DAY = 24 * 60 * 60 * 1000;
const operations = require('./addon-operations.cjs');
async function status(data) {
  return storage.json(path.join(data, 'daily-updates.json'), { nextAt: 0, runs: [] });
}
async function run({
  data,
  roots,
  ignored = {},
  legacyRio = false,
  force = false,
  now = Date.now(),
  onProgress = () => {},
  api = engine,
  refreshTalents = talentBuilds.refresh,
}) {
  const previous = await status(data);
  if (!force && now < (previous.nextAt || 0)) return previous;
  if (await api.gameRunning())
    return { ...previous, deferred: true, reason: 'Waiting for WoW to close.' };
  const report = { time: new Date(now).toISOString(), results: [], status: 'complete' };
  let talentRefreshFailed = false;
  for (const root of [...new Set(roots)]) {
    const type = flavor(root),
      inv = await api.inventory(root, data);
    for (const addon of inv.addons) {
      if (
        (!addon.repo && !addon.bundled) ||
        !allowed(addon, type) ||
        ignored[type]?.includes(addon.id)
      )
        continue;
      if (legacyRio && addon.id === 'rio') {
        report.results.push({
          id: addon.id,
          name: addon.name,
          client: type,
          status: 'external',
          message: 'Handled by the existing daily Raider.IO updater on this Mac.',
        });
        continue;
      }
      if (await api.gameRunning()) {
        report.status = 'deferred';
        break;
      }
      try {
        const outcome = await operations.execute({
          addon,
          root,
          data,
          api,
          onProgress,
          versionPolicy: 'newer',
          refreshTalents,
          now,
          // Daily outcomes already have their own retained report.
          track: (_data, _root, _addon, _action, operation) => operation(),
        });
        const release = outcome.release;
        if (outcome.status === 'review') {
          report.results.push({
            id: addon.id,
            name: addon.name,
            client: type,
            status: 'review',
            message: 'Version format needs a manual review.',
          });
          continue;
        }
        if (outcome.status === 'current' || outcome.refreshed) {
          const refreshed = outcome.refreshed;
          report.results.push({
            id: addon.id,
            name: addon.name,
            client: type,
            status: refreshed ? 'updated' : 'current',
            ...(refreshed ? { message: 'Druid talent builds refreshed.' } : {}),
          });
          continue;
        }
        report.results.push({
          id: addon.id,
          name: addon.name,
          client: type,
          status: 'updated',
          from: addon.version,
          to: release.version,
          notes: release.notes || '',
        });
      } catch (e) {
        report.results.push({
          id: addon.id,
          name: addon.name,
          client: type,
          status: 'error',
          message: e.message,
        });
        if (e.code === 'WOW_RUNNING') {
          report.status = 'deferred';
          break;
        }
        if (addon.id === 'talents') {
          talentRefreshFailed = true;
          continue;
        }
        report.status = 'attention';
        if (e.code === 'RATE_LIMIT') {
          report.status = 'attention';
          break;
        }
      }
    }
    if (report.status !== 'complete') break;
  }
  if (talentRefreshFailed && report.status === 'complete') report.status = 'attention';
  const result = {
    nextAt:
      now + (report.status === 'deferred' ? 60000 : report.status === 'attention' ? 3600000 : DAY),
    runs: [report, ...(previous.runs || [])].slice(0, 14),
  };
  await storage.save(path.join(data, 'daily-updates.json'), result);
  return result;
}
module.exports = { status, run };
