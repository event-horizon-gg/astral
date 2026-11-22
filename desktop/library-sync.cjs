const { installed, shareable } = require('./library-inventory.cjs');
const { validate, merge, required } = require('./library-data.cjs');
const crypto = require('node:crypto');
const engine = require('./engine.cjs');
const activity = require('./addon-activity.cjs');
const operations = require('./addon-operations.cjs');
const { allowed, flavor } = require('./flavors.cjs');
async function plan({ root, data, shared = [], target = [], api = engine }) {
  validate(shared);
  validate(target);
  const definitions = await api.definitions(root, data),
    inventory = await api.inventory(root, data);
  const folders = new Set(
    inventory.addons.flatMap((a) => a.modules || [a.folder]).map((name) => name.toLowerCase()),
  );
  const queue = [],
    manual = [],
    seen = new Set(),
    visiting = new Set();
  const definition = (folder) =>
    definitions.find((a) => a.folder.toLowerCase() === folder.toLowerCase());
  function visit(item, required) {
    const key = item.folder.toLowerCase();
    if (folders.has(key) || seen.has(key)) return;
    const addon = definition(item.folder);
    if ((!addon?.repo && !addon?.bundled) || !allowed(addon, flavor(root))) {
      if (required) {
        manual.push({
          folder: item.folder,
          name: addon?.name || item.name,
          id: addon?.id || null,
          reason:
            addon && !allowed(addon, flavor(root))
              ? 'No compatible source for this game client.'
              : 'A manual install or source review is required.',
        });
        seen.add(key);
      }
      return;
    }
    if (visiting.has(key)) throw Error('The addon collection contains a dependency cycle.');
    visiting.add(key);
    for (const folder of addon.requires || [])
      visit({ folder, name: definition(folder)?.name || folder }, true);
    visiting.delete(key);
    seen.add(key);
    queue.push({ id: addon.id, name: addon.name, folder: addon.folder });
  }
  for (const item of shared) visit(item, false);
  for (const item of target) visit(item, true);
  return {
    queue,
    manual,
    fingerprint: crypto
      .createHash('sha256')
      .update(
        JSON.stringify({
          queue,
          manual,
          installed: inventory.addons.map((a) => ({
            id: a.id,
            version: a.version,
            modules: a.modules,
          })),
        }),
      )
      .digest('hex'),
  };
}
async function install({
  root,
  data,
  plan,
  onProgress = () => {},
  afterInstall = async () => {},
  api = engine,
  track = activity.track,
}) {
  if (!plan.queue.length) return [];
  if (await api.gameRunning()) throw Error('Close WoW before installing synced addons.');
  const done = [];
  for (const item of plan.queue) {
    if (await api.gameRunning()) throw Error('WoW started. Settings sync is paused.');
    const addon = (await api.definitions(root, data)).find((a) => a.id === item.id);
    if ((!addon?.repo && !addon?.bundled) || !allowed(addon, flavor(root)))
      throw Error('The addon source changed. Review sync again.');
    try {
      await operations.execute({ addon, root, data, api, track, onProgress, verb: 'Installing' });
      await afterInstall();
      done.push(addon.name);
    } catch (error) {
      throw Error(addon.name + ': ' + error.message + ' Settings have not been applied.');
    }
  }
  return done;
}
async function publish({ request, route, local }) {
  for (let attempt = 0; attempt < 2; attempt++) {
    const current = await request(route),
      addons = merge(current.addons || [], local);
    if (JSON.stringify(addons) === JSON.stringify(merge(current.addons || []))) return current;
    try {
      return await request(route, {
        method: 'PUT',
        body: { addons, revision: current.revision || 0 },
      });
    } catch (error) {
      if (!error.conflict || attempt) throw error;
    }
  }
}
module.exports = {
  validate,
  merge,
  installed,
  shareable,
  required,
  plan,
  install,
  publish,
};
