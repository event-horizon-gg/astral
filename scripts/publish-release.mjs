// Run locally after CI prepares the draft. The private key never leaves this machine.
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { macSigningIdentity, verifyStableMacApp } from './mac-signing.mjs';
import { inspectZip } from '../desktop/app-updates.cjs';
const run = promisify(execFile);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const native = args.includes('--native');
const [keyFile, ...extra] = args.filter((arg) => arg !== '--native');
if (!keyFile || extra.length)
  throw Error('Usage: node scripts/publish-release.mjs [--native] /path/to/local-private-key.pem');
if (process.platform !== 'darwin')
  throw Error(
    'Publish from macOS so the Mac package’s persistent signing identity can be verified.',
  );
const signingIdentity = macSigningIdentity(process.env, { release: true });
const keyInfo = await fs.stat(keyFile);
if (process.platform !== 'win32' && keyInfo.mode & 0o077)
  throw Error('The release key must be readable only by its owner (chmod 600).');
const version = native
  ? (await fs.readFile(path.join(root, 'native/Cargo.toml'), 'utf8')).match(
      /^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/m,
    )?.[1]
  : JSON.parse(await fs.readFile(path.join(root, 'package.json'))).version;
if (!version) throw Error('A stable application version is required.');
const tag = (native ? 'native-v' : 'v') + version,
  repo = 'theedward/astral';
const gh = async (args) =>
  (await run('gh', [...args, '--repo', repo], { maxBuffer: 1024 * 1024 })).stdout;
const release = JSON.parse(
  await gh(['release', 'view', tag, '--json', 'tagName,isDraft,assets,databaseId']),
);
if (release.tagName !== tag || !release.isDraft)
  throw Error('Only the matching draft release may be signed and published.');
const names = native
  ? [`Astral-Native-${version}-mac-arm64.zip`, `Astral-Native-${version}-win-x64.zip`]
  : [`Astral-${version}-mac-arm64.zip`, `Astral-${version}-win-x64.exe`];
const manifest = native ? 'astral-native-update.json' : 'astral-update.json';
for (const name of names)
  if (!release.assets.some((a) => a.name === name && a.size > 0 && a.size <= 600 * 1024 * 1024))
    throw Error('Both platform installers must be present before publication.');
const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-publish-'));
try {
  await gh(['release', 'download', tag, '--dir', temp, ...names.flatMap((n) => ['--pattern', n])]);
  const extracted = path.join(temp, 'mac-package');
  const bundleName = native ? 'Astral Native.app' : 'Astral.app';
  await inspectZip(path.join(temp, names[0]), bundleName);
  await fs.mkdir(extracted);
  await run('/usr/bin/ditto', ['-xk', path.join(temp, names[0]), extracted]);
  await verifyStableMacApp(path.join(extracted, bundleName), signingIdentity, {
    identifier: native ? 'com.joaoseixas.astral.native' : 'com.joaoseixas.astral',
    version,
  });
  await run(
    process.execPath,
    [path.join(root, 'scripts/release-manifest.mjs'), ...(native ? ['--native'] : []), temp],
    {
      env: { ...process.env, ASTRAL_RELEASE_PRIVATE_KEY: await fs.readFile(keyFile, 'utf8') },
    },
  );
  await gh(['release', 'upload', tag, path.join(temp, manifest), '--clobber']);
  if (native) {
    if (!Number.isSafeInteger(release.databaseId) || release.databaseId <= 0)
      throw Error('The native draft release ID is missing.');
    await run('gh', [
      'api',
      '--method',
      'PATCH',
      `repos/${repo}/releases/${release.databaseId}`,
      '-F',
      'draft=false',
      '-f',
      'make_latest=false',
    ]);
  } else {
    await gh(['release', 'edit', tag, '--draft=false', '--latest']);
  }
  console.log(`Published verified release: https://github.com/${repo}/releases/tag/${tag}`);
} finally {
  await fs.rm(temp, { recursive: true, force: true });
}
