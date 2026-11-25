// Signs and publishes the draft CI prepared. Runs in CI's `publish` job, with the key in
// ASTRAL_RELEASE_PRIVATE_KEY, or locally with the key file as the argument.
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import releaseContract from '../shared/release-contract.cjs';
const { REPOSITORY, RELEASE_TARGETS, MAX_INSTALLER_BYTES, installerName } = releaseContract;
const run = promisify(execFile);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const keyFile = process.argv[2];
if (!keyFile && !process.env.ASTRAL_RELEASE_PRIVATE_KEY)
  throw Error('Usage: node scripts/publish-release.mjs /path/to/local-private-key.pem (or set ASTRAL_RELEASE_PRIVATE_KEY)');
if (keyFile) {
  const keyInfo = await fs.stat(keyFile);
  if (process.platform !== 'win32' && keyInfo.mode & 0o077)
    throw Error('The release key must be readable only by its owner (chmod 600).');
}
const privateKey = keyFile ? await fs.readFile(keyFile, 'utf8') : process.env.ASTRAL_RELEASE_PRIVATE_KEY;
const { version } = JSON.parse(await fs.readFile(path.join(root, 'package.json')));
const tag = 'v' + version,
  repo = REPOSITORY;
const gh = async (args) =>
  (await run('gh', [...args, '--repo', repo], { maxBuffer: 1024 * 1024 })).stdout;
const release = JSON.parse(await gh(['release', 'view', tag, '--json', 'tagName,isDraft,assets']));
if (release.tagName !== tag || !release.isDraft)
  throw Error('Only the matching draft release may be signed and published.');
const names = RELEASE_TARGETS.map(({ platform, arch }) => installerName(version, platform, arch));
for (const name of names)
  if (!release.assets.some((a) => a.name === name && a.size > 0 && a.size <= MAX_INSTALLER_BYTES))
    throw Error('Both platform installers must be present before publication.');
const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-publish-'));
try {
  await gh(['release', 'download', tag, '--dir', temp, ...names.flatMap((n) => ['--pattern', n])]);
  await run(process.execPath, [path.join(root, 'scripts/release-manifest.mjs'), temp], {
    env: { ...process.env, ASTRAL_RELEASE_PRIVATE_KEY: privateKey },
  });
  await gh(['release', 'upload', tag, path.join(temp, 'astral-update.json'), '--clobber']);
  await gh(['release', 'edit', tag, '--draft=false', '--latest']);
  console.log(`Published verified release: https://github.com/${repo}/releases/tag/${tag}`);
} finally {
  await fs.rm(temp, { recursive: true, force: true });
}
