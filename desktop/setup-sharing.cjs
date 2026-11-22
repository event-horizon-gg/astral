const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const format = require('./setup-format.cjs');
const engine = require('./engine.cjs');
const settings = require('./settings.cjs');
const selections = require('./addon-selection.cjs');
const packages = require('./library-sync.cjs');
const setup = require('./setup-application.cjs');
const flavors = require('./flavors.cjs');
const savedData = require('./saved-data.cjs');
const MAX_FILE = 36 * 1024 ** 2;
const hash = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const scopeValid = (scope) =>
  typeof scope === 'string' &&
  (scope === '' ||
    (scope.split('/').length === 2 &&
      scope
        .split('/')
        .every(
          (p) =>
            p &&
            p.length <= 128 &&
            !['.', '..', 'SavedVariables'].includes(p) &&
            !/[\\:\x00-\x1f]/.test(p),
        )));
function options(value = {}) {
  if (
    !value ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    Object.keys(value).some((key) => !['settings', 'bindings', 'controller'].includes(key)) ||
    Object.values(value).some((value) => typeof value !== 'boolean')
  )
    throw Error('Invalid sharing options.');
  return {
    settings: value.settings !== false,
    bindings: value.bindings !== false,
    controller: value.controller === true,
  };
}
function wanted(file, include) {
  // Blizzard layout cache files are portable. Built-in SavedVariables (chat,
  // account history, etc.) are outside addon profile sharing.
  if (/^Blizzard_/i.test(path.posix.basename(file)) && file.endsWith('.lua')) return false;
  return file.endsWith('bindings-cache.wtf') ? include.bindings : include.settings;
}
function text(file) {
  const bytes = Buffer.from(file.content, 'base64'),
    result = bytes.toString('utf8');
  if (bytes.length > 8 * 1024 ** 2 || !Buffer.from(result).equals(bytes))
    throw Error('A settings file is too large or is not UTF-8 text.');
  return result;
}
async function characters(root, account) {
  const base = await settings.accountRoot(root, account),
    result = [];
  for (const realm of await fs.readdir(base, { withFileTypes: true })) {
    if (!realm.isDirectory() || realm.name === 'SavedVariables') continue;
    for (const character of await fs.readdir(path.join(base, realm.name), {
      withFileTypes: true,
    })) {
      const value = realm.name + '/' + character.name;
      if (character.isDirectory() && scopeValid(value))
        result.push({ value, label: character.name + ' — ' + realm.name });
    }
  }
  return result.sort((a, b) => a.label.localeCompare(b.label));
}
async function declarations(root, folder) {
  if (!/^[^/\\:\x00-\x1f]{1,100}$/.test(folder)) throw Error('Invalid addon settings folder.');
  const dir = path.join(root, 'Interface', 'AddOns', folder);
  const stat = await fs.lstat(dir).catch(() => null);
  if (!stat?.isDirectory() || stat.isSymbolicLink())
    throw Error('Install ' + folder + ' before importing its settings.');
  const names = new Set();
  for (const file of await fs.readdir(dir, { withFileTypes: true })) {
    if (!file.isFile() || !/\.toc$/i.test(file.name)) continue;
    const toc = await fs.readFile(path.join(dir, file.name), 'utf8');
    if (toc.length > 1024 ** 2) throw Error('The addon manifest is too large.');
    for (const match of toc.matchAll(/^##\s*SavedVariables(?:PerCharacter)?\s*:\s*([^\r\n]*)/gim))
      for (const name of match[1].split(/[,\s]+/).filter(Boolean)) names.add(name);
  }
  return names;
}
async function validateGlobals(root, files) {
  const known = new Map();
  for (const file of files.filter((file) => file.path.endsWith('.lua'))) {
    const folder = path.posix.basename(file.path, '.lua');
    if (!known.has(folder)) known.set(folder, await declarations(root, folder));
    for (const name of savedData.parse(text(file)).globals)
      if (!known.get(folder).has(name))
        throw Error(folder + ' does not declare the saved variable ' + name + '.');
  }
}
function decode(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length > MAX_FILE)
    throw Error('This setup exceeds the 36 MB file limit.');
  let share;
  try {
    share = JSON.parse(bytes.toString('utf8'));
  } catch {
    throw Error('This is not an Astral setup file.');
  }
  if (
    !share ||
    share.format !== 'astral.setup' ||
    share.schema !== 1 ||
    !['retail', 'forever'].includes(share.client) ||
    typeof share.name !== 'string' ||
    !share.name.trim() ||
    share.name.length > 80 ||
    /[\x00-\x1f]/.test(share.name) ||
    !scopeValid(share.character) ||
    typeof share.payload !== 'string' ||
    typeof share.sha256 !== 'string' ||
    !/^[a-f0-9]{64}$/.test(share.sha256) ||
    hash(Buffer.from(share.payload, 'base64')) !== share.sha256
  )
    throw Error('Invalid or damaged Astral setup file.');
  const bundle = format.decode(share.payload);
  if (
    bundle.schema !== 2 ||
    !bundle.library ||
    bundle.files.some(
      (file) =>
        file.path.split('/').length > 2 &&
        (!share.character || !file.path.startsWith(share.character + '/')),
    ) ||
    Object.keys(bundle.addonSelection.scopes).some(
      (scope) => scope !== '' && scope !== share.character,
    )
  )
    throw Error('The setup contains unexpected character settings.');
  const seen = new Set();
  for (const file of bundle.files) {
    const key = file.path.toLowerCase();
    if (seen.has(key)) throw Error('The setup contains duplicate settings paths.');
    seen.add(key);
    if (file.path.endsWith('.lua')) savedData.parse(text(file));
  }
  return { name: share.name.trim(), client: share.client, character: share.character, bundle };
}
async function create({
  root,
  account,
  data,
  name,
  character = '',
  include,
  running = engine.gameRunning,
}) {
  if (await running()) throw Error('Close WoW to export your latest settings.');
  const list = await characters(root, account);
  if (!scopeValid(character) || (character && !list.some((item) => item.value === character)))
    throw Error('Choose a local character.');
  if (typeof name !== 'string' || !name.trim() || name.length > 80 || /[\x00-\x1f]/.test(name))
    throw Error('Name your setup (up to 80 characters).');
  include = options(include);
  const inventory = await engine.inventory(root, data);
  const installed = new Set(
    inventory.addons
      .flatMap((addon) => addon.modules || [addon.folder])
      .map((name) => name.toLowerCase()),
  );
  const snapshot = await settings.capture(root, account, data, {
    selectFile: (relative) =>
      wanted(relative, include) &&
      (!relative.endsWith('.lua') ||
        installed.has(path.posix.basename(relative, '.lua').toLowerCase())) &&
      (relative.split('/').length <= 2 || (character && relative.startsWith(character + '/'))),
  });
  const bundle = format.decode(snapshot.payload);
  bundle.addonSelection.scopes = Object.fromEntries(
    Object.entries(bundle.addonSelection.scopes).filter(
      ([scope]) => scope === '' || scope === character,
    ),
  );
  if (!include.controller) delete bundle.input;
  await validateGlobals(root, bundle.files);
  if (await running()) throw Error('WoW started. Export again after closing it.');
  const payload = format.encode(bundle).payload;
  const share = {
    format: 'astral.setup',
    schema: 1,
    name: name.trim(),
    client: flavors.flavor(root),
    character,
    payload,
    sha256: hash(Buffer.from(payload, 'base64')),
  };
  const bytes = Buffer.from(JSON.stringify(share));
  decode(bytes);
  return bytes;
}
function mapped(share, character, include) {
  if (!scopeValid(character) || (share.character && !character))
    throw Error('Choose the character to receive this setup.');
  include = options(include);
  const bundle = structuredClone(share.bundle),
    replacements = new Map();
  if (share.character && character) {
    const [realm, name] = share.character.split('/'),
      [targetRealm, targetName] = character.split('/');
    for (const [a, b] of [
      [`${name} - ${realm}`, `${targetName} - ${targetRealm}`],
      [`${name}-${realm}`, `${targetName}-${targetRealm}`],
      [`${realm} - ${name}`, `${targetRealm} - ${targetName}`],
    ])
      replacements.set(a, b);
  }
  bundle.files = bundle.files
    .filter((file) => wanted(file.path, include))
    .map((file) => {
      const content = Buffer.from(
        file.path.endsWith('.lua') ? savedData.parse(text(file), replacements).text : text(file),
      );
      return {
        path:
          share.character && file.path.startsWith(share.character + '/')
            ? character + file.path.slice(share.character.length)
            : file.path,
        content: content.toString('base64'),
        sha256: hash(content),
      };
    });
  if (!include.controller) delete bundle.input;
  return bundle;
}
async function prepare({ root, account, data, share, character = '', include, api = engine }) {
  if (share.client !== flavors.flavor(root))
    throw Error(
      'Select WoW ' +
        (share.client === 'retail' ? 'Retail' : 'Forever') +
        ' before importing this setup.',
    );
  if (character && !(await characters(root, account)).some((item) => item.value === character))
    throw Error('Choose a local character.');
  const bundle = mapped(share, character, include);
  const affected = new Set(bundle.files.map((file) => file.path));
  const current = await settings.capture(root, account, data, {
    selectFile: (relative) => affected.has(relative),
  });
  const before = format.decode(current.payload);
  // Preserve unrelated local characters and addons instead of spreading the
  // source character's overrides to every character in the recipient account.
  const original = bundle.addonSelection;
  bundle.addonSelection = structuredClone(before.addonSelection);
  Object.assign(bundle.addonSelection.scopes[''], original.scopes['']);
  if (character)
    bundle.addonSelection.scopes[character] = {
      ...(bundle.addonSelection.scopes[character] || {}),
      ...(original.scopes[share.character] || original.scopes['']),
    };
  const packagePlan = await packages.plan({ root, data, target: bundle.library, api });
  const old = new Map(before.files.map((file) => [file.path, file.sha256]));
  const changedSettings = bundle.files
    .filter((file) => old.get(file.path) !== file.sha256)
    .map((file) => file.path);
  if (bundle.input !== undefined && JSON.stringify(bundle.input) !== JSON.stringify(before.input))
    changedSettings.push('Controller settings');
  const inventory = await api.inventory(root, data);
  const oldChoices = selections.rows(before.addonSelection, inventory.addons);
  const changes = selections
    .rows(bundle.addonSelection, inventory.addons)
    .filter((row) => oldChoices.find((old) => old.id === row.id)?.enabled !== row.enabled)
    .map((row) => ({ ...row, before: oldChoices.find((old) => old.id === row.id)?.enabled }));
  const target = format.encode(bundle).payload;
  const fingerprint = hash(
    JSON.stringify({
      current: current.hash,
      target: format.inspect(target).hash,
      packages: packagePlan.fingerprint,
    }),
  );
  return { current, target, packages: packagePlan, changedSettings, changes, fingerprint };
}
async function apply({
  root,
  account,
  data,
  share,
  character,
  include,
  expected,
  running = engine.gameRunning,
  install = packages.install,
  api = engine,
  onProgress,
  commit = async () => {},
}) {
  if (await running()) throw Error('Close WoW before importing a setup.');
  const plan = await prepare({ root, account, data, share, character, include, api });
  if (expected !== plan.fingerprint) throw Error('Your setup changed. Review the import again.');
  if (plan.packages.manual.length)
    throw Error('Connect or install the missing addons before importing this setup.');
  await setup.installPackages({
    root,
    account,
    data,
    plan: plan.packages,
    current: plan.current,
    running,
    onProgress,
    api,
    library: { install },
  });
  const bundle = format.decode(plan.target);
  await validateGlobals(root, bundle.files);
  await settings.apply(root, account, plan.target, data, running, commit);
  return plan;
}
module.exports = { MAX_FILE, options, characters, create, decode, prepare, apply };
