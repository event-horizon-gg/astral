function newer(addon, release) {
  if (addon.versionScheme === 'questie-bundle') {
    const parse = (value) => String(value || '').match(/^(\d+(?:\.\d+)*)\+v(\d+(?:\.\d+)*)$/);
    const a = parse(addon.version),
      b = parse(release.version);
    if (!a || !b) return null;
    const main = newer({ version: a[1] }, { version: b[1] });
    if (main) return true;
    if (newer({ version: b[1] }, { version: a[1] })) return false;
    return newer({ version: a[2] }, { version: b[2] });
  }
  if (addon.versionScheme === 'revision') {
    const a = String(addon.version || '').match(/^r?(\d+)$/)?.[1];
    const b = String(release.version || '').match(/^r?(\d+)$/)?.[1];
    return a && b ? Number(b) > Number(a) : null;
  }
  if (addon.versionScheme === 'letter-patch') {
    // These authors use letter patches, e.g. TOC "1.8.7 b" and tag "v1.8.7-b".
    // Do not interpret arbitrary prerelease labels as stable letter patches.
    const parse = (value) => {
      const match = String(value || '')
        .trim()
        .match(/^v?(\d+(?:\.\d+)*)(?:[ -]?([a-z]))?$/i);
      return (
        match && { numbers: match[1].split('.').map(Number), patch: match[2]?.toLowerCase() || '' }
      );
    };
    const a = parse(addon.version),
      b = parse(release.version);
    if (!a || !b) return null;
    for (let i = 0; i < Math.max(a.numbers.length, b.numbers.length); i++) {
      if ((b.numbers[i] || 0) !== (a.numbers[i] || 0))
        return (b.numbers[i] || 0) > (a.numbers[i] || 0);
    }
    return b.patch > a.patch;
  }
  const clean = (value) =>
    String(value || '')
      .replace(/^v\.?\s*/, '')
      .match(/^(\d+(?:\.\d+)*)(?:[-+].*)?$/)?.[1]
      ?.split('.')
      .map(Number);
  if (addon.id === 'rio') {
    const a = String(addon.version).match(/v(\d{12})/)?.[1],
      b = String(release.tag).match(/v(\d{12})/)?.[1];
    return a && b ? b > a : null;
  }
  const a = clean(addon.version),
    b = clean(release.version);
  if (!a || !b) return null;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((b[i] || 0) !== (a[i] || 0)) return (b[i] || 0) > (a[i] || 0);
  }
  return false;
}
function needsUpdate(addon, release) {
  return newer(addon, release) === true;
}
module.exports = { newer, needsUpdate };
