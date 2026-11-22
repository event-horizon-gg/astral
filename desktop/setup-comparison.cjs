const { effective } = require('./addon-selection-data.cjs');
function fileGroup(path) {
  const name = path.split('/').at(-1);
  if (name === 'bindings-cache.wtf') return 'bindings';
  if (name.startsWith('layout-') || name.startsWith('edit-mode-')) return 'layout';
  return 'addons';
}
const count = (n, noun) => `${n} ${noun}${n === 1 ? '' : 's'}`;
function controller(bundle) {
  const entry = Object.entries(bundle.input || {}).find(
    ([key]) => key.toLowerCase() === 'gamepadenable',
  );
  return entry
    ? entry[1] === '1'
      ? 'On'
      : entry[1] === '0'
        ? 'Off'
        : 'Not recorded'
    : 'Not recorded';
}
function compare(before, after) {
  const rows = [];
  for (const [id, label] of [
    ['bindings', 'Keybindings'],
    ['layout', 'Blizzard layouts'],
    ['addons', 'Addon settings'],
  ]) {
    const left = new Map(
      before.files
        .filter((file) => fileGroup(file.path) === id)
        .map((file) => [file.path, file.sha256]),
    );
    // Applying a snapshot replaces its files; files absent from it stay local.
    const right = new Map([
      ...left,
      ...after.files
        .filter((file) => fileGroup(file.path) === id)
        .map((file) => [file.path, file.sha256]),
    ]);
    const paths = [...new Set([...left.keys(), ...right.keys()])].sort();
    if (!paths.length) continue;
    const files = paths
      .filter((path) => left.get(path) !== right.get(path))
      .map((path) => ({ path, local: left.has(path), remote: right.has(path) }));
    rows.push({
      id,
      label,
      local: count(left.size, 'file'),
      remote: count(right.size, 'file'),
      changed: !!files.length,
      files,
    });
  }
  const effectiveInput = after.input === undefined ? before.input : after.input;
  const inputKey = (values) =>
    JSON.stringify(
      Object.entries(values || {})
        .map(([key, value]) => [key.toLowerCase(), value])
        .sort(),
    );
  const inputChanged = inputKey(before.input) !== inputKey(effectiveInput);
  if (before.input !== undefined || after.input !== undefined)
    rows.push({
      id: 'controller',
      label: 'Controller',
      local: controller(before),
      remote: controller({ input: effectiveInput }),
      changed: inputChanged,
    });
  const beforeScopes = before.addonSelection?.scopes || {};
  const afterScopes = after.addonSelection?.scopes || beforeScopes;
  const scopes = new Set([...Object.keys(beforeScopes), ...Object.keys(afterScopes)]);
  const names = new Map();
  for (const values of [...Object.values(beforeScopes), ...Object.values(afterScopes)])
    for (const name of Object.keys(values)) names.set(name.toLowerCase(), name);
  const choices = [];
  for (const scope of scopes) {
    const left = beforeScopes[scope] || beforeScopes[''] || {};
    const right = afterScopes[scope] || afterScopes[''] || {};
    for (const folder of names.values()) {
      const local = before.addonSelection ? effective(left, folder) : null;
      const remote =
        after.addonSelection || before.addonSelection ? effective(right, folder) : null;
      const label = (value) => (value === null ? 'Not recorded' : value ? 'On' : 'Off');
      if (local !== remote)
        choices.push({ folder, scope, local: label(local), remote: label(remote) });
    }
  }
  const changedChoices = choices.length;
  if (before.addonSelection || after.addonSelection)
    rows.push({
      id: 'choices',
      label: 'Addon choices',
      local: before.addonSelection ? 'Saved' : 'Not recorded',
      remote: after.addonSelection || before.addonSelection ? 'Saved' : 'Not recorded',
      changed: !!changedChoices,
      details:
        count(changedChoices, 'choice') + (changedChoices === 1 ? ' differs' : ' differ'),
      choices: choices.sort(
        (a, b) => a.scope.localeCompare(b.scope) || a.folder.localeCompare(b.folder),
      ),
    });
  return rows;
}
module.exports = { compare };
