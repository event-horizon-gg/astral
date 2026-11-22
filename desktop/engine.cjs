const { definitions, register, inventory, validDefinition } = require('./addon-inventory.cjs');
const github = require('./github.cjs');
const { toc, allowedFolder, targetInterface, inspectPackage } = require('./addon-metadata.cjs');
const fs = require('node:fs/promises'),
  path = require('node:path');
const crypto = require('node:crypto'),
  { execFile } = require('node:child_process'),
  { promisify } = require('node:util');
const run = promisify(execFile),
  yauzl = require('yauzl');
const catalog = require('./catalog.cjs');
const { flavor, allowed: allowedFlavor } = require('./flavors.cjs');
const { exists, json, save } = require('./storage.cjs');
const { newer, needsUpdate } = require('./addon-versions.cjs');
async function gameRunning() {
  if (process.platform === 'win32') {
    const { stdout } = await run('tasklist', ['/FO', 'CSV', '/NH'], { windowsHide: true });
    return /^"(?:Wow|Wow-64|WowClassic|WowF|WowForever)\.exe"/im.test(stdout);
  }
  try {
    await run('/usr/bin/pgrep', ['-x', 'Wow|World of Warcraft']);
    return true;
  } catch (e) {
    if (e.code === 1) return false;
    throw Error('Could not check whether WoW is running.');
  }
}
async function defaultRoot() {
  if (process.platform === 'darwin') return '/Applications/World of Warcraft/_retail_';
  for (const base of [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, 'C:\\Games']) {
    if (base) {
      const root = path.join(base, 'World of Warcraft', '_retail_');
      if (await exists(path.join(root, 'Interface', 'AddOns'))) return root;
    }
  }
  return '';
}
async function getRelease(addon, data, force = false) {
  if (addon.bundled) return require('./bundled.cjs').release(addon);
  if (!addon.repo) throw Error('Automatic downloads are not connected for this source.');
  const file = path.join(data, 'releases', addon.id + '.json');
  const cached = await json(file, null);
  if (cached && !force && Date.now() - cached.checked < 15 * 60e3) return cached;
  const r = await github.latest(addon.repo, {
    unavailableMessage: (status) => 'Could not check ' + addon.name + ' (' + status + ').',
  });
  const matching = r.assets?.filter((a) => new RegExp(addon.asset, 'i').test(a.name)) || [];
  if (addon.dynamic && matching.length !== 1)
    throw Error('Review this project’s release packages in Discover before updating.');
  const asset = matching[0];
  if (!asset) throw Error('No packaged release available from this repository.');
  const prefix = 'https://github.com/' + addon.repo + '/releases/download/';
  if (!asset.browser_download_url.startsWith(prefix))
    throw Error('Unexpected release download address.');
  const result = {
    tag: r.tag_name,
    version: r.tag_name.replace(
      addon.versionScheme === 'questie-bundle' ? /^bundle\/v/ : /^v\.?\s*/,
      '',
    ),
    url: asset.browser_download_url,
    size: asset.size,
    digest: asset.digest || null,
    checked: Date.now(),
    published: r.published_at,
    notes: r.body?.slice(0, 12000) || '',
  };
  await save(file, result);
  return result;
}
async function unzip(archive, destination, allowed) {
  await fs.mkdir(destination, { recursive: true });
  return new Promise((resolve, reject) =>
    yauzl.open(archive, { lazyEntries: true, validateEntrySizes: true }, (err, z) => {
      if (err) return reject(err);
      let bytes = 0;
      const fail = (e) => {
        z.close();
        reject(e);
      };
      z.on('error', fail);
      z.on('end', resolve);
      z.on('entry', async (entry) => {
        try {
          const name = entry.fileName;
          const parts = name.split('/');
          if (
            name.startsWith('/') ||
            name.includes('\\') ||
            parts.includes('..') ||
            parts.includes('.') ||
            !allowed(parts[0]) ||
            ((entry.externalFileAttributes >>> 16) & 0xf000) === 0xa000
          )
            throw Error('The release contains an unsafe or unexpected path.');
          bytes += entry.uncompressedSize;
          if (bytes > 1024 ** 3) throw Error('Release exceeds the installation size limit.');
          const output = path.join(destination, ...parts);
          if (name.endsWith('/')) {
            await fs.mkdir(output, { recursive: true });
            z.readEntry();
            return;
          }
          await fs.mkdir(path.dirname(output), { recursive: true });
          z.openReadStream(entry, async (error, stream) => {
            if (error) return fail(error);
            try {
              const { pipeline } = require('node:stream/promises');
              const { createWriteStream } = require('node:fs');
              await pipeline(stream, createWriteStream(output, { flags: 'wx' }));
              z.readEntry();
            } catch (e) {
              fail(e);
            }
          });
        } catch (e) {
          fail(e);
        }
      });
      z.readEntry();
    }),
  );
}
async function download(release, file, onProgress = () => {}) {
  const r = await fetch(release.url, { signal: AbortSignal.timeout(180000) });
  if (!r.ok) throw Error('Release download failed (' + r.status + ').');
  const out = await fs.open(file, 'wx');
  const hash = crypto.createHash('sha256');
  let bytes = 0;
  try {
    for await (const chunk of r.body) {
      bytes += chunk.length;
      if (bytes > 512 * 1024 ** 2) throw Error('Download exceeds the size limit.');
      await out.write(chunk);
      hash.update(chunk);
      onProgress(release.size ? Math.min(85, Math.round((bytes / release.size) * 85)) : 30);
    }
  } finally {
    await out.close();
  }
  if (release.size && bytes !== release.size) throw Error('Download incomplete.');
  if (release.digest?.startsWith('sha256:') && hash.digest('hex') !== release.digest.slice(7))
    throw Error('Release checksum mismatch.');
}
async function install(addon, root, data, release, onProgress = () => {}, options = {}) {
  const running = options.running || gameRunning;
  if (!allowedFlavor(addon, flavor(root)))
    throw Error('This addon is not supported for the selected game client.');
  if (await running())
    throw Object.assign(Error('Close World of Warcraft before changing addon files.'), {
      code: 'WOW_RUNNING',
    });
  if (addon.requires) {
    for (const folder of addon.requires)
      if (!(await exists(path.join(root, 'Interface', 'AddOns', folder))))
        throw Error('Install ' + folder + ' first.');
  }
  const addons = path.join(root, 'Interface', 'AddOns');
  if (!(await exists(addons))) throw Error('Select your WoW installation folder first.');
  const lock = path.join(data, 'install.lock');
  await fs.mkdir(data, { recursive: true });
  try {
    await fs.mkdir(lock);
  } catch {
    throw Error('Another installation is running.');
  }
  let temp;
  try {
    temp = await fs.mkdtemp(path.join(root, 'Interface', '.astral-'));
    const archive = path.join(temp, 'release.zip');
    const unpacked = path.join(temp, 'unpacked');
    if (addon.bundled) {
      await require('./bundled.cjs').prepare(addon, unpacked);
    } else {
      await (options.download || download)(release, archive, onProgress);
      await unzip(archive, unpacked, (n) => allowedFolder(addon, n));
    }
    let names = (await fs.readdir(unpacked, { withFileTypes: true }))
      .filter((e) => e.isDirectory())
      .map((e) => e.name);
    names = (await inspectPackage({ unpacked, root, addon, folders: names })).names;
    for (const n of names) {
      const old = path.join(addons, n);
      if ((await fs.lstat(old).catch(() => null))?.isSymbolicLink())
        throw Error('Linked addon folders need manual review.');
    }

    onProgress(90);
    if (await running())
      throw Object.assign(Error('WoW started during preparation. Update deferred.'), {
        code: 'WOW_RUNNING',
      });
    const id = Date.now() + '-' + addon.id;
    const backup = path.join(data, 'backups', id);
    await fs.mkdir(backup, { recursive: true });
    const moved = [],
      added = [];
    try {
      for (const n of names) {
        const dest = path.join(addons, n);
        if (await exists(dest)) {
          await fs.cp(dest, path.join(backup, n), { recursive: true });
          await fs.rename(dest, path.join(temp, 'old-' + n));
          moved.push(n);
        }
        await (options.rename || fs.rename)(path.join(unpacked, n), dest);
        added.push(n);
      }
      await save(path.join(backup, 'manifest.json'), {
        id,
        root: path.resolve(root),
        flavor: flavor(root),
        addonId: addon.id,
        name: addon.name,
        version: addon.version || null,
        release: release.tag,
        notes: release.notes || '',
        definition: addon.dynamic
          ? {
              id: addon.id,
              name: addon.name,
              folder: addon.folder,
              folders: addon.folders,
              repo: addon.repo,
            }
          : undefined,
        folders: names,
        previousFolders: moved,
        time: new Date().toISOString(),
      });
    } catch (e) {
      for (const n of added) await fs.rm(path.join(addons, n), { recursive: true, force: true });
      for (const n of moved) await fs.rename(path.join(temp, 'old-' + n), path.join(addons, n));
      throw e;
    }
    onProgress(100);
    return { id, version: release.version };
  } finally {
    if (temp) await fs.rm(temp, { recursive: true, force: true });
    await fs.rm(lock, { recursive: true, force: true });
  }
}
async function importZip(id, root, data, archive, onProgress = () => {}, options = {}) {
  const source = (await definitions(root, data)).find((a) => a.id === id);
  if (!source || source.local || source.bundled)
    throw Error('ZIP updates are available for supported third-party addons.');
  if (!allowedFlavor(source, flavor(root)))
    throw Error('This addon is not supported for the selected game client.');
  if (typeof archive !== 'string' || path.extname(archive).toLowerCase() !== '.zip')
    throw Error('Choose an addon ZIP downloaded from its official source.');
  const stat = await fs.stat(archive);
  if (!stat.isFile() || stat.size > 1024 ** 3)
    throw Error('Choose an addon ZIP smaller than 1 GB.');
  const current = (await inventory(root, data)).addons.find((a) => a.id === id);
  if (!current) throw Error('Select an installed addon before importing its update.');
  return install(
    current,
    root,
    data,
    { tag: 'ZIP: ' + path.basename(archive), version: 'Imported ZIP' },
    onProgress,
    {
      ...options,
      // Use the selected local archive; downloads and renderer-provided paths are never used.
      download: async (_, destination) => fs.copyFile(archive, destination),
    },
  );
}
async function backups(data, root) {
  const dir = path.join(data, 'backups');
  const files = await fs.readdir(dir).catch(() => []);
  const list = await Promise.all(files.map((f) => json(path.join(dir, f, 'manifest.json'), null)));
  return list
    .filter((m) => m && (!root || m.root === path.resolve(root)))
    .sort((a, b) => b.time.localeCompare(a.time));
}
async function restore(root, data, id, options = {}) {
  if (!/^[0-9]+-[a-z0-9-]+$/.test(id)) throw Error('Invalid backup.');
  if (await (options.running || gameRunning)()) throw Error('Close WoW before restoring.');
  const backup = path.join(data, 'backups', id),
    m = await json(path.join(backup, 'manifest.json'), null);
  if (!m) throw Error('Backup not found.');
  if (m.root !== path.resolve(root) || m.flavor !== flavor(root))
    throw Error('This backup belongs to another game installation.');
  const addon =
    (await definitions(root, data)).find((a) => a.id === m.addonId) ||
    (validDefinition(m.definition) ? m.definition : null);
  if (!addon) throw Error('Unknown backup source.');
  for (const n of m.folders)
    if (!/^[\w-]+$/.test(n) || !allowedFolder(addon, n)) throw Error('Invalid backup folder.');
  const addons = path.join(root, 'Interface', 'AddOns'),
    stage = await fs.mkdtemp(path.join(root, 'Interface', '.astral-restore-'));
  const touched = [];
  try {
    for (const n of m.folders) {
      const current = path.join(addons, n),
        had = await exists(current);
      if (had) await fs.rename(current, path.join(stage, n));
      touched.push({ n, had });
      if (m.previousFolders.includes(n))
        await (options.copy || fs.cp)(path.join(backup, n), current, { recursive: true });
    }
  } catch (error) {
    for (const { n, had } of touched.reverse()) {
      await fs.rm(path.join(addons, n), { recursive: true, force: true });
      if (had) await fs.rename(path.join(stage, n), path.join(addons, n));
    }
    throw error;
  } finally {
    await fs.rm(stage, { recursive: true, force: true });
  }
}

module.exports = {
  catalog,
  definitions,
  register,
  targetInterface,
  allowedFolder,
  download,
  toc,
  inventory,
  defaultRoot,
  gameRunning,
  getRelease,
  needsUpdate,
  newer,
  unzip,
  install,
  backups,
  importZip,
  restore,
  json,
  save,
  exists,
};
