const profiles = require('../shared/profiles.cjs');
const { flavor } = require('./flavors.cjs');
const fields = [
  'root',
  'account',
  'lastHash',
  'lastRemoteHash',
  'lastSync',
  'syncRevision',
  'syncProfile',
  'syncProfilePending',
];
const savedContext = (config) => Object.fromEntries(fields.map((key) => [key, config[key]]));
const profileId = (config) => profiles.selectedId(config.syncProfile);
const selected = (config) => ({
  root: config.root,
  account: config.account,
  profile: profileId(config),
});
function syncScope(config) {
  const client = flavor(config.root),
    profile = profileId(config);
  return profile === profiles.defaults[0].id ? client : client + '-' + profile;
}
function localChanges(config, profile) {
  return {
    ...config,
    ...(profile === undefined ? {} : { syncProfile: profile }),
    syncProfilePending: true,
    lastHash: null,
    lastRemoteHash: null,
    lastSync: null,
    syncRevision: 0,
  };
}
function accountChanged(config, account) {
  return { ...config, account, syncRevision: 0, lastHash: null, lastRemoteHash: null };
}
module.exports = {
  ...profiles,
  profileId,
  selected,
  savedContext,
  syncScope,
  localChanges,
  accountChanged,
};
