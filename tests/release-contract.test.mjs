import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import updates from '../desktop/app-updates.cjs';
const run = promisify(execFile);

test('the actual release signer produces assets accepted by both installed-platform verifiers', async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-release-contract-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  for (const folder of ['scripts', 'shared', 'desktop', 'release'])
    await fs.mkdir(path.join(root, folder));
  for (const file of ['scripts/release-manifest.mjs', 'shared/release-contract.cjs'])
    await fs.copyFile(new URL('../' + file, import.meta.url), path.join(root, file));
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  await fs.writeFile(
    path.join(root, 'desktop', 'update-public-key.pem'),
    publicKey.export({ type: 'spki', format: 'pem' }),
  );
  await fs.writeFile(path.join(root, 'package.json'), JSON.stringify({ version: '2.3.4' }));
  await fs.writeFile(
    path.join(root, 'CHANGELOG.md'),
    '# Changes\n\n## 2.3.4\n\nA real release note.\n',
  );
  const assets = [
    ['darwin', 'arm64', 'Astral-2.3.4-mac-arm64.zip'],
    ['win32', 'x64', 'Astral-2.3.4-win-x64.exe'],
  ];
  for (const [platform, , file] of assets)
    await fs.writeFile(path.join(root, 'release', file), 'fixture installer for ' + platform);
  await run(process.execPath, [path.join(root, 'scripts', 'release-manifest.mjs')], {
    cwd: root,
    env: {
      ...process.env,
      ASTRAL_RELEASE_PRIVATE_KEY: privateKey.export({ type: 'pkcs8', format: 'pem' }),
    },
  });
  const envelope = JSON.parse(
    await fs.readFile(path.join(root, 'release', 'astral-update.json'), 'utf8'),
  );
  for (const [platform, arch, file] of assets) {
    const verified = updates.verify(envelope, publicKey, platform, arch);
    assert.equal(verified.version, '2.3.4');
    assert.ok(verified.notes.includes('A real release note.'));
    const expected = await fs.readFile(path.join(root, 'release', file));
    assert.equal(verified.asset.size, expected.length);
    assert.equal(verified.asset.sha256, crypto.createHash('sha256').update(expected).digest('hex'));
    assert.equal(
      verified.asset.url,
      'https://github.com/theedward/astral/releases/download/v2.3.4/' + file,
    );
  }
});
