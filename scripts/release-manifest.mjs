import fs from 'node:fs/promises';
import path from 'node:path';
import { createReadStream } from 'node:fs';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const native = args.includes('--native');
const directory = args.find((arg) => arg !== '--native') || path.join(root, 'release');
const version = native
  ? (await fs.readFile(path.join(root, 'native/Cargo.toml'), 'utf8'))
      .match(/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/m)?.[1]
  : JSON.parse(await fs.readFile(path.join(root, 'package.json'))).version;
if (!version) throw Error('A stable application version is required.');
const tag = (native ? 'native-v' : 'v') + version;
if (!process.env.ASTRAL_RELEASE_PRIVATE_KEY) throw Error('ASTRAL_RELEASE_PRIVATE_KEY is required.');
const publicKey = await fs.readFile(path.join(root, 'desktop/update-public-key.pem'));
const key = crypto.createPrivateKey(process.env.ASTRAL_RELEASE_PRIVATE_KEY);
const expectedKey = crypto.createPublicKey(publicKey).export({ type: 'spki', format: 'der' });
if (!crypto.createPublicKey(key).export({ type: 'spki', format: 'der' }).equals(expectedKey))
  throw Error('Release key does not match the pinned application key.');
const assets = {};
const packages = native ? [
  ['darwin-arm64', `Astral-Native-${version}-mac-arm64.zip`],
  ['win32-x64', `Astral-Native-${version}-win-x64.zip`],
] : [
  ['darwin-arm64', `Astral-${version}-mac-arm64.zip`],
  ['win32-x64', `Astral-${version}-win-x64.exe`],
];
for (const [id, name] of packages) {
  const filename = path.join(directory, name);
  const hash = crypto.createHash('sha256');
  let size = 0;
  for await (const chunk of createReadStream(filename)) {
    size += chunk.length;
    if (size > 600 * 1024 * 1024) throw Error('Release package exceeds the size limit.');
    hash.update(chunk);
  }
  if (size === 0) throw Error('Release package is empty.');
  assets[id] = {
    url: `https://github.com/theedward/astral/releases/download/${tag}/${name}`,
    size,
    sha256: hash.digest('hex'),
  };
}
const notes = await fs.readFile(path.join(root, 'CHANGELOG.md'), 'utf8');
const payload = Buffer.from(
  JSON.stringify({
    schema: 1,
    repository: 'theedward/astral',
    ...(native ? { app: 'astral-native' } : {}),
    version,
    notes: notes.split(/^## /m)[1]?.slice(0, 20000) || '',
    assets,
  }),
);
await fs.writeFile(
  path.join(directory, native ? 'astral-native-update.json' : 'astral-update.json'),
  JSON.stringify({
    payload: payload.toString('base64'),
    signature: crypto.sign(null, payload, key).toString('base64'),
  }) + '\n',
);
console.log(`Signed update manifest for Astral ${version}.`);
