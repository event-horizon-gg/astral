const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const os = require('node:os');
const storage = require('./storage.cjs');
function create({
  configuration,
  directory,
  credentials,
  changed,
  request: transport = fetch,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  const sessionFile = () => path.join(directory(), 'session.json');
  async function request(route, { method = 'GET', body, token = true } = {}) {
    if (!configuration().service)
      throw Error('Sign-in is temporarily unavailable. Please try again later.');
    const headers = { 'Content-Type': 'application/json' };
    if (token) {
      const stored = await storage.json(sessionFile(), null);
      if (!stored) throw Error('Sign in with Battle.net first.');
      headers.Authorization = 'Bearer ' + (await credentials.decrypt(stored.token));
    }
    const r = await transport(configuration().service + route, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(30000),
    });
    const result = await r.json().catch(() => ({ error: 'Unexpected service response.' }));
    if (!r.ok) {
      if (r.status === 401 && token) {
        await fs.rm(sessionFile(), { force: true });
        changed();
      }
      if (r.status === 409) {
        const e = Error(
          result.error || 'Both computers have changed. Review both versions before syncing.',
        );
        e.conflict = true;
        throw e;
      }
      throw Error(result.error || 'The sync service could not complete the request.');
    }
    return result;
  }

  async function connect(openExternal) {
    await credentials.available();
    const verifier = crypto.randomBytes(32).toString('base64url');
    const challenge = crypto.createHash('sha256').update(verifier).digest('base64url');
    const pair = await request('/auth/device', {
      method: 'POST',
      body: { challenge, label: os.hostname() },
      token: false,
    });
    if (!pair.url?.startsWith(configuration().service + '/auth/'))
      throw Error('Invalid sign-in address.');
    await openExternal(pair.url);
    for (let i = 0; i < 120; i++) {
      await sleep(2000);
      const result = await request('/auth/device/poll', {
        method: 'POST',
        body: { id: pair.id, verifier },
        token: false,
      });
      if (result.token) {
        await storage.save(sessionFile(), {
          token: await credentials.encrypt(result.token),
          name: result.name,
        });
        return result;
      }
    }
    throw Error('Sign-in expired. Try connecting again.');
  }
  const clear = () => fs.rm(sessionFile(), { force: true });
  async function disconnect() {
    try {
      await request('/auth/logout', { method: 'POST' });
    } finally {
      await clear();
    }
  }
  const exists = () => storage.exists(sessionFile());
  async function identity() {
    const value = await storage.json(sessionFile(), null);
    return value ? { name: value.name } : null;
  }
  return { request, connect, disconnect, clear, exists, identity };
}
module.exports = { create };
