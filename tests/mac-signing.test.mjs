import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import crypto from 'node:crypto';
import { macSigningIdentity, verifyStableMacApp } from '../scripts/mac-signing.mjs';

const leaf = Buffer.from('synthetic public certificate fixture');
const identity = crypto.createHash('sha1').update(leaf).digest('hex');
const metadata = {
  CFBundleIdentifier: 'com.joaoseixas.astral.native',
  CFBundleShortVersionString: '0.1.0',
};

function verifier({ details, certificate = leaf, plist = metadata, rejectSignature = false } = {}) {
  const calls = [];
  const run = async (command, args) => {
    calls.push([command, ...args]);
    if (command === '/usr/bin/plutil') return { stdout: JSON.stringify(plist) };
    if (args.includes('--verify')) {
      if (rejectSignature) throw Error('Invalid code signature');
      return { stdout: '' };
    }
    const extraction = args.find((arg) => arg.startsWith('--extract-certificates='));
    if (extraction) {
      await fs.writeFile(extraction.slice('--extract-certificates='.length) + '0', certificate);
      return { stdout: '' };
    }
    return {
      stderr:
        details ??
        'Authority=Astral Local Development\ndesignated => identifier "com.joaoseixas.astral.native" and certificate leaf = H"public-certificate"',
    };
  };
  return {
    run,
    calls,
    identifier: metadata.CFBundleIdentifier,
    version: metadata.CFBundleShortVersionString,
  };
}

test('publication requires a pinned certificate; ad hoc remains available for isolated CI builds', () => {
  assert.equal(macSigningIdentity({}), '-');
  assert.equal(
    macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: 'Astral Local Development' }),
    'Astral Local Development',
  );
  assert.throws(() => macSigningIdentity({}, { release: true }), /persistent/);
  assert.throws(
    () => macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: '-' }, { release: true }),
    /persistent/,
  );
  assert.equal(
    macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: identity }, { release: true }),
    identity,
  );
  assert.throws(() => macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: '--anything' }), /Invalid/);
});

test('release verification accepts only the expected app, version and signing certificate', async () => {
  const valid = verifier();
  await verifyStableMacApp('package-under-test', identity, valid);
  const extracted = valid.calls
    .find((call) => call.some((arg) => arg.startsWith('--extract-certificates=')))
    .find((arg) => arg.startsWith('--extract-certificates='))
    .slice('--extract-certificates='.length);
  await assert.rejects(fs.access(extracted + '0'), { code: 'ENOENT' });
  for (const options of [
    { certificate: Buffer.from('another public certificate') },
    { plist: { ...metadata, CFBundleIdentifier: 'other.app' } },
    { plist: { ...metadata, CFBundleShortVersionString: '9.0.0' } },
    { rejectSignature: true },
    { details: 'Signature=adhoc\n# designated => cdhash H"one-build"' },
    { details: 'Authority=Astral Local Development\ndesignated => cdhash H"one-build"' },
    { details: 'Authority=Astral Local Development' },
    { details: 'Authority=Astral Local Development\ndesignated => true' },
    {
      details:
        'Authority=Astral Local Development\ndesignated => identifier "com.joaoseixas.astral.native"',
    },
    {
      details:
        'Authority=Astral Local Development\ndesignated => identifier "another.app" and certificate leaf = H"public-certificate"',
    },
  ]) {
    await assert.rejects(verifyStableMacApp('package-under-test', identity, verifier(options)));
  }
});
