const fs = require('node:fs/promises');
const path = require('node:path');
const { flavor, interfaceFor } = require('./flavors.cjs');
const { validateFiles } = require('./toc-files.cjs');
function toc(text) {
  const result = {};
  for (const line of text.split(/\r?\n/)) {
    const m = line.match(/^##\s*([^:]+):\s*(.*)/);
    if (m) result[m[1].trim()] = m[2].trim();
  }
  return result;
}
function allowedFolder(addon, n) {
  if (addon.folders) return addon.folders.includes(n);
  if (addon.id === 'ellesmere') return n.startsWith('EllesmereUI');
  if (addon.id === 'rio') return /^RaiderIO(?:_DB_(?:EU|US|KR|TW|CN)_[MRF])?$/.test(n);
  return n === addon.folder || (!!addon.prefix && n.startsWith(addon.prefix + '_'));
}
async function targetInterface(root) {
  const build = await fs.readFile(path.join(root, '..', '.build.info'), 'utf8').catch(() => null);
  if (!build) return interfaceFor(flavor(root));
  const lines = build.trim().split(/\r?\n/),
    keys = lines[0].split('|').map((h) => h.split('!')[0]);
  const version = lines
    .slice(1)
    .map((l) => Object.fromEntries(keys.map((k, i) => [k, l.split('|')[i]])))
    .find(
      (r) => r.Product === (flavor(root) === 'forever' ? 'wowf' : 'wow') && r.Active === '1',
    )?.Version;
  if (!version) throw Error('Could not identify the active WoW build.');
  const p = version.split('.').map(Number);
  return String(p[0] * 10000 + p[1] * 100 + p[2]);
}

function preferredToc(files, folder, client) {
  return [
    ...(client === 'forever' ? [folder + '_Camelot.toc'] : []),
    folder + '_Mainline.toc',
    folder + '.toc',
  ].find((name) => files.includes(name));
}
function packageNames(addon, names, client) {
  if (addon?.id === 'ellesmere')
    return names.filter((name) =>
      client === 'forever'
        ? ![
            'EllesmereUIMythicTimer',
            'EllesmereUIFriends',
            'EllesmereUILoadoutManager',
            'EllesmereUIGroupFinder',
          ].includes(name)
        : name !== 'EllesmereUIForeverEssentials',
    );
  if (addon?.id === 'rio')
    return names.filter(
      (name) =>
        ['RaiderIO', 'RaiderIO_DB_EU_M', 'RaiderIO_DB_EU_R', 'RaiderIO_DB_EU_F'].includes(name) ||
        addon.modules?.includes(name),
    );
  return names;
}
function supports(meta, target, addon, client) {
  const values = (meta.Interface || '').split(/[,\s]+/);
  return (
    values.includes(target) ||
    (addon?.allowRetailPatchMismatch &&
      client === 'retail' &&
      values.some((value) => /^\d{6}$/.test(value) && value.slice(0, -2) === target.slice(0, -2)))
  );
}
async function inspectPackage({ unpacked, root, addon, folders }) {
  const client = flavor(root),
    target = await targetInterface(root);
  const names = packageNames(addon, folders, client),
    details = [];
  if (addon && !names.includes(addon.folder))
    throw Error('Main addon folder missing from release.');
  for (const folder of names) {
    const directory = path.join(unpacked, folder),
      files = await fs.readdir(directory);
    const file = preferredToc(files, folder, client);
    if (!file) throw Error('Client addon metadata missing: ' + folder);
    const content = await fs.readFile(path.join(directory, file), 'utf8'),
      meta = toc(content);
    if (!supports(meta, target, addon, client)) {
      if (addon && folder !== addon.folder) continue;
      throw Object.assign(Error(folder + ' does not list the selected client build.'), {
        code: 'INCOMPATIBLE_ADDON',
        folder,
        client,
      });
    }
    await validateFiles(content, directory, unpacked, client);
    details.push({
      folder,
      name: (meta.Title || folder).replace(/\|c[0-9a-f]{8}|\|r/gi, ''),
      version: meta.Version,
      requires: (meta.Dependencies || meta.RequiredDeps || '').split(/[,\s]+/).filter(Boolean),
    });
  }
  return { names: details.map((item) => item.folder), details, interface: target };
}
module.exports = {
  toc,
  allowedFolder,
  preferredToc,
  targetInterface,
  packageNames,
  supports,
  inspectPackage,
};
