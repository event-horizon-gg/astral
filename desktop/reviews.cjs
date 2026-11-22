const { randomUUID } = require('node:crypto');
function create({
  context,
  ttl,
  message,
  dispose = async () => {},
  now = Date.now,
  cleanupFailed = (error) => console.warn('Could not remove review files:', error.message),
}) {
  const entries = new Map();
  let generation = 0;
  const capture = () => structuredClone(context());
  const matches = (scope) => JSON.stringify(scope) === JSON.stringify(context());
  async function release(value) {
    try {
      await dispose(value);
    } catch (error) {
      cleanupFailed(error);
    }
  }
  async function clearEntries() {
    const removed = [...entries.values()];
    entries.clear();
    for (const entry of removed) await release(entry.value);
  }
  async function clear() {
    generation++;
    await clearEntries();
  }
  async function replace(value, scope = capture()) {
    const request = ++generation;
    await clearEntries();
    if (request !== generation || !matches(scope)) {
      await release(value);
      throw Error(message);
    }
    const token = randomUUID();
    entries.set(token, { value, scope, expires: now() + ttl });
    return token;
  }
  async function lookup(token, consume = false) {
    const entry = entries.get(token);
    if (!entry) throw Error(message);
    const invalid = entry.expires < now() || !matches(entry.scope);
    if (invalid || consume) entries.delete(token);
    if (invalid) {
      await release(entry.value);
      throw Error(message);
    }
    return entry.value;
  }
  const get = (token) => lookup(token);
  async function run(token, operation) {
    const value = await lookup(token, true);
    try {
      return await operation(value);
    } finally {
      await release(value);
    }
  }
  return { capture, replace, get, run, clear };
}
module.exports = { create };
