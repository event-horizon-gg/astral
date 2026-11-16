import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execute = promisify(execFile);

export function macSigningIdentity(env = process.env, { release = false } = {}) {
  const identity = env.ASTRAL_MAC_SIGN_IDENTITY?.trim() || '-';
  if (/[\x00-\x1f\x7f]/.test(identity) || (identity.startsWith('-') && identity !== '-'))
    throw Error('Invalid macOS signing identity.');
  if (release && !/^[a-f0-9]{40}$/i.test(identity))
    throw Error(
      'Set ASTRAL_MAC_SIGN_IDENTITY to the persistent signing certificate’s SHA-1 fingerprint before publishing.',
    );
  return identity;
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
