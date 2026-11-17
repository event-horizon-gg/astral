import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import engine from '../desktop/engine.cjs';
import updater from '../desktop/updater.cjs';
import { refresh } from '../desktop/talent-builds.cjs';

const prefixes = { balance: 'CYG', feral: 'CcG', guardian: 'CgG', restoration: 'CkG' };
const row = (mode, code, best = '') =>
  String.raw`[tr][symbol=wow-hero-talent-wildstalker] ${best} [copy=\"${mode}\"]${code}[\/copy][\/tr]`;
const guide = (prefix, patch = '12.1.5') =>
  `<div class="interior-sidebar-header-text-subtitle">Patch ${patch}</div>` +
  ['Mythic+', 'Raid', 'Delves']
    .map((mode) => row(mode, prefix + 'A'.repeat(110), '(Best)'))
    .join('');

test('daily refresh verifies all four specs and keeps the old file when a guide is stale or WoW starts', async (t) => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-talents-'));
  t.after(() => fs.rm(data, { recursive: true, force: true }));
  const root = path.join(data, '_retail_');
  const dir = path.join(root, 'Interface', 'AddOns', 'AstralTalents');
  const file = path.join(dir, 'Builds.lua');
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(file, '-- prior builds');

  let stale = true;
  let running = false;
  const download = async (url) => {
    const spec = url.split('/').at(-2);
    return new Response(guide(prefixes[spec], stale && spec === 'guardian' ? '12.1.0' : undefined));
  };
  const addon = engine.catalog.find((addon) => addon.id === 'talents');
  const api = {
    targetInterface: async () => '120105',
    gameRunning: async () => running,
    inventory: async () => ({ addons: [{ ...addon, version: '1.0.0' }] }),
    getRelease: async () => ({ version: '1.0.0' }),
  };
  const run = (now) =>
    updater.run({
      data,
      roots: [root],
      force: true,
      api,
      now,
      refreshTalents: (root, api, now) => refresh(root, api, now, download),
    });

  const now = Date.UTC(2026, 10, 7);
  const failed = await run(now);
  assert.equal(failed.runs[0].status, 'attention');
  assert.equal(await fs.readFile(file, 'utf8'), '-- prior builds');

  stale = false;
  const done = await run(now);
  assert.equal(done.runs[0].results[0].status, 'updated');
  const verified = await fs.readFile(file, 'utf8');
  assert.match(verified, /ns.checked = "2026-11-07"/);
  assert.equal([...verified.matchAll(/C[Ycgk]G[A-Z]{110}/g)].length, 12);

  // The game starts after the last request, before the on-disk replacement.
  const lateDownload = async (url) => {
    running = true;
    return download(url);
  };
  await assert.rejects(refresh(root, api, now + 86400000, lateDownload), /WoW started/);
  assert.equal(await fs.readFile(file, 'utf8'), verified);
  assert.deepEqual(await fs.readdir(dir), ['Builds.lua']);
});
