const definition = require('./profiles.json');
const custom = new RegExp(definition.customIdPattern);
const builtins = new Set(definition.defaults.map(({ id }) => id));
const validId = (id) => typeof id === 'string' && (builtins.has(id) || custom.test(id));
const isCustom = (id) => typeof id === 'string' && custom.test(id);
const selectedId = (value) => (validId(value) ? value : definition.defaults[0].id);
module.exports = { ...definition, validId, isCustom, selectedId };
