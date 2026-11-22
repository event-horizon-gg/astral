const path = require('node:path');
const engine = require('./engine.cjs');
const activity = require('./addon-activity.cjs');
const operations = require('./addon-operations.cjs');
const { flavor } = require('./flavors.cjs');
const FOREVER = [
  'ellesmere',
  'dialogueui',
  'narcissus',
  'keyui',
  'handynotes',
  'questie',
  'dbm',
  'dbm-dungeons',
  'dbm-raids',
];
async function install({
  id,
  root,
  data,
  api = engine,
  track = activity.track,
  onProgress = () => {},
}) {
  if (id !== 'forever' || flavor(root) !== 'forever')
    throw Error('Select a Forever installation for this suite.');
  if (!(await api.exists(path.join(root, 'Interface', 'AddOns'))))
    throw Error('Select your Forever installation folder first.');
  if (await api.gameRunning()) throw Error('Close WoW before installing the Forever suite.');
  const inventory = await api.inventory(root, data);
  const present = new Set(inventory.addons.map((addon) => addon.id));
  const pending = FOREVER.filter((id) => !present.has(id));
  const result = {
    installed: [],
    existing: FOREVER.filter((id) => present.has(id)),
    failed: null,
    pending: [],
  };
  for (const [index, id] of pending.entries()) {
    const addon = api.catalog.find((addon) => addon.id === id);
    try {
      if (!addon?.repo) throw Error('The suite contains an unavailable source.');
      await operations.execute({ addon, root, data, api, track, onProgress, verb: 'Installing' });
      result.installed.push(id);
    } catch (error) {
      result.failed = { id, name: addon?.name || id, message: error.message };
      result.pending = pending.slice(index + 1);
      break;
    }
  }
  return result;
}
module.exports = { FOREVER, install };
