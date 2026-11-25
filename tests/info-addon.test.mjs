import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import catalog from '../desktop/catalog.cjs';
import engine from '../desktop/engine.cjs';
import bundled from '../desktop/bundled.cjs';
import format from '../desktop/setup-format.cjs';

test('Astral Info installs from its bundled source for both clients and its settings are eligible for profile sync', async (t) => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-info-'));
  t.after(() => fs.rm(temp, { recursive: true, force: true }));
  const definition = catalog.find((addon) => addon.id === 'info');
  const release = await bundled.release(definition);
  assert.equal(release.version, '0.1.0');
  for (const [client, build] of [
    ['retail', 120105],
    ['forever', 16001],
  ]) {
    const root = path.join(temp, '_' + client + '_'),
      data = path.join(temp, 'data-' + client);
    await fs.mkdir(path.join(root, 'Interface', 'AddOns'), { recursive: true });
    await engine.install(definition, root, data, release, () => {}, {
      running: async () => false,
    });
    const toc = await fs.readFile(
      path.join(root, 'Interface', 'AddOns', 'AstralInfo', 'AstralInfo.toc'),
      'utf8',
    );
    assert.match(toc, new RegExp(String(build)));
    assert.equal(
      (await engine.inventory(root, data)).addons.find((addon) => addon.id === 'info').version,
      '0.1.0',
    );
    for (const file of ['Settings.lua', 'Performance.lua', 'MountTooltip.lua', 'Info.lua'])
      assert.ok(
        (await fs.stat(path.join(root, 'Interface', 'AddOns', 'AstralInfo', file))).isFile(),
      );
  }
  assert.ok(format.allowed('SavedVariables/AstralInfo.lua'));
});
