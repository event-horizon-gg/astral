const fs = require('node:fs/promises'),
  path = require('node:path'),
  crypto = require('node:crypto'),
  { allowed, decode, inspect, encode } = require('./setup-format.cjs');
const { exists, save } = require('./storage.cjs');
const selections = require('./addon-selection.cjs');
const library = require('./library-inventory.cjs');
const gamepad = require('./gamepad-settings.cjs');
async function accounts(root) {
  return (
    await fs.readdir(path.join(root, 'WTF', 'Account'), { withFileTypes: true }).catch(() => [])
  )
    .filter((e) => e.isDirectory() && e.name !== 'SavedVariables')
    .map((e) => e.name);
}
async function accountRoot(root, account) {
  if (!account || /[\\/]|^\.{1,2}$/.test(account)) throw Error('Select a local WoW account first.');
  const dir = path.join(root, 'WTF', 'Account', account);
  if (!(await fs.lstat(dir).catch(() => null))?.isDirectory())
    throw Error('The selected WoW account was not found.');
  return dir;
}
async function capture(root, account, data, { selectFile = () => true } = {}) {
  const base = await accountRoot(root, account),
    files = [];
  async function walk(dir, prefix = '', depth = 0) {
    if (depth > 4) return;
    for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
      const rel = prefix + entry.name,
        p = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) await walk(p, rel + '/', depth + 1);
      else if (allowed(rel) && selectFile(rel)) {
        const content = await fs.readFile(p);
        if (content.length > 8 * 1024 ** 2)
          throw Error('One settings file exceeds the 8 MB preview limit.');
        files.push({
          path: rel,
          content: content.toString('base64'),
          sha256: crypto.createHash('sha256').update(content).digest('hex'),
        });
      }
    }
  }
  await walk(base);
  files.sort((a, b) => a.path.localeCompare(b.path));
  const addonSelection = await selections.capture(root, base);
  return encode({
    schema: 2,
    files,
    addonSelection,
    library: await library.installed(root, data),
    input: gamepad.parse(await gamepad.read(root)),
  });
}
async function apply(root, account, payload, data, running, commit = async () => {}) {
  const bundle = decode(payload);
  if (await running()) throw Error('Close WoW before applying settings.');
  const base = await accountRoot(root, account);
  const files = [
    ...bundle.files.map(({ path, content }) => ({ path, content })),
    ...(bundle.schema === 2 ? await selections.plan(root, base, bundle.addonSelection) : []),
  ];
  if (bundle.input !== undefined) {
    const text = await gamepad.read(root);
    const content = gamepad.render(text, bundle.input);
    if (text !== content)
      files.push({
        path: '@installation/Config.wtf',
        installation: true,
        content: Buffer.from(content).toString('base64'),
      });
  }
  const backup = path.join(data, 'settings-backups', Date.now() + '-' + crypto.randomUUID());
  await fs.mkdir(backup, { recursive: true });
  const changed = [];
  try {
    for (const f of files) {
      if (await running()) throw Error('WoW started. Settings change cancelled.');
      const parent = f.installation ? path.join(root, 'WTF') : base;
      const parts = f.installation ? ['Config.wtf'] : f.path.split('/');
      const dest = path.join(parent, ...parts);
      let current = parent;
      if ((await fs.lstat(parent)).isSymbolicLink())
        throw Error('Settings paths cannot contain links.');
      for (const part of parts) {
        current = path.join(current, part);
        if ((await fs.lstat(current).catch(() => null))?.isSymbolicLink())
          throw Error('Settings paths cannot contain links.');
      }
      const had = await exists(dest);
      if (had) {
        const bp = path.join(backup, ...f.path.split('/'));
        await fs.mkdir(path.dirname(bp), { recursive: true });
        await fs.copyFile(dest, bp);
      }
      changed.push({ path: f.path, had, installation: !!f.installation });
      await fs.mkdir(path.dirname(dest), { recursive: true });
      const tmp = dest + '.astral-' + crypto.randomUUID() + '.tmp';
      try {
        await fs.writeFile(tmp, Buffer.from(f.content, 'base64'), { flag: 'wx' });
        await fs.rename(tmp, dest);
      } finally {
        await fs.rm(tmp, { force: true });
      }
    }
    if (await running()) throw Error('WoW started. Settings change cancelled.');
    await save(path.join(backup, 'manifest.json'), {
      files: changed,
      time: new Date().toISOString(),
    });
    await commit();
    return files.length;
  } catch (e) {
    for (const f of changed.reverse()) {
      const dest = f.installation
        ? path.join(root, 'WTF', 'Config.wtf')
        : path.join(base, ...f.path.split('/'));
      if (f.had) await fs.copyFile(path.join(backup, ...f.path.split('/')), dest);
      else await fs.rm(dest, { force: true });
    }
    throw e;
  }
}
module.exports = { inspect, allowed, accounts, accountRoot, capture, decode, apply };
