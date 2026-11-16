import assert from 'node:assert/strict';
import { mkdir, mkdtemp, realpath, rm, symlink } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import reference from '../desktop/ui-fixture.cjs';

test('Electron comparison uses temporary data and accepts only fixture clients and read actions', async (t) => {
  const base = await mkdtemp(path.join(os.tmpdir(), 'astral-reference-'));
  t.after(() => rm(base, { recursive: true, force: true }));
  const fixture = path.join(await realpath(base), 'fixture');
  const retail = path.join(fixture, '_retail_');
  const addons = path.join(retail, 'Interface', 'AddOns');
  const external = path.join(base, 'external');
  await mkdir(addons, { recursive: true });
  await mkdir(path.join(external, 'AddOns'), { recursive: true });
  const forever = path.join(fixture, '_forever_');
  await mkdir(forever);
  await symlink(external, path.join(forever, 'Interface'), 'junction');
  const paths = {};
  const app = { isPackaged: false, setPath: (key, value) => (paths[key] = value) };
  assert.equal(reference.createReferenceFixture(app, []), null);
  assert.throws(() =>
    reference.createReferenceFixture({ ...app, isPackaged: true }, ['--ui-fixture', fixture]),
  );
  assert.throws(() =>
    reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--daily-update']),
  );
  const mode = reference.createReferenceFixture(app, ['--ui-fixture', fixture]);
  assert.deepEqual([...mode.roots], [retail]);
  assert.equal(mode.root, retail);
  assert.equal(mode.theme, 'dark');
  assert.deepEqual(mode.viewport, { width: 1250, height: 840 });
  assert.equal(path.dirname(paths.userData), fixture);
  assert.equal(paths.sessionData, paths.userData);
  assert.equal(mode.allowsAddonDirectory(addons), true);
  assert.equal(mode.allowsAddonDirectory(path.join(forever, 'Interface', 'AddOns')), false);
  assert.doesNotThrow(() => mode.assertAction('state'));
  assert.doesNotThrow(() => mode.assertAction('client'));
  for (const action of ['startup', 'chooseRoot', 'update', 'connect', 'sync', 'installAppUpdate'])
    assert.throws(() => mode.assertAction(action), /unavailable/);
  const conflict = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--sync-conflict',
  ]);
  assert.deepEqual(conflict.dialog, { conflict: true });
  const review = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--profile-review',
  ]);
  assert.equal(review.page, 'Your setup');
  assert.equal(review.dialog, null);
  assert.equal(review.profileReview.name, 'Handheld');
  assert.equal(review.profileReview.handheld, 'retail');
  assert.equal(review.profileReview.packages[0].name, 'ConsolePort');
  assert.throws(() => review.assertAction('applyProfile'), /unavailable/);
  assert.throws(() =>
    reference.createReferenceFixture(app, [
      '--ui-fixture',
      fixture,
      '--profile-review',
      '--sync-conflict',
    ]),
  );
  assert.throws(() => conflict.assertAction('sync'), /unavailable/);
  const missing = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--sync-missing',
  ]);
  assert.equal(missing.dialog.missing[0].folder, 'ExampleAddon');
  const light = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--retail',
    '--light',
    '--sync-conflict',
  ]);
  assert.equal(light.theme, 'light');
  assert.equal(light.root, retail);
  assert.deepEqual(light.dialog, { conflict: true });
  assert.throws(
    () => reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--forever']),
    /missing/,
  );
  assert.throws(() =>
    reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--light', '--dark']),
  );
  const compact = reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--compact']);
  assert.deepEqual(compact.viewport, { width: 1000, height: 720 });
  const wide = reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--wide']);
  assert.deepEqual(wide.viewport, { width: 1920, height: 1000 });
  assert.throws(() =>
    reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--compact', '--wide']),
  );
  const signed = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--account',
    '--signed-in',
  ]);
  assert.equal(signed.accountName, 'AstralPlayer#1234');
  assert.equal(signed.response('syncProfiles').profiles.length, 3);
  assert.equal(signed.response('cloudInfo').devices[1].label, 'ROG Ally');
  assert.equal(signed.response('cloudInfo').devices[0].last_seen, 1794690000);
  assert.equal(signed.response('cloudInfo').history[0].revision, 2);
  assert.deepEqual(signed.response('sharedLibrary'), { addons: null });
  for (const action of [
    'createSyncProfile',
    'renameSyncProfile',
    'deleteSyncProfile',
    'sync',
    'connect',
    'revokeDevice',
  ]) {
    assert.equal(signed.response(action), undefined);
    assert.throws(() => signed.assertAction(action), /unavailable/);
  }
  assert.throws(() =>
    reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--signed-in', '--signed-in']),
  );
  const setup = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--account',
    '--compact',
  ]);
  assert.equal(setup.page, 'Your setup');
  assert.throws(() =>
    reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--settings', '--activity']),
  );
  const notice = reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--notice']);
  assert.deepEqual(notice.toast, {
    message: 'Cloud backup restored. Local backup saved.',
    error: false,
  });
  const noticeError = reference.createReferenceFixture(app, [
    '--ui-fixture',
    fixture,
    '--notice-error',
  ]);
  assert.equal(noticeError.toast.error, true);
  assert.throws(() =>
    reference.createReferenceFixture(app, ['--ui-fixture', fixture, '--notice', '--notice-error']),
  );
});
