import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import crypto from 'node:crypto';
import path from 'node:path';
import os from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import {
  macSigningIdentity as resolveIdentity,
  saveMacSigningIdentity,
  verifyStableMacApp,
} from '../scripts/mac-signing.mjs';

const macSigningIdentity = (env, options = {}) =>
  resolveIdentity(env, { configPath: null, ...options });
const execute = promisify(execFile);

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

test('the saved public fingerprint is reused and invalid configuration never falls back to ad hoc', async (t) => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-signing-pin-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const configPath = path.join(directory, 'certificate');
  assert.equal(macSigningIdentity({}, { configPath }), '-');
  await saveMacSigningIdentity(identity.toUpperCase(), { configPath });
  assert.equal(await fs.readFile(configPath, 'utf8'), identity + '\n');
  assert.equal(macSigningIdentity({}, { configPath, release: true }), identity);
  assert.equal(
    macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: 'Explicit certificate' }, { configPath }),
    'Explicit certificate',
  );
  assert.equal(macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: '-' }, { configPath }), '-');
  assert.throws(
    () => macSigningIdentity({ ASTRAL_MAC_SIGN_IDENTITY: '-' }, { configPath, release: true }),
    /persistent/,
  );
  for (const content of [
    '',
    'unconfigured',
    '--sign-anything',
    identity + '\nextra',
    ' ' + identity,
  ]) {
    await fs.writeFile(configPath, content);
    assert.throws(() => macSigningIdentity({}, { configPath }), /Invalid/);
  }
  await assert.rejects(saveMacSigningIdentity('invalid', { configPath }), /fingerprint/);
  assert.equal(
    await fs.readFile(configPath, 'utf8'),
    ' ' + identity,
    'invalid input does not replace the previous file',
  );
  assert.deepEqual(await fs.readdir(directory), ['certificate']);
});

test(
  'native shell packaging resolves the same certificate and rejects missing persistent signing',
  { skip: process.platform === 'win32' },
  async (t) => {
    const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'astral-native-signing-pin-'));
    t.after(() => fs.rm(directory, { recursive: true, force: true }));
    const configPath = path.join(directory, 'certificate');
    const script = fileURLToPath(new URL('../scripts/mac-signing-identity.sh', import.meta.url));
    const resolve = async (value = '', required = false) =>
      (
        await execute('/bin/sh', [script, ...(required ? ['--persistent'] : []), configPath], {
          env: { ...process.env, ASTRAL_MAC_SIGN_IDENTITY: value },
        })
      ).stdout.trim();
    assert.equal(await resolve(), '-');
    await assert.rejects(resolve('', true));
    await saveMacSigningIdentity(identity, { configPath });
    assert.equal(await resolve('', true), identity);
    assert.equal(await resolve('  Explicit certificate  '), 'Explicit certificate');
    for (const value of ['--option', 'a\nb', '\n', 'a\tb']) await assert.rejects(resolve(value));
    for (const content of ['', 'not a certificate', identity + '\nextra', ' ' + identity]) {
      await fs.writeFile(configPath, content);
      await assert.rejects(resolve());
    }
    const bin = path.join(directory, 'bin');
    const marker = path.join(directory, 'unexpected-build');
    await fs.mkdir(bin);
    await fs.writeFile(
      path.join(bin, 'cargo'),
      '#!/bin/sh\nprintf called > "$ASTRAL_TEST_CARGO_MARKER"\nexit 88\n',
      { mode: 0o755 },
    );
    const packager = fileURLToPath(new URL('../native/package-mac.sh', import.meta.url));
    const env = {
      ...process.env,
      ASTRAL_MAC_SIGN_IDENTITY: '-',
      ASTRAL_TEST_CARGO_MARKER: marker,
      PATH: bin + path.delimiter + process.env.PATH,
    };
    await assert.rejects(execute('/bin/sh', [packager, '--persistent'], { env }), /persistent/);
    await assert.rejects(execute('/bin/sh', [packager, '--unexpected'], { env }), /Usage/);
    await assert.rejects(fs.access(marker), { code: 'ENOENT' });
  },
);

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
