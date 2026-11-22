import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import zlib from 'node:zlib';
import sharing from '../desktop/setup-sharing.cjs';
import saved from '../desktop/saved-data.cjs';
import settings from '../desktop/settings.cjs';
const running = async () => false;
const hash = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
async function fixture(t, name, client = 'retail') {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-share-'));
  t.after(() => fs.rm(temp, { recursive: true, force: true }));
  const root = path.join(temp, '_' + client + '_'),
    data = path.join(temp, 'data'),
    account = name;
  const base = path.join(root, 'WTF', 'Account', account),
    addon = path.join(root, 'Interface', 'AddOns', 'KeyUI');
  for (const dir of [
    addon,
    path.join(base, 'SavedVariables'),
    path.join(base, 'Realm', name, 'SavedVariables'),
    path.join(base, 'Realm', 'Other'),
  ])
    await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(
    path.join(addon, 'KeyUI.toc'),
    '## Interface: 120105\n## Title: KeyUI\n## SavedVariables: KeyUIData\n## SavedVariablesPerCharacter: KeyUICharacter\n',
  );
  await fs.writeFile(
    path.join(base, 'SavedVariables', 'KeyUI.lua'),
    `KeyUIData = { ["profileKeys"] = { ["${name} - Realm"] = "Default" }, ["message"] = "${name}" }\n`,
  );
  await fs.writeFile(
    path.join(base, 'Realm', name, 'SavedVariables', 'KeyUI.lua'),
    'KeyUICharacter = { ["scale"] = 1.2 }\n',
  );
  await fs.writeFile(path.join(base, 'bindings-cache.wtf'), 'bind A STRAFELEFT\n');
  await fs.writeFile(path.join(base, 'Realm', name, 'bindings-cache.wtf'), 'bind Q SPELL\n');
  await fs.writeFile(path.join(base, 'Realm', 'Other', 'bindings-cache.wtf'), 'other character\n');
  await fs.writeFile(
    path.join(root, 'WTF', 'Config.wtf'),
    'SET gxResolution "1920x1080"\nSET GamePadEnable "0"\n',
  );
  return { root, data, account, base, running, character: 'Realm/' + name };
}
async function share(source, include) {
  return sharing.decode(await sharing.create({ ...source, name: 'Raid setup', include }));
}
function archive(bundle, extra = {}) {
  const payload = zlib.gzipSync(Buffer.from(JSON.stringify(bundle))).toString('base64');
  return Buffer.from(
    JSON.stringify({
      format: 'astral.setup',
      schema: 1,
      name: 'Shared',
      client: 'retail',
      character: '',
      ...extra,
      payload,
      sha256: hash(Buffer.from(payload, 'base64')),
    }),
  );
}

test('cross-account sharing maps a character, backs up settings, and preserves graphics and other characters', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  const shared = await share(source),
    args = { ...target, share: shared };
  assert.equal(shared.bundle.input, undefined);
  assert.equal(
    shared.bundle.files.some((file) => file.path.startsWith('Realm/Other/')),
    false,
  );
  const plan = await sharing.prepare(args);
  assert.deepEqual(plan.packages.manual, []);
  const before = await fs.readFile(path.join(target.base, 'SavedVariables', 'KeyUI.lua'), 'utf8');
  await sharing.apply({ ...args, expected: plan.fingerprint });
  const applied = await fs.readFile(path.join(target.base, 'SavedVariables', 'KeyUI.lua'), 'utf8');
  assert.match(applied, /\["Target - Realm"\]/);
  assert.doesNotMatch(applied, /Source - Realm/);
  assert.match(applied, /\["message"\] = "Source"/);
  assert.equal(
    await fs.readFile(path.join(target.base, 'Realm', 'Other', 'bindings-cache.wtf'), 'utf8'),
    'other character\n',
  );
  assert.equal(
    await fs.readFile(path.join(target.root, 'WTF', 'Config.wtf'), 'utf8'),
    'SET gxResolution "1920x1080"\nSET GamePadEnable "0"\n',
  );
  const backups = await fs.readdir(path.join(target.data, 'settings-backups'));
  assert.equal(
    await fs.readFile(
      path.join(target.data, 'settings-backups', backups[0], 'SavedVariables', 'KeyUI.lua'),
      'utf8',
    ),
    before,
  );
});

test('export respects category choices and requires WoW to be closed', async (t) => {
  const source = await fixture(t, 'Source');
  const shared = await share(source, { settings: false, bindings: true, controller: false });
  assert.ok(shared.bundle.files.every((file) => file.path.endsWith('bindings-cache.wtf')));
  const addonsOnly = await share(source, { settings: false, bindings: false });
  assert.equal(addonsOnly.bundle.files.length, 0);
  assert.equal(addonsOnly.bundle.library[0].folder, 'KeyUI');
  await assert.rejects(
    sharing.create({ ...source, name: 'Live', running: async () => true }),
    /Close WoW/,
  );
});

test('stale previews and imports into a different game client are refused', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  const shared = await share(source),
    args = { ...target, share: shared };
  const plan = await sharing.prepare(args);
  await fs.writeFile(path.join(target.base, 'bindings-cache.wtf'), 'changed after review');
  await assert.rejects(sharing.apply({ ...args, expected: plan.fingerprint }), /changed/);
  assert.equal(
    await fs.readFile(path.join(target.base, 'bindings-cache.wtf'), 'utf8'),
    'changed after review',
  );
  const forever = await fixture(t, 'Forever', 'forever');
  await assert.rejects(sharing.prepare({ ...forever, share: shared }), /Retail/);
});

test('missing/manual addons stay visible and block application', async (t) => {
  const target = await fixture(t, 'Target');
  const current = (await share(target)).bundle;
  current.library.push({ folder: 'UnknownAddon', name: 'Unknown addon' });
  const shared = sharing.decode(archive(current, { character: target.character }));
  const args = { ...target, share: shared },
    plan = await sharing.prepare(args);
  assert.equal(plan.packages.manual[0].folder, 'UnknownAddon');
  await assert.rejects(sharing.apply({ ...args, expected: plan.fingerprint }), /missing addons/);
});

test('shared settings reject executable code and undeclared globals', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  await fs.writeFile(
    path.join(source.base, 'SavedVariables', 'KeyUI.lua'),
    'KeyUIData = os.execute("bad")',
  );
  await assert.rejects(sharing.create({ ...source, name: 'Unsafe' }), /executable Lua/);
  await fs.writeFile(path.join(source.base, 'SavedVariables', 'KeyUI.lua'), 'UIParent = {}');
  await assert.rejects(sharing.create({ ...source, name: 'Wrong globals' }), /does not declare/);
  const current = (await share(target)).bundle;
  const file = current.files.find((file) => file.path === 'SavedVariables/KeyUI.lua');
  file.content = Buffer.from('UIParent = {}').toString('base64');
  file.sha256 = hash(Buffer.from(file.content, 'base64'));
  const shared = sharing.decode(archive(current, { character: target.character }));
  const args = { ...target, share: shared },
    plan = await sharing.prepare(args);
  await assert.rejects(sharing.apply({ ...args, expected: plan.fingerprint }), /does not declare/);
  assert.match(
    await fs.readFile(path.join(target.base, 'SavedVariables', 'KeyUI.lua'), 'utf8'),
    /KeyUIData/,
  );
});

test('WoW opening during import rolls back the settings write', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  const shared = await share(source),
    args = { ...target, share: shared };
  const before = await settings.capture(target.root, target.account, target.data),
    plan = await sharing.prepare(args);
  let calls = 0;
  await assert.rejects(
    sharing.apply({ ...args, expected: plan.fingerprint, running: async () => ++calls >= 4 }),
    /WoW started/,
  );
  assert.equal(
    (await settings.capture(target.root, target.account, target.data)).hash,
    before.hash,
  );
});

test('malformed, oversized, duplicate and foreign-character files are rejected', async (t) => {
  assert.throws(() => sharing.decode(Buffer.from('{}')), /Invalid/);
  assert.throws(() => sharing.decode(Buffer.alloc(sharing.MAX_FILE + 1)), /36 MB/);
  const source = await fixture(t, 'Source');
  const current = settings.decode(
    (await settings.capture(source.root, source.account, source.data)).payload,
  );
  assert.throws(() => sharing.decode(archive(current)), /unexpected character/);
  current.files.push({ ...current.files[0] });
  assert.throws(
    () => sharing.decode(archive(current, { character: source.character })),
    /Invalid file/,
  );
});

test('data grammar supports Unicode, binary escapes, comments and inert code-looking strings', () => {
  const source =
    'DB = { ["Jo\\195\\163o - Realm"] = [[function() is just text]], ["binary"] = "\\255", [1] = -1.2e3, foo = true, } -- comment';
  const parsed = saved.parse(source, new Map([['João - Realm', 'Target - Realm']]));
  assert.match(parsed.text, /Target - Realm/);
  assert.deepEqual(parsed.globals, ['DB']);
  for (const code of [
    'DB = function() end',
    'DB = { [nil] = 1 }',
    'DB = { function = 1 }',
    'DB = {}; print("bad")',
    'DB = setmetatable({}, {})',
    'DB.x = 1',
    'DB = { os.time() }',
    'DB = "\\q"',
    'DB = { [1] = 1; }; return DB',
  ])
    assert.throws(() => saved.parse(code), /executable Lua/);
});

test('Forever setups share independently of Retail', async (t) => {
  const source = await fixture(t, 'Source', 'forever'),
    target = await fixture(t, 'Target', 'forever');
  const shared = await share(source),
    args = { ...target, share: shared };
  assert.equal(shared.client, 'forever');
  const plan = await sharing.prepare(args);
  await sharing.apply({ ...args, expected: plan.fingerprint });
  assert.match(
    await fs.readFile(path.join(target.base, 'SavedVariables', 'KeyUI.lua'), 'utf8'),
    /Target - Realm/,
  );
});

test('import installs connected addons before applying their declared settings', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  await fs.rm(path.join(target.root, 'Interface', 'AddOns', 'KeyUI'), { recursive: true });
  const shared = await share(source),
    args = { ...target, share: shared };
  const plan = await sharing.prepare(args);
  assert.equal(plan.packages.queue[0].folder, 'KeyUI');
  let installed = false;
  await sharing.apply({
    ...args,
    expected: plan.fingerprint,
    install: async ({ afterInstall }) => {
      await fs.cp(
        path.join(source.root, 'Interface', 'AddOns', 'KeyUI'),
        path.join(target.root, 'Interface', 'AddOns', 'KeyUI'),
        { recursive: true },
      );
      installed = true;
      await afterInstall();
    },
  });
  assert.ok(installed);
  assert.match(
    await fs.readFile(path.join(target.base, 'SavedVariables', 'KeyUI.lua'), 'utf8'),
    /Target - Realm/,
  );
});

test('export excludes Astral credentials and Blizzard account history', async (t) => {
  const source = await fixture(t, 'Source');
  await fs.mkdir(source.data, { recursive: true });
  await fs.writeFile(
    path.join(source.data, 'session.json'),
    JSON.stringify({ token: 'SECRET_SESSION_TOKEN' }),
  );
  await fs.writeFile(
    path.join(source.base, 'SavedVariables', 'Blizzard_ChatFrame.lua'),
    'CHAT_HISTORY = { "private chat" }',
  );
  const shared = await share(source);
  assert.ok(shared.bundle.files.every((file) => !file.path.includes('Blizzard_ChatFrame')));
  const exported = shared.bundle.files.map((file) => ({
    path: file.path,
    content: Buffer.from(file.content, 'base64').toString('utf8'),
  }));
  assert.ok(exported.every((file) => !file.path.includes('session.json')));
  assert.ok(!JSON.stringify(exported).includes('SECRET_SESSION_TOKEN'));
});

test('a failed preferences commit rolls back overwritten and newly created settings', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  const newFile = path.join(target.base, 'Realm', 'Target', 'SavedVariables', 'KeyUI.lua');
  await fs.rm(newFile);
  const shared = await share(source),
    args = { ...target, share: shared };
  const before = await settings.capture(target.root, target.account, target.data),
    plan = await sharing.prepare(args);
  await assert.rejects(
    sharing.apply({
      ...args,
      expected: plan.fingerprint,
      commit: async () => {
        throw Error('Preferences could not be saved');
      },
    }),
    /Preferences/,
  );
  await assert.rejects(fs.stat(newFile), { code: 'ENOENT' });
  assert.equal(
    (await settings.capture(target.root, target.account, target.data)).hash,
    before.hash,
  );
});

test('excluded and orphaned SavedVariables are not read into a share or import preview', async (t) => {
  const source = await fixture(t, 'Source'),
    target = await fixture(t, 'Target');
  // This would exceed the ordinary capture limit if read before filtering.
  for (const setup of [source, target])
    await fs.writeFile(
      path.join(setup.base, 'SavedVariables', 'EllesmereUIOrphan.lua'),
      Buffer.alloc(8 * 1024 ** 2 + 1),
    );
  const shared = await share(source);
  assert.ok(shared.bundle.files.every((file) => !file.path.includes('Orphan')));
  await fs.writeFile(
    path.join(source.base, 'SavedVariables', 'KeyUI.lua'),
    Buffer.alloc(8 * 1024 ** 2 + 1),
  );
  const bindings = await share(source, { settings: false, bindings: true });
  assert.ok(bindings.bundle.files.every((file) => file.path.endsWith('bindings-cache.wtf')));
  const plan = await sharing.prepare({ ...target, share: shared });
  assert.equal(plan.packages.manual.length, 0);
});
