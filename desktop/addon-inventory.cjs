const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const catalog = require('./catalog.cjs');
const github = require('./github.cjs');
const { exists, json, save } = require('./storage.cjs');
const { toc, preferredToc } = require('./addon-metadata.cjs');
const { flavor, allowed: allowedFlavor } = require('./flavors.cjs');
const { icon: addonIcon } = require('./icons.cjs');
function validDefinition(a) {
  return (
    a &&
    /^gh-[a-f0-9]{16}$/.test(a.id) &&
    github.validRepo(a.repo) &&
    /^[A-Za-z0-9_-]+$/.test(a.folder || '') &&
    Array.isArray(a.folders) &&
    a.folders.length > 0 &&
    a.folders.length <= 64 &&
    a.folders.includes(a.folder) &&
    a.folders.every((n) => /^[A-Za-z0-9_-]+$/.test(n))
  );
}
function registryPath(root, data) {
  return path.join(
    data,
    'managed',
    crypto.createHash('sha256').update(path.resolve(root)).digest('hex').slice(0, 16) + '.json',
  );
}
async function definitions(root, data) {
  if (!data) return catalog;
  const extra = await json(registryPath(root, data), []);
  return [...catalog, ...(Array.isArray(extra) ? extra.filter(validDefinition) : [])];
}
async function register(root, data, definition) {
  if (!validDefinition(definition)) throw Error('Invalid addon source definition.');
  const file = registryPath(root, data),
    prior = await json(file, []);
  await save(file, [...prior.filter((a) => a.id !== definition.id), definition]);
}
async function inventory(root, data) {
  const dir = path.join(root, 'Interface', 'AddOns');
  const entries = await fs.readdir(dir, { withFileTypes: true }).catch(() => []);
  const folders = entries.filter((e) => e.isDirectory()).map((e) => e.name);
  const groups = [];
  const claimed = new Set();
  for (const item of await definitions(root, data)) {
    if (!folders.includes(item.folder)) continue;
    const family = folders.filter(
      (n) =>
        item.folders?.includes(n) ||
        n === item.folder ||
        (item.prefix && n.startsWith(item.prefix + '_')) ||
        (item.id === 'ellesmere' && n.startsWith('EllesmereUI')),
    );
    family.forEach((n) => claimed.add(n));
    const files = await fs.readdir(path.join(dir, item.folder));
    const file =
      preferredToc(files, item.folder, flavor(root)) || files.find((file) => file.endsWith('.toc'));
    const meta = file ? toc(await fs.readFile(path.join(dir, item.folder, file), 'utf8')) : {};
    if (item.versionCompanion) {
      const companion = path.join(dir, item.versionCompanion);
      let companionToc;
      for (const name of [item.versionCompanion + '_Camelot.toc', item.versionCompanion + '.toc']) {
        if (await exists(path.join(companion, name))) {
          companionToc = name;
          break;
        }
      }
      const companionMeta = companionToc
        ? toc(await fs.readFile(path.join(companion, companionToc), 'utf8'))
        : {};
      meta.Version = (meta.Version || 'Unknown') + '+v' + (companionMeta.Version || '0');
    }
    groups.push({
      ...item,
      installed: true,
      canImport: !item.local && !item.bundled && allowedFlavor(item, flavor(root)),
      version: meta.Version || 'Unknown',
      author: meta.Author || '',
      icon: await addonIcon(root, meta.IconTexture),
      modules: family,
      source: item.bundled
        ? 'Astral'
        : item.repo
          ? 'GitHub'
          : item.local
            ? 'Local development'
            : 'Manual',
      status: item.repo || item.bundled ? 'unchecked' : item.local ? 'local' : 'manual',
    });
  }
  for (const name of folders.filter((f) => !claimed.has(f))) {
    const files = await fs.readdir(path.join(dir, name));
    const file = files.find((f) => f === name + '.toc') || files.find((f) => f.endsWith('.toc'));
    const meta = file ? toc(await fs.readFile(path.join(dir, name, file), 'utf8')) : {};
    groups.push({
      id: 'local:' + name,
      name: (meta.Title || name).replace(/\|c[0-9a-f]{8}|\|r/gi, ''),
      folder: name,
      icon: await addonIcon(root, meta.IconTexture),
      version: meta.Version || 'Unknown',
      installed: true,
      modules: [name],
      source: 'Manual',
      status: 'manual',
      category: 'Other',
      mark: name.slice(0, 2),
      color: 'slate',
    });
  }
  return { addons: groups, folderCount: folders.length };
}
module.exports = { catalog, definitions, register, inventory, validDefinition };
