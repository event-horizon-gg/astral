const REPOSITORY = 'event-horizon-gg/astral';
const INSTALLER_PATTERN = 'Astral-${version}-${os}-${arch}.${ext}';
const DOWNLOAD_BASE = `https://github.com/${REPOSITORY}/releases/download/`;
const MAX_INSTALLER_BYTES = 600 * 1024 * 1024;
const RELEASE_TARGETS = Object.freeze([
  Object.freeze({ platform: 'darwin', arch: 'arm64' }),
  Object.freeze({ platform: 'win32', arch: 'x64' }),
]);
function installerName(version, platform, arch) {
  const suffix = { darwin: ['mac', 'zip'], win32: ['win', 'exe'] }[platform];
  if (!suffix) throw Error('There is no update for this computer yet.');
  return INSTALLER_PATTERN.replace('${version}', version)
    .replace('${os}', suffix[0])
    .replace('${arch}', arch)
    .replace('${ext}', suffix[1]);
}
module.exports = {
  INSTALLER_PATTERN,
  REPOSITORY,
  DOWNLOAD_BASE,
  MAX_INSTALLER_BYTES,
  RELEASE_TARGETS,
  installerName,
};
