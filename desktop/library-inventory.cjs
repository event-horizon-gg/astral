const inventory = require('./addon-inventory.cjs');
const { validate } = require('./library-data.cjs');
async function installed(root, data, api = inventory) {
  return validate(
    (await api.inventory(root, data)).addons.map(({ folder, name }) => ({ folder, name })),
  );
}
async function shareable(root, data, api = inventory) {
  return (await installed(root, data, api))
    .filter((a) => /^[a-z0-9_-]{1,100}$/i.test(a.folder))
    .map((a) => ({ ...a, name: a.name.trim().slice(0, 100) || a.folder }));
}
module.exports = { installed, shareable };
