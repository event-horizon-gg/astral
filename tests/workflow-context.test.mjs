import test from 'node:test';
import assert from 'node:assert/strict';
import cloudHandlers from '../desktop/cloud-handlers.cjs';
test('a conflict decision from another root/account is rejected before any cloud or file operation', async () => {
  const handlers = new Map(),
    calls = [];
  cloudHandlers.register({
    handle: (name, fn) => handlers.set(name, fn),
    runtime: {
      data: '/unused',
      config: { root: '/new/_retail_', account: 'NEW', syncProfile: 'regular' },
    },
    state: async () => ({}),
    persist: async () => {},
    send: () => {},
    session: { request: async (...args) => calls.push(args) },
    shell: {},
    busy: () => false,
  });
  await assert.rejects(
    handlers.get('sync')('cloud', 'retail', 3, 'same-hash', {
      root: '/old/_retail_',
      account: 'OLD',
      profile: 'regular',
    }),
    /selected setup changed/,
  );
  assert.deepEqual(calls, []);
});
