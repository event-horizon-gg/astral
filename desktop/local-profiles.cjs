const path = require('node:path');
const crypto = require('node:crypto');
const fs = require('node:fs/promises');
const engine = require('./engine.cjs');
const storage = require('./storage.cjs');
const format = require('./setup-format.cjs');
const settings = require('./settings.cjs');
const selection = require('./addon-selection.cjs');
const policy = require('./profile-policy.cjs');
const packages = require('./library-sync.cjs');
const setup = require('./setup-application.cjs');
const { validId } = require('../shared/profiles.cjs');
function location(data, root, account, id) {
  if (!validId(id)) throw Error('Unknown profile.');
  const key = crypto
    .createHash('sha256')
    .update(JSON.stringify([path.resolve(root), account]))
    .digest('hex');
  return path.join(data, 'local-profiles', key, id + '.json');
}
async function read(data, root, account, id) {
  const saved = await storage.json(location(data, root, account, id), null);
  if (saved && format.inspect(saved.payload).hash !== saved.hash)
    throw Error('The saved local profile checksum does not match.');
  return saved;
}
async function prepare({ data, root, account, from, to, edits = {} }) {
  if (!validId(from) || !validId(to)) throw Error('Unknown profile.');
  if (!edits || typeof edits !== 'object' || Array.isArray(edits))
    throw Error('Invalid addon choices.');
  const current = await settings.capture(root, account, data);
  const saved = from === to ? current : await read(data, root, account, to);
  const currentBundle = format.decode(current.payload);
  let bundle = format.decode((saved || current).payload);
  const inventory = await engine.inventory(root, data);
  if (Object.keys(edits).length) {
    bundle.schema = 2;
    bundle.addonSelection = selection.edit(
      bundle.addonSelection || currentBundle.addonSelection,
      inventory.addons,
      edits,
    );
  }
  bundle = policy.apply(bundle, {
    root,
    profile: to,
    inventory,
    currentInput: currentBundle.input,
  });
  const target = format.encode(bundle);
  const packagePlan = await packages.plan({
    root,
    data,
    target: packages.required(bundle.library || [], bundle.addonSelection),
  });
  const base = await settings.accountRoot(root, account);
  if (bundle.addonSelection) await selection.plan(root, base, bundle.addonSelection);
  const before = selection.rows(currentBundle.addonSelection, inventory.addons);
  const after = selection.rows(
    bundle.addonSelection || currentBundle.addonSelection,
    inventory.addons,
  );
  const changes = after
    .filter((row) => before.find((old) => old.id === row.id)?.enabled !== row.enabled)
    .map((row) => ({ ...row, before: before.find((old) => old.id === row.id)?.enabled }));
  const oldFiles = new Map(currentBundle.files.map((file) => [file.path, file.sha256]));
  const changedSettings = bundle.files
    .filter((file) => oldFiles.get(file.path) !== file.sha256)
    .map((file) => file.path);
  if (JSON.stringify(bundle.input) !== JSON.stringify(currentBundle.input))
    changedSettings.push('Controller settings');
  const fingerprint = crypto
    .createHash('sha256')
    .update(
      JSON.stringify({
        current: current.hash,
        target: target.hash,
        inventory: inventory.addons.map(({ id, modules, version }) => ({ id, modules, version })),
      }),
    )
    .digest('hex');
  return {
    fingerprint,
    current,
    target,
    packages: packagePlan,
    initialized: !saved,
    changes,
    changedSettings,
    rows: after,
  };
}
async function apply({
  data,
  root,
  account,
  from,
  to,
  edits = {},
  expected,
  running,
  commit = async () => {},
  onProgress = () => {},
  installPackages = packages.install,
}) {
  if (await running()) throw Error('Close WoW before changing profiles.');
  const plan = await prepare({ data, root, account, from, to, edits });
  if (plan.fingerprint !== expected) throw Error('The setup changed. Review the profile again.');
  if (plan.packages.manual.length)
    throw Error(
      'Install or connect these addons before switching: ' +
        plan.packages.manual.map((a) => a.name).join(', '),
    );
  const sourceFile = location(data, root, account, from),
    targetFile = location(data, root, account, to);
  const previousTarget = await fs.readFile(targetFile).catch((error) => {
    if (error.code === 'ENOENT') return null;
    throw error;
  });
  await storage.save(sourceFile, plan.current);
  try {
    await setup.installPackages({
      root,
      account,
      data,
      plan: plan.packages,
      current: plan.current,
      running,
      onProgress,
      library: { install: installPackages },
    });
    await settings.apply(root, account, plan.target.payload, data, running, async () => {
      await storage.save(targetFile, plan.target);
      await commit();
    });
  } catch (error) {
    if (from === to) await storage.save(targetFile, plan.current);
    else if (previousTarget) await fs.writeFile(targetFile, previousTarget);
    else if (from !== to) await fs.rm(targetFile, { force: true });
    throw error;
  }
  return plan;
}
module.exports = { prepare, apply, read, validId };
