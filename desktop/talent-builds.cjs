const fs = require('node:fs/promises');
const path = require('node:path');
const { randomUUID } = require('node:crypto');

const specs = [
  [102, 'balance', 'dps', 'keeper-of-the-grove'],
  [103, 'feral', 'dps', 'druid-of-the-claw'],
  [104, 'guardian', 'tank', 'elunes-chosen'],
  [105, 'restoration', 'healer', 'wildstalker'],
];
const heroes = {
  'keeper-of-the-grove': 'Keeper of the Grove',
  'elunes-chosen': "Elune's Chosen",
  'druid-of-the-claw': 'Druid of the Claw',
  wildstalker: 'Wildstalker',
};
const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const modes = ['Mythic+', 'Raid', 'Delves'];

function parseGuide(html, spec, expectedInterface) {
  const [id, name, , fallbackHero] = spec;
  const version = html.match(/interior-sidebar-header-text-subtitle">\s*Patch (\d+)\.(\d+)\.(\d+)/);
  if (
    !version ||
    String(+version[1] * 10000 + +version[2] * 100 + +version[3]) !== expectedInterface
  )
    throw Error(`The ${name} talent guide does not match this WoW patch.`);

  // The guide's table contains the labels and Blizzard export strings as BBCode.
  const guide = html.replaceAll('\\"', '"').replaceAll('\\/', '/');
  const rows = Array.from(guide.matchAll(/\[tr\]([\s\S]*?)\[\/tr\]/g), ([, row]) => {
    const match = row.match(/\[copy="(Mythic\+|Raid|Delves)"\]([A-Za-z0-9+/]{60,512})\[\/copy\]/);
    const hero = row.match(/symbol=wow-hero-talent-([a-z-]+)\]/)?.[1];
    if (!match || !Object.hasOwn(heroes, hero)) return null;
    return { mode: match[1], code: match[2], hero, best: row.includes('(Best)') };
  }).filter(Boolean);

  return modes.map((mode) => {
    const candidates = rows.filter((row) => row.mode === mode);
    const build =
      candidates.find((row) => row.best) || candidates.find((row) => row.hero === fallbackHero);
    // The first 24 bits encode the serialization version and specialization ID.
    const header =
      build &&
      [...build.code.slice(0, 4)].reduce(
        (bits, char, i) => bits | (alphabet.indexOf(char) << (6 * i)),
        0,
      );
    if (header !== ((id << 8) | 2)) throw Error(`No valid ${mode} build in the ${name} guide.`);
    return { hero: heroes[build.hero], code: build.code };
  });
}

function bundle(guides, interfaceNumber, checkedAt) {
  if (!/^\d{6}$/.test(interfaceNumber)) throw Error('Unknown Retail patch.');
  const lines = [
    'local _, ns = ...',
    `ns.interface = ${interfaceNumber}`,
    `ns.checked = "${new Date(checkedAt).toISOString().slice(0, 10)}"`,
    'ns.builds = {',
  ];
  for (const spec of specs) {
    lines.push(`    [${spec[0]}] = {`);
    for (const { hero, code } of parseGuide(guides[spec[1]], spec, interfaceNumber))
      lines.push(`        { "${hero}", "${code}" },`);
    lines.push('    },');
  }
  lines.push('}', '');
  return lines.join('\n');
}

async function refresh(root, api, now, download = fetch) {
  const addons = await fs.realpath(path.join(root, 'Interface', 'AddOns'));
  const folder = path.join(addons, 'AstralTalents');
  const file = path.join(folder, 'Builds.lua');
  if (!(await fs.lstat(folder)).isDirectory() || !(await fs.lstat(file)).isFile())
    throw Error('Reinstall Astral Talents to restore its build file.');

  const interfaceNumber = await api.targetInterface(root);
  const guides = Object.fromEntries(
    await Promise.all(
      specs.map(async ([, name, role]) => {
        const response = await download(
          `https://www.wowhead.com/guide/classes/druid/${name}/talent-builds-pve-${role}`,
          { signal: AbortSignal.timeout(15000), redirect: 'error' },
        );
        if (!response.ok) throw Error(`Wowhead returned HTTP ${response.status} for ${name}.`);
        return [name, await response.text()];
      }),
    ),
  );
  const content = bundle(guides, interfaceNumber, now);
  const previous = await fs.readFile(file, 'utf8');
  if (content === previous) return false;

  const temp = file + '.' + randomUUID() + '.tmp';
  try {
    await fs.writeFile(temp, content, { flag: 'wx' });
    if (await api.gameRunning()) throw Error('WoW started. Talent refresh deferred.');
    await fs.rename(temp, file);
  } finally {
    await fs.rm(temp, { force: true });
  }
  return content.replace(/^ns.checked = .*$/m, '') !== previous.replace(/^ns.checked = .*$/m, '');
}

module.exports = { bundle, refresh };
