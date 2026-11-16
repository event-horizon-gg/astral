// Prepare disposable macOS apps for CUA comparisons. Does not launch either app.
// node native/prepare-comparison.cjs --fixture /path/to/sample --electron-app /path/to/Electron.app [UI flags]
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const repository = path.resolve(__dirname, '..');
const args = process.argv.slice(2);
const take = (flag) => {
  const index = args.indexOf(flag);
  if (index < 0 || !args[index + 1]) throw Error(`Supply ${flag}.`);
  return args.splice(index, 2)[1];
};
const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
const run = (command, flags) => execFileSync(command, flags, { stdio: 'inherit' });

if (process.platform !== 'darwin') throw Error('This comparison bundle helper requires macOS.');
const source = fs.realpathSync(take('--fixture'));
const runtime = fs.realpathSync(take('--electron-app'));
const native = args.includes('--native-binary')
  ? fs.realpathSync(take('--native-binary'))
  : path.join(__dirname, 'target/release/astral-native');
const groups = [
  ['--retail', '--forever'],
  ['--light', '--dark'],
  ['--compact', '--wide'],
  ['--library', '--discover', '--account', '--activity', '--sources', '--settings'],
  ['--sync-conflict', '--sync-missing', '--profile-review'],
  ['--signed-in'],
  ['--notice', '--notice-error'],
];
if (
  args.some((flag) => !groups.flat().includes(flag)) ||
  groups.some((group) => args.filter((flag) => group.includes(flag)).length > 1)
)
  throw Error('Use one client, theme, page, dialog and viewport flag.');
if (
  !fs.existsSync(path.join(runtime, 'Contents/MacOS/Electron')) ||
  !fs.existsSync(path.join(runtime, 'Contents/Resources/default_app.asar')) ||
  fs.existsSync(path.join(runtime, 'Contents/Resources/app.asar'))
)
  throw Error('Supply the unmodified Electron development runtime, not an installed product.');
if (!fs.existsSync(native) || !fs.existsSync(path.join(repository, 'dist/index.html')))
  throw Error('Build the native release binary and Electron frontend first.');

const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'astral-comparison-'));
// Keep the reference's runtime reads out of Documents/FileProvider. Ad-hoc
// comparison identities can otherwise stall on an OS folder-access request
// before Electron has created a window to display it.
const reference = path.join(directory, 'reference');
for (const folder of ['desktop', 'dist', 'assets', 'addons']) {
  fs.cpSync(path.join(repository, folder), path.join(reference, folder), {
    recursive: true,
    dereference: true,
  });
}
const copiedDependencies = new Set();
function copyDependency(name) {
  if (copiedDependencies.has(name)) return;
  copiedDependencies.add(name);
  const manifest = require.resolve(name + '/package.json', { paths: [repository] });
  const info = JSON.parse(fs.readFileSync(manifest, 'utf8'));
  fs.cpSync(path.dirname(manifest), path.join(reference, 'node_modules', name), {
    recursive: true,
    dereference: true,
  });
  for (const dependency of Object.keys(info.dependencies || {})) copyDependency(dependency);
}
copyDependency('yauzl');
fs.cpSync(path.join(__dirname, 'fixtures'), path.join(reference, 'native/fixtures'), { recursive: true });
const fixture = path.join(directory, 'fixture');
let clients = 0;
for (const client of ['_retail_', '_forever_']) {
  const base = path.join(source, client);
  if (!fs.existsSync(path.join(base, 'Interface/AddOns'))) continue;
  fs.cpSync(base, path.join(fixture, client), { recursive: true, verbatimSymlinks: true });
  clients++;
}
if (!clients) throw Error('The fixture must contain a client with Interface/AddOns.');
const launchArgs = ['--ui-fixture', fixture, ...args];

const nativeApp = path.join(directory, 'Astral Native Comparison.app');
const nativeMac = path.join(nativeApp, 'Contents/MacOS');
fs.mkdirSync(nativeMac, { recursive: true });
fs.copyFileSync(native, path.join(nativeMac, 'astral-native'));
fs.writeFileSync(
  path.join(nativeMac, 'launch'),
  '#!/bin/sh\nexec "$(dirname "$0")/astral-native" ' + launchArgs.map(quote).join(' ') + '\n',
  { mode: 0o755 },
);
fs.writeFileSync(
  path.join(nativeApp, 'Contents/Info.plist'),
  `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>launch</string>
<key>CFBundleIdentifier</key><string>com.joaoseixas.astral.comparison-native</string>
<key>CFBundleName</key><string>Astral Native Comparison</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>\n`,
);
run('/usr/bin/codesign', ['--force', '--sign', '-', path.join(nativeMac, 'astral-native')]);
run('/usr/bin/codesign', ['--force', '--sign', '-', nativeApp]);
run('/usr/bin/codesign', ['--verify', '--deep', '--strict', nativeApp]);

// Keep Electron as the actual bundle executable. A shell wrapper around it can
// change app.isPackaged or leave Launch Services unable to reconnect on relaunch.
const electronApp = path.join(directory, 'Astral Electron Comparison.app');
run('/usr/bin/ditto', ['--norsrc', '--noextattr', runtime, electronApp]);
const plist = path.join(electronApp, 'Contents/Info.plist');
run('/usr/bin/plutil', [
  '-replace',
  'CFBundleIdentifier',
  '-string',
  'com.joaoseixas.astral.comparison-electron',
  plist,
]);
run('/usr/bin/plutil', [
  '-replace',
  'CFBundleName',
  '-string',
  'Astral Electron Comparison',
  plist,
]);
run('/usr/bin/plutil', [
  '-replace',
  'CFBundleDisplayName',
  '-string',
  'Astral Electron Comparison',
  plist,
]);
const application = path.join(electronApp, 'Contents/Resources/app');
fs.mkdirSync(application);
const version = JSON.parse(fs.readFileSync(path.join(repository, 'package.json'), 'utf8')).version;
fs.writeFileSync(
  path.join(application, 'package.json'),
  JSON.stringify({
    name: 'astral-ui-reference',
    version,
    main: 'bootstrap.cjs',
  }),
);
const entry = path.join(reference, 'desktop/main.cjs');
fs.writeFileSync(
  path.join(application, 'bootstrap.cjs'),
  `
process.argv = [process.execPath, ${JSON.stringify(entry)}, ...${JSON.stringify(launchArgs)}];
try { require(${JSON.stringify(entry)}); }
catch (error) {
  require('node:fs').writeFileSync(${JSON.stringify(path.join(directory, 'reference-error.txt'))}, String(error.stack || error));
  require('electron').app.exit(1);
}
`,
);
run('/usr/bin/codesign', ['--force', '--sign', '-', electronApp]);
run('/usr/bin/codesign', ['--verify', '--deep', '--strict', electronApp]);
console.log(JSON.stringify({ directory, fixture, nativeApp, electronApp }, null, 2));
