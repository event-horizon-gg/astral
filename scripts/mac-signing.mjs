import fs from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
export const macSigningConfig = fileURLToPath(
  new URL('../.astral-mac-certificate', import.meta.url),
);

export function macSigningIdentity(
  env = process.env,
  { release = false, configPath = macSigningConfig } = {},
) {
  const configured = env.ASTRAL_MAC_SIGN_IDENTITY || '';
  if (/[\x00-\x1f\x7f]/.test(configured)) throw Error('Invalid macOS signing identity.');
  let identity = configured.trim();
  if (!identity && configPath) {
    let pin;
    try {
      pin = readFileSync(configPath, 'utf8').replace(/\n+$/, '');
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
    if (pin !== undefined) {
      if (!/^[a-f0-9]{40}$/i.test(pin))
        throw Error('Invalid .astral-mac-certificate: expected a certificate fingerprint.');
      identity = pin;
    }
  }
  identity ||= '-';
  if (/[\x00-\x1f\x7f]/.test(identity) || (identity.startsWith('-') && identity !== '-'))
    throw Error('Invalid macOS signing identity.');
  if (release && !/^[a-f0-9]{40}$/i.test(identity))
    throw Error(
      'Configure a persistent macOS signing certificate before publishing or installing.',
    );
  return identity;
}

export async function saveMacSigningIdentity(identity, { configPath = macSigningConfig } = {}) {
  const pin = identity.trim();
  if (!/^[a-f0-9]{40}$/i.test(pin))
    throw Error('Expected the certificate’s 40-character SHA-1 fingerprint.');
  const staged = `${configPath}.${crypto.randomBytes(8).toString('hex')}.tmp`;
  try {
    await fs.writeFile(staged, pin.toLowerCase() + '\n', { flag: 'wx', mode: 0o600 });
    await fs.rename(staged, configPath);
  } finally {
    await fs.rm(staged, { force: true });
  }
}

export async function verifyStableMacApp(app, identity, { identifier, version, run = execute }) {
  if (!/^[a-f0-9]{40}$/i.test(identity))
    throw Error('A persistent signing certificate fingerprint is required.');
  const metadata = JSON.parse(
    (
      await run('/usr/bin/plutil', [
        '-convert',
        'json',
        '-o',
        '-',
        path.join(app, 'Contents/Info.plist'),
      ])
    ).stdout,
  );
  if (
    !identifier ||
    !version ||
    metadata.CFBundleIdentifier !== identifier ||
    metadata.CFBundleShortVersionString !== version
  )
    throw Error('The macOS application identity or version does not match the release.');
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]);
  const result = await run('/usr/bin/codesign', ['-d', '--verbose=4', '-r-', app]);
  const details = `${result.stdout || ''}\n${result.stderr || ''}`;
  if (/^Signature=adhoc$/m.test(details) || !/^Authority=.+$/m.test(details))
    throw Error(
      'The macOS app has a temporary signature. Upload a package signed with the persistent certificate.',
    );
  if (!/^#?\s*designated =>/m.test(details) || /^#?\s*designated =>.*\bcdhash\b/m.test(details))
    throw Error(
      'The macOS app’s identity is tied to one build instead of its signing certificate.',
    );
  const requirement = details.match(/^#?\s*designated =>(.+)$/m)?.[1] || '';
  if (
    !requirement.includes(`identifier "${identifier}"`) ||
    !/\b(?:anchor|certificate)\b/.test(requirement)
  )
    throw Error(
      'The signing requirement must bind the application identifier and its certificate.',
    );
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-signing-check-'));
  try {
    const prefix = path.join(directory, 'certificate');
    await run('/usr/bin/codesign', ['-d', `--extract-certificates=${prefix}`, app]);
    const certificate = await fs.readFile(prefix + '0');
    const fingerprint = crypto.createHash('sha1').update(certificate).digest('hex');
    if (fingerprint.toLowerCase() !== identity.toLowerCase())
      throw Error('The macOS package uses a different signing certificate.');
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
}
