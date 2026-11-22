import test from 'node:test';
import assert from 'node:assert/strict';
import { compare } from '../desktop/setup-comparison.cjs';
const file = (path, sha256) => ({ path, sha256, content: 'must never be shown in the review' });
const bundle = (files = [], extra = {}) => ({ files, ...extra });

test('comparison detects content changes even when file counts match and omits file contents', () => {
  const before = bundle([
    file('bindings-cache.wtf', 'old'),
    file('SavedVariables/EllesmereUI.lua', 'same'),
  ]);
  const after = bundle([
    file('bindings-cache.wtf', 'new'),
    file('SavedVariables/EllesmereUI.lua', 'same'),
  ]);
  const rows = compare(before, after);
  assert.equal(rows.find((r) => r.id === 'bindings').changed, true);
  assert.equal(rows.find((r) => r.id === 'addons').changed, false);
  assert.equal(JSON.stringify(rows).includes('must never'), false);
  assert.deepEqual(rows.find((r) => r.id === 'bindings').files, [
    { path: 'bindings-cache.wtf', local: true, remote: true },
  ]);
});
test('files and controller input omitted from a snapshot are kept, not shown as removals', () => {
  const rows = compare(
    bundle([file('Realm/Other/bindings-cache.wtf', 'keep')], { input: { GamePadEnable: '1' } }),
    bundle(),
  );
  assert.equal(rows.find((r) => r.id === 'bindings').remote, '1 file');
  assert.equal(
    rows.some((r) => r.changed),
    false,
  );
  assert.equal(rows.find((r) => r.id === 'controller').remote, 'On');
});
test('unknown controller state is not presented as disabled, and mapping changes remain visible', () => {
  const rows = compare(
    bundle([], { input: {} }),
    bundle([], { input: { GamePadEnable: '1' } }),
  );
  assert.equal(rows[0].local, 'Not recorded');
  assert.equal(rows[0].remote, 'On');
  const mappings = compare(
    bundle([], { input: { GamePadEnable: '1', GamePadEmulateShift: 'PADLTRIGGER' } }),
    bundle([], { input: { GamePadEnable: '1', GamePadEmulateShift: 'PADRTRIGGER' } }),
  );
  assert.equal(mappings[0].changed, true);
});
test('addon choices are compared by scope and folder rather than object ordering', () => {
  const before = bundle([], {
    addonSelection: { scopes: { account: { ConsolePort: true, Details: false } } },
  });
  const same = bundle([], {
    addonSelection: { scopes: { account: { Details: false, ConsolePort: true } } },
  });
  assert.equal(compare(before, same)[0].changed, false);
  same.addonSelection.scopes.account.ConsolePort = false;
  assert.equal(compare(before, same)[0].details, '1 choice differs');
});
test('character choices use the same account fallback and case-insensitive folder lookup as apply', () => {
  const before = bundle([], {
    addonSelection: {
      scopes: { '': { ConsolePort: false }, 'Realm/Tank': { ConsolePort: false } },
    },
  });
  const after = bundle([], { addonSelection: { scopes: { '': { consoleport: true } } } });
  const row = compare(before, after)[0];
  assert.equal(row.choices.length, 2);
  assert.deepEqual(
    row.choices.map((c) => [c.scope, c.local, c.remote]),
    [
      ['', 'Off', 'On'],
      ['Realm/Tank', 'Off', 'On'],
    ],
  );
  assert.equal(compare(before, bundle())[0].changed, false);
});
test('controller CVar case changes do not imply an input change', () => {
  assert.equal(
    compare(
      bundle([], { input: { gamepadenable: '1' } }),
      bundle([], { input: { GamePadEnable: '1' } }),
    )[0].changed,
    false,
  );
});
