import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import storage from '../desktop/storage.cjs';
import github from '../desktop/github.cjs';
import operations from '../desktop/addon-operations.cjs';
import activity from '../desktop/addon-activity.cjs';
import updater from '../desktop/updater.cjs';
async function directory(t) {
  const folder = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-boundaries-'));
  t.after(() => fs.rm(folder, { recursive: true, force: true }));
  return folder;
}
test('missing state has a default, but corrupt and unreadable state is never silently replaced', async (t) => {
  const data = await directory(t),
    file = path.join(data, 'state.json');
  assert.deepEqual(await storage.json(file, { default: true }), { default: true });
  await fs.writeFile(file, '{broken');
  await assert.rejects(storage.json(file, { default: true }), SyntaxError);
  await assert.rejects(storage.json(data, { default: true }), { code: 'EISDIR' });
  assert.equal(await fs.readFile(file, 'utf8'), '{broken');
});
test('a failed atomic save leaves the existing destination and no temporary state', async (t) => {
  const data = await directory(t),
    target = path.join(data, 'state.json');
  await fs.mkdir(target);
  await fs.writeFile(path.join(target, 'untouched'), 'original');
  await assert.rejects(storage.save(target, { overwrite: true }));
  assert.deepEqual(await fs.readdir(data), ['state.json']);
  assert.equal(await fs.readFile(path.join(target, 'untouched'), 'utf8'), 'original');
});
test('provider failures carry stable codes independently of presentation wording', async () => {
  const request = async () => ({ ok: false, status: 429 });
  await assert.rejects(
    github.latest('author/addon', { request, rateMessage: 'Please retry tomorrow.' }),
    (error) =>
      error.code === 'RATE_LIMIT' &&
      error.status === 429 &&
      error.message === 'Please retry tomorrow.',
  );
  await assert.rejects(github.latest('../wrong', { request }), /Invalid GitHub/);
});
test('manual update preserves current versions and records release failures in Activity', async (t) => {
  const data = await directory(t),
    root = path.join(data, '_retail_');
  const addon = { id: 'fixture', folder: 'Fixture', name: 'Fixture', version: '2' };
  let installs = 0;
  const api = {
    gameRunning: async () => false,
    getRelease: async () => ({ version: '1', tag: 'v1' }),
    install: async () => {
      installs++;
    },
  };
  const current = await operations.execute({
    data,
    root,
    addon,
    api,
    versionPolicy: 'never-downgrade',
  });
  assert.equal(current.status, 'current');
  assert.equal(installs, 0);
  assert.equal((await activity.list(data, root))[0].status, 'current');
  api.getRelease = async () => {
    throw Error('The publisher is unavailable.');
  };
  await assert.rejects(operations.execute({ data, root, addon, api }), /publisher/);
  assert.equal((await activity.list(data, root))[0].status, 'error');
});
for (const [code, status, delay] of [
  ['RATE_LIMIT', 'attention', 3600000],
  ['WOW_RUNNING', 'deferred', 60000],
]) {
  test('daily scheduling reacts to ' + code + ', rather than matching error text', async (t) => {
    const data = await directory(t),
      now = 1000000;
    let checked = 0;
    const api = {
      gameRunning: async () => false,
      inventory: async () => ({
        addons: ['First', 'Second'].map((id) => ({
          id,
          name: id,
          repo: 'author/' + id,
          version: '1',
        })),
      }),
      getRelease: async () => {
        checked++;
        throw Object.assign(Error('Translated provider message'), { code });
      },
    };
    const result = await updater.run({ data, roots: ['/WoW/_retail_'], api, now });
    assert.equal(checked, 1);
    assert.equal(result.runs[0].status, status);
    assert.equal(result.nextAt, now + delay);
  });
}
