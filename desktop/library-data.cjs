function validate(list) {
  if (!Array.isArray(list) || list.length > 250) throw Error('Invalid addon collection.');
  const seen = new Set();
  for (const item of list) {
    if (
      !item ||
      typeof item.folder !== 'string' ||
      !/^[^/\\:\x00-\x1f]{1,100}$/.test(item.folder) ||
      item.folder.trim() !== item.folder ||
      ['.', '..'].includes(item.folder) ||
      /^Blizzard_/i.test(item.folder) ||
      typeof item.name !== 'string' ||
      item.name.length > 120 ||
      /[\x00-\x1f]/.test(item.name) ||
      seen.has(item.folder.toLowerCase())
    )
      throw Error('Invalid addon collection.');
    seen.add(item.folder.toLowerCase());
  }
  return list.map(({ folder, name }) => ({ folder, name }));
}
function merge(...lists) {
  const all = new Map();
  for (const list of lists)
    for (const item of validate(list)) all.set(item.folder.toLowerCase(), item);
  return validate(
    [...all.values()].sort((a, b) => (a.folder < b.folder ? -1 : a.folder > b.folder ? 1 : 0)),
  );
}
function required(list, selection) {
  if (!selection) return list;
  return list.filter((item) =>
    Object.values(selection.scopes).some((values) =>
      Object.entries(values).some(
        ([folder, on]) => on && folder.toLowerCase() === item.folder.toLowerCase(),
      ),
    ),
  );
}
module.exports = { validate, merge, required };
