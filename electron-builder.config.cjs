const { INSTALLER_PATTERN, installerName } = require('./shared/release-contract.cjs');
module.exports = {
  appId: 'com.joaoseixas.astral',
  productName: 'Astral',
  directories: {
    output: 'release',
  },
  files: ['dist/**/*', 'desktop/**/*', 'shared/**/*', 'package.json', 'assets/**/*', 'addons/**/*'],
  mac: {
    category: 'public.app-category.utilities',
    hardenedRuntime: false,
    identity: '-',
    icon: 'assets/icon.png',
  },
  win: {
    target: 'nsis',
    icon: 'assets/icon.png',
  },
  artifactName: INSTALLER_PATTERN,
  nsis: {
    oneClick: false,
    perMachine: false,
    allowToChangeInstallationDirectory: true,
    artifactName: installerName('${version}', 'win32', '${arch}'),
  },
};
