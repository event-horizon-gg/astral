import rules from '../../../shared/profiles.json' with { type: 'json' };
const customId = new RegExp(rules.customIdPattern);
const ids = new Set(rules.defaults.map((profile) => profile.id));
const cacheKey = (root, account) => 'astral-profile-names:' + JSON.stringify([root, account]);
export const defaultProfiles = rules.defaults;
export const profileNameLimit = rules.maxNameLength;
export const isCustomProfile = (id) => customId.test(id);
export function readProfileNames(storage, root, account) {
  try {
    const value = JSON.parse(storage.getItem(cacheKey(root, account)));
    return Array.isArray(value)
      ? value
          .filter(
            (profile) =>
              profile &&
              (ids.has(profile.id) || customId.test(profile.id)) &&
              typeof profile.name === 'string' &&
              profile.name.length <= rules.maxNameLength,
          )
          .slice(0, rules.maxProfiles)
      : null;
  } catch {
    return null;
  }
}
export function saveProfileNames(storage, root, account, profiles) {
  try {
    storage.setItem(
      cacheKey(root, account),
      JSON.stringify(profiles.map(({ id, name }) => ({ id, name }))),
    );
    return true;
  } catch {
    return false;
  }
}
