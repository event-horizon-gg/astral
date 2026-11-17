// Store only the public certificate fingerprint. This never creates an identity,
// reads a private key, changes Keychain trust, or grants application access.
import { saveMacSigningIdentity } from './mac-signing.mjs';

if (process.argv.length !== 3)
  throw Error('Usage: node scripts/save-mac-certificate.mjs <certificate SHA-1 fingerprint>');
await saveMacSigningIdentity(process.argv[2]);
console.log('Saved the local macOS signing certificate. Future builds will reuse it.');
