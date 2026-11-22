const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const { json, save } = require('./storage.cjs');

function location(root, data) {
  const key = crypto.createHash('sha256').update(path.resolve(root)).digest('hex');
  return path.join(data, 'installed-releases', key + '.json');
}
// File metadata detects external replacement without rereading large database addons.
async function fingerprint(root, folders) {
  const entries = [];
  async function visit(relative) {
    const stat = await fs.lstat(path.join(root, 'Interface', 'AddOns', relative));
    if (stat.isSymbolicLink()) throw Error('Linked addon content has no installation receipt.');
    if (stat.isDirectory()) {
      for (const name of (
        await fs.readdir(path.join(root, 'Interface', 'AddOns', relative))
      ).sort())
        await visit(path.join(relative, name));
    } else if (stat.isFile()) entries.push([relative, stat.size, stat.mtimeMs, stat.ctimeMs]);
  }
  for (const folder of [...folders].sort()) {
    if (!/^[A-Za-z0-9_-]+$/.test(folder)) throw Error('Invalid addon folder.');
    await visit(folder);
  }
  return crypto.createHash('sha256').update(JSON.stringify(entries)).digest('hex');
}
async function read(root, data) {
  return data ? json(location(root, data), {}) : {};
}
async function record(root, data, addon, release, folders) {
  if (!release.url || !addon.repo) return;
  const records = await read(root, data);
  records[addon.id] = {
    repo: addon.repo,
    tag: release.tag,
    version: release.version,
    folders,
    fingerprint: await fingerprint(root, folders),
  };
  await save(location(root, data), records);
}
async function installedVersion(root, addon, records) {
  const receipt = records?.[addon.id];
  if (
    !receipt ||
    receipt.repo !== addon.repo ||
    typeof receipt.version !== 'string' ||
    !Array.isArray(receipt.folders)
  )
    return null;
  try {
    return (await fingerprint(root, receipt.folders)) === receipt.fingerprint
      ? receipt.version
      : null;
  } catch {
    return null;
  }
}
module.exports = { read, record, installedVersion };
