const fs = require('node:fs/promises'),
  path = require('node:path'),
  crypto = require('node:crypto');
const engine = require('./engine.cjs'),
  { flavor } = require('./flavors.cjs');
const storage = require('./storage.cjs');
const metadata = require('./addon-metadata.cjs');
const github = require('./github.cjs');
async function releases(repo, request = fetch) {
  const release = await github.latest(repo, {
    request,
    timeout: 25000,
    unavailableMessage: 'No published packaged release is available for this project.',
  });
  if (release.draft || release.prerelease) throw Error('A stable packaged release is required.');
  return {
    repo,
    tag: release.tag_name,
    notes: (release.body || '').slice(0, 12000),
    published: release.published_at,
    assets: (release.assets || [])
      .filter(
        (a) =>
          a.state === 'uploaded' &&
          /\.zip$/i.test(a.name) &&
          a.size > 0 &&
          a.size <= 1024 ** 3 &&
          a.browser_download_url?.startsWith('https://github.com/' + repo + '/releases/download/'),
      )
      .map((a) => ({
        id: a.id,
        name: a.name,
        url: a.browser_download_url,
        size: a.size,
        digest: a.digest || null,
      })),
  };
}
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
async function inspect(repo, assetId, root, data, options = {}) {
  const release = await releases(repo, options.request),
    asset = release.assets.find((a) => a.id === assetId);
  if (!asset) throw Error('Choose a published addon ZIP.');
  await fs.mkdir(path.join(data, 'previews'), { recursive: true });
  const temp = await fs.mkdtemp(path.join(data, 'previews', 'package-'));
  try {
    const archive = path.join(temp, 'release.zip');
    await (options.download || engine.download)(asset, archive, () => {});
    const unpacked = path.join(temp, 'unpacked');
    await engine.unzip(
      archive,
      unpacked,
      (n) => /^[A-Za-z0-9_-]+$/.test(n) && !n.startsWith('Blizzard_'),
    );
    let names = (await fs.readdir(unpacked, { withFileTypes: true }))
      .filter((e) => e.isDirectory())
      .map((e) => e.name)
      .sort();
    if (!names.length || names.length > 64) throw Error('This is not a packaged addon release.');
    const curated = engine.catalog.find(
      (a) => a.repo?.toLowerCase() === repo.toLowerCase() && names.includes(a.folder),
    );
    names = metadata.packageNames(curated, names, flavor(root));
    for (const name of names) {
      const known = engine.catalog.find((addon) => metadata.allowedFolder(addon, name));
      if (known && known.repo?.toLowerCase() !== repo.toLowerCase())
        throw Error('This package would replace ' + known.name + ' from a different source.');
    }
    let inspected;
    try {
      inspected = await metadata.inspectPackage({ unpacked, root, addon: curated, folders: names });
    } catch (error) {
      if (error.code === 'INCOMPATIBLE_ADDON')
        throw Error(
          error.folder + ' does not declare support for this ' + error.client + ' build.',
        );
      throw error;
    }
    const target = inspected.interface;
    names = inspected.names;
    const details = await Promise.all(
      inspected.details.map(async (detail) => ({
        ...detail,
        version: detail.version || release.tag,
        replaces: await storage.exists(path.join(root, 'Interface', 'AddOns', detail.folder)),
      })),
    );
    const normalized = (s) => s.toLowerCase().replace(/[^a-z0-9]/g, '');
    const primary =
      details.find((a) => normalized(a.folder) === normalized(repo.split('/')[1])) || details[0];
    const known = curated;
    const versionPart = release.tag.replace(/^v/i, '');
    const pattern = escape(asset.name)
      .replace(escape(release.tag), '.*')
      .replace(escape(versionPart), '.*');
    const definition = known || {
      id:
        'gh-' +
        crypto
          .createHash('sha256')
          .update(repo.toLowerCase() + '|' + primary.folder + '|' + flavor(root))
          .digest('hex')
          .slice(0, 16),
      name: primary.name,
      folder: primary.folder,
      folders: details.map((d) => d.folder),
      repo,
      asset: '^' + pattern + '$',
      flavors: [flavor(root)],
      requires: [...new Set(details.flatMap((a) => a.requires))].filter((n) => !names.includes(n)),
      dynamic: true,
      category: 'Community',
      color: 'violet',
      description: 'Installed from ' + repo + ' on GitHub.',
    };
    if ((definition.requires || []).some((n) => !n.match(/^[A-Za-z0-9_-]+$/)))
      throw Error('Invalid dependency metadata.');
    return {
      root: path.resolve(root),
      temp,
      archive,
      definition,
      release: {
        ...asset,
        tag: release.tag,
        version: release.tag.replace(/^v/i, ''),
        notes: release.notes,
      },
      details,
      interface: target,
      repo,
      tag: release.tag,
      notes: release.notes,
      assetName: asset.name,
      created: Date.now(),
    };
  } catch (e) {
    await fs.rm(temp, { recursive: true, force: true });
    throw e;
  }
}
async function install(preview, root, data, onProgress = () => {}, options = {}) {
  if (preview.root !== path.resolve(root) || Date.now() - preview.created > 10 * 60000)
    throw Error('This package review expired or belongs to another game client. Review it again.');
  const result = await engine.install(preview.definition, root, data, preview.release, onProgress, {
    ...options,
    download: async (_, dest) => fs.copyFile(preview.archive, dest),
  });
  if (preview.definition.dynamic) await engine.register(root, data, preview.definition);
  return result;
}
module.exports = { releases, inspect, install };
