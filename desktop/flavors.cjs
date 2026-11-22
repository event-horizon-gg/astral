const path = require('node:path');
function flavor(root) {
  const n = path.basename(root);
  return ['_forever_', '_classic_beta_'].includes(n) ? 'forever' : 'retail';
}
function allowed(addon, type) {
  return (addon.flavors || ['retail', 'forever']).includes(type);
}
function interfaceFor(type) {
  return type === 'forever' ? '16001' : '120105';
}
module.exports = { flavor, allowed, interfaceFor };
