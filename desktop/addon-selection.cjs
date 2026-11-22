const fs = require('node:fs/promises');
const path = require('node:path');
const metadata = require('./addon-metadata.cjs');
const { flavor } = require('./flavors.cjs');
const {
  validFolder,
  validScope,
  parse,
  entryNames,
  validate,
  effective,
  edit,
  rows,
} = require('./addon-selection-data.cjs');
async function folders(root) {
  const dir = path.join(root, 'Interface', 'AddOns');
  const config = await fs.readFile(path.join(root, 'WTF', 'Config.wtf'), 'utf8').catch(() => '');
  const disabledByDefault = /^SET\s+disableUserAddonsByDefault\s+"1"\s*$/im.test(config);
  const entries = await fs.readdir(dir, { withFileTypes: true });
  const result = [];
  for (const entry of entries) {
    if (!entry.isDirectory() || !validFolder(entry.name)) continue;
    const base = path.join(dir, entry.name),
      files = await fs.readdir(base);
    const toc = metadata.preferredToc(files, entry.name, flavor(root));
    const meta = toc ? metadata.toc(await fs.readFile(path.join(base, toc), 'utf8')) : {};
    result.push({
      name: entry.name,
      default: !disabledByDefault && meta.DefaultState?.toLowerCase() !== 'disabled',
      dependencies: (meta.Dependencies || meta.RequiredDeps || '').split(/[,\s]+/).filter(Boolean),
    });
  }
  return result;
}
async function scopes(base) {
  const result = [''];
  for (const realm of await fs.readdir(base, { withFileTypes: true })) {
    if (!realm.isDirectory() || realm.name === 'SavedVariables') continue;
    for (const character of await fs.readdir(path.join(base, realm.name), {
      withFileTypes: true,
    })) {
      const scope = realm.name + '/' + character.name;
      if (character.isDirectory() && validScope(scope)) result.push(scope);
    }
  }
  return result.sort();
}
async function file(base, scope) {
  if (!validScope(scope)) throw Error('Invalid addon selection scope.');
  const dir = path.join(base, ...scope.split('/').filter(Boolean));
  let current = base;
  for (const part of scope.split('/').filter(Boolean)) {
    current = path.join(current, part);
    const stat = await fs.lstat(current).catch(() => null);
    if (stat && !stat.isDirectory()) throw Error('Addon selection paths cannot contain links.');
  }
  const names = (
    await fs.readdir(dir).catch((error) => {
      if (error.code === 'ENOENT') return [];
      throw error;
    })
  ).filter((name) => name.toLowerCase() === 'addons.txt');
  if (names.length > 1) throw Error('Duplicate addon selection files need attention.');
  const name = names[0] || 'AddOns.txt';
  const stat = await fs.lstat(path.join(dir, name)).catch(() => null);
  if (stat && !stat.isFile()) throw Error('Addon selection files cannot be links.');
  return {
    relative: (scope ? scope + '/' : '') + name,
    text: stat ? await fs.readFile(path.join(dir, name), 'utf8') : '',
  };
}
async function capture(root, base) {
  const installed = await folders(root),
    result = { schema: 1, scopes: {} };
  const all = new Map();
  const known = new Map();
  for (const scope of await scopes(base)) {
    const current = await file(base, scope);
    all.set(scope, parse(current.text));
    for (const name of entryNames(current.text)) known.set(name.toLowerCase(), name);
  }
  for (const addon of installed) known.set(addon.name.toLowerCase(), addon.name);
  const names = [...known.values()].sort(),
    defaults = all.get('');
  const native = new Map(installed.map((addon) => [addon.name.toLowerCase(), addon.default]));
  for (const [scope, override] of all) {
    result.scopes[scope] = Object.fromEntries(
      names.map((name) => [
        name,
        override[name.toLowerCase()] ??
          defaults[name.toLowerCase()] ??
          native.get(name.toLowerCase()) ??
          true,
      ]),
    );
  }
  return result;
}

async function plan(root, base, selection) {
  validate(selection);
  const before = await capture(root, base);
  const installed = await folders(root),
    changes = [];
  const known = new Map();
  for (const values of [...Object.values(before.scopes), ...Object.values(selection.scopes)])
    for (const name of Object.keys(values)) known.set(name.toLowerCase(), name);
  for (const addon of installed) known.set(addon.name.toLowerCase(), addon.name);
  const allNames = [...known.values()].sort();
  const installedNames = new Set(installed.map((addon) => addon.name.toLowerCase()));
  const allScopes = [
    ...new Set([...Object.keys(before.scopes), ...Object.keys(selection.scopes)]),
  ].sort();
  for (const scope of allScopes) {
    const values = selection.scopes[scope] || selection.scopes[''];
    const states = new Map(allNames.map((name) => [name.toLowerCase(), effective(values, name)]));
    for (const addon of installed)
      if (states.get(addon.name.toLowerCase())) {
        for (const dependency of addon.dependencies) {
          if (/^Blizzard_/i.test(dependency)) continue;
          const oldValues = before.scopes[scope] || before.scopes[''];
          if (
            (!installedNames.has(dependency.toLowerCase()) ||
              !states.get(dependency.toLowerCase())) &&
            (!effective(oldValues, addon.name) || effective(oldValues, dependency))
          )
            throw Error(
              addon.name + ' requires ' + dependency + '. Enable both addons in this profile.',
            );
        }
      }
    const current = await file(base, scope),
      lines = current.text.split(/\r?\n/);
    const names = new Set(allNames.map((name) => name.toLowerCase()));
    const kept = lines.filter((line) => {
      const match = line.match(/^\s*([^:]+):/);
      return !match || !names.has(match[1].trim().toLowerCase());
    });
    while (kept.at(-1) === '') kept.pop();
    const content =
      [
        ...kept,
        ...allNames.map(
          (name) => `${name}: ${states.get(name.toLowerCase()) ? 'enabled' : 'disabled'}`,
        ),
      ].join('\n') + '\n';
    if (content !== current.text)
      changes.push({ path: current.relative, content: Buffer.from(content).toString('base64') });
  }
  return changes;
}
module.exports = { capture, validate, edit, plan, rows };
