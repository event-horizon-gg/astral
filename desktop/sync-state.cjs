function syncAction(local, remote, lastHash, choice, lastRemoteHash = lastHash) {
  if (!remote.revision) return 'upload';
  if (remote.hash === local.hash || (local.hash === lastHash && remote.hash === lastRemoteHash))
    return 'unchanged';
  if (choice === 'cloud' || local.files === 0 || lastHash === local.hash) return 'download';
  if (choice === 'local' || remote.hash === lastRemoteHash) return 'upload';
  return 'conflict';
}
module.exports = { syncAction };
