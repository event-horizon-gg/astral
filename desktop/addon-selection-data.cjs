const safeName = (name) =>
  typeof name === 'string' &&
  name.length > 0 &&
  name.length <= 128 &&
  name.trim() === name &&
  !['.', '..'].includes(name) &&
  !/[/\\:\x00-\x1f]/.test(name);
const validFolder = (name) => safeName(name) && !/^Blizzard_/i.test(name);
const validScope = (scope) =>
  scope === '' ||
  (scope.split('/').length === 2 &&
    scope
      .split('/')
      .every(
        (part) =>
          part && !['.', '..', 'SavedVariables'].includes(part) && !/[\\:\x00-\x1f]/.test(part),
      ));

function parse(text) {
  const values = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const match = line.match(/^([^:]+):\s*(enabled|disabled)\s*$/i);
    if (!match || !safeName(match[1]))
      throw Error('An addon selection file has an unsupported format.');
    values[match[1].toLowerCase()] = match[2].toLowerCase() === 'enabled';
  }
  return values;
}
function entryNames(text) {
  return text.split(/\r?\n/).flatMap((line) => {
    const match = line.trim().match(/^([^:]+):\s*(enabled|disabled)\s*$/i);
    return match && validFolder(match[1]) ? [match[1]] : [];
  });
}
function validate(selection) {
  if (
    !selection ||
    selection.schema !== 1 ||
    !selection.scopes ||
    Array.isArray(selection.scopes) ||
    typeof selection.scopes !== 'object' ||
    !Object.hasOwn(selection.scopes, '')
  )
    throw Error('Invalid addon selection.');
  const entries = Object.entries(selection.scopes);
  if (entries.length > 500) throw Error('Too many character addon selections.');
  for (const [scope, values] of entries) {
    if (
      !validScope(scope) ||
      !values ||
      typeof values !== 'object' ||
      Array.isArray(values) ||
      Object.keys(values).length > 2000 ||
      Object.entries(values).some(
        ([name, value]) => !validFolder(name) || typeof value !== 'boolean',
      )
    )
      throw Error('Invalid addon selection.');
  }
  return selection;
}
function effective(values, folder) {
  const match = Object.keys(values).find((key) => key.toLowerCase() === folder.toLowerCase());
  return match === undefined ? false : values[match];
}
function edit(selection, groups, edits) {
  const result = structuredClone(validate(selection));
  if (
    !edits ||
    typeof edits !== 'object' ||
    Array.isArray(edits) ||
    Object.keys(edits).length > 250
  )
    throw Error('Invalid addon choices.');
  for (const [id, enabled] of Object.entries(edits)) {
    const addon = groups.find((addon) => addon.id === id);
    if (!addon || !addon.modules.some(validFolder) || typeof enabled !== 'boolean')
      throw Error('Unknown addon choice.');
    for (const values of Object.values(result.scopes))
      for (const folder of addon.modules) if (validFolder(folder)) values[folder] = enabled;
  }
  return result;
}
function rows(selection, groups) {
  validate(selection);
  return groups
    .filter((addon) => addon.modules.some(validFolder))
    .map((addon) => {
      const states = Object.values(selection.scopes).flatMap((values) =>
        addon.modules.filter(validFolder).map((folder) => effective(values, folder)),
      );
      return {
        id: addon.id,
        name: addon.name,
        enabled: states.every(Boolean) ? true : states.every((value) => !value) ? false : null,
      };
    });
}
module.exports = {
  safeName,
  validFolder,
  validScope,
  parse,
  entryNames,
  validate,
  effective,
  edit,
  rows,
};
