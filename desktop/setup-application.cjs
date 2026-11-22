const engine = require('./engine.cjs');
const settings = require('./settings.cjs');
const packages = require('./library-sync.cjs');
const format = require('./setup-format.cjs');

// Installing an addon changes WoW's default enablement. Preserve the selected
// setup after each package, including when a later package fails or is deferred.
async function installPackages({
  root,
  account,
  data,
  plan,
  current,
  api = engine,
  store = settings,
  library = packages,
  running = api.gameRunning,
  onProgress = () => {},
}) {
  const selection = store.decode(current.payload).addonSelection;
  const preserve = format.encode({ schema: 2, files: [], addonSelection: selection });
  return library.install({
    root,
    data,
    plan,
    api,
    onProgress,
    afterInstall: () => store.apply(root, account, preserve.payload, data, running),
  });
}
module.exports = { installPackages };
