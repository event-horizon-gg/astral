const crypto = require('node:crypto');
const zlib = require('node:zlib');
const catalog = require('./catalog.cjs');
const selections = require('./addon-selection-data.cjs');
const library = require('./library-data.cjs');
const gamepad = require('./gamepad-settings.cjs');
const names = new Set(catalog.flatMap((a) => a.folders || [a.folder]));
const layoutNames = new Set([
  'bindings-cache.wtf',
  'layout-local.txt',
  'edit-mode-cache-account.txt',
  'edit-mode-cache-character.txt',
]);
function allowed(relative) {
  const parts = relative.split('/');
  if (
    parts.some((p) => !p || p === '.' || p === '..' || p.includes('\\') || p.includes(':')) ||
    parts.length > 5
  )
    return false;
  const name = parts.at(-1);
  if (layoutNames.has(name)) return parts.length === 1 || parts.length === 3;
  return (
    name.endsWith('.lua') &&
    parts.at(-2) === 'SavedVariables' &&
    (names.has(name.slice(0, -4)) ||
      name.startsWith('EllesmereUI') ||
      name.startsWith('Blizzard_')) &&
    (parts.length === 2 || parts.length === 4)
  );
}
function unpack(payload) {
  const raw = zlib.gunzipSync(Buffer.from(payload, 'base64'), { maxOutputLength: 24 * 1024 ** 2 });
  const bundle = JSON.parse(raw);
  if (![1, 2].includes(bundle.schema) || !Array.isArray(bundle.files) || bundle.files.length > 2000)
    throw Error('Unsupported settings format.');
  if (bundle.schema === 2) selections.validate(bundle.addonSelection);
  else if (bundle.addonSelection !== undefined) throw Error('Unsupported settings format.');
  if (bundle.library !== undefined) {
    if (bundle.schema !== 2) throw Error('Unsupported settings format.');
    library.validate(bundle.library);
  }
  if (bundle.input !== undefined) {
    if (bundle.schema !== 2) throw Error('Unsupported settings format.');
    gamepad.validate(bundle.input);
  }
  const seen = new Set();
  for (const f of bundle.files) {
    if (
      typeof f.path !== 'string' ||
      !allowed(f.path) ||
      seen.has(f.path) ||
      typeof f.content !== 'string'
    )
      throw Error('Invalid file in settings snapshot.');
    seen.add(f.path);
    const bytes = Buffer.from(f.content, 'base64');
    if (crypto.createHash('sha256').update(bytes).digest('hex') !== f.sha256)
      throw Error('Settings checksum mismatch.');
  }
  return { bundle, raw };
}
function decode(payload) {
  return unpack(payload).bundle;
}
function inspect(payload) {
  const { bundle, raw } = unpack(payload);
  return {
    hash: crypto.createHash('sha256').update(raw).digest('hex'),
    files: bundle.files.length + (bundle.schema === 2 ? 1 : 0),
    bytes: raw.length,
  };
}
function encode(bundle) {
  const raw = Buffer.from(JSON.stringify(bundle));
  if (raw.length > 24 * 1024 ** 2) throw Error('This setup exceeds the 24 MB preview limit.');
  const payload = zlib.gzipSync(raw).toString('base64');
  const validated = unpack(payload).bundle;
  return {
    payload,
    hash: crypto.createHash('sha256').update(raw).digest('hex'),
    files: validated.files.length + (validated.schema === 2 ? 1 : 0),
    bytes: raw.length,
  };
}
module.exports = { allowed, decode, inspect, encode };
