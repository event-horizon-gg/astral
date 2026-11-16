# Releases

Public source and release assets live at https://github.com/theedward/astral.

1. Run `npm ci`, `npm test` and `npm run build`.
2. Review the change and update `package.json`, `package-lock.json` and `CHANGELOG.md`.
3. Commit, then create the matching `vX.Y.Z` tag and push it.
4. The build workflow packages macOS arm64 and Windows x64. Only after both succeed does it prepare a draft release containing the installers. Drafts are invisible to the app updater.
5. On this Mac, set `ASTRAL_MAC_SIGN_IDENTITY` to the 40-character SHA-1 fingerprint of the persistent code-signing certificate. Build the Mac package from the tagged commit with `npm run package:mac`, then replace the matching Mac ZIP in the draft release. Keep using that certificate for subsequent builds. A local certificate can provide a stable identity without an Apple membership; it does not provide Apple notarization. Certificate setup and its initial approval must be done while the user is present.
6. Run `node scripts/publish-release.mjs /path/to/local-private-key.pem` on macOS. This downloads both draft installers, checks the Mac ZIP paths, application identifier/version, valid code signature, stable designated requirement and exact certificate fingerprint, then signs the exact installer bytes and publishes the complete release. It rejects ad-hoc signatures and signatures tied to a particular build. The Ed25519 release key must be owner-readable only and match the pinned public key. Neither private key is uploaded or printed.
7. Check the release from an older installed Astral. Verify download, restart, retained preferences and Keychain access on macOS; verify the NSIS flow on Windows before claiming Windows runtime support.

Never publish partial assets as a stable release. Never reuse a version for different binaries. The release signing key is a local Ed25519 PKCS8 PEM file, supplied only to the local signer through `ASTRAL_RELEASE_PRIVATE_KEY`. It is not uploaded to GitHub. Its public half is pinned in `desktop/update-public-key.pem`. The signing script refuses a mismatched key. Restrict repository write access, protect the main branch and tags where the account supports it, and keep an offline protected backup of the key. Forks must use their own repository and verification key.

The initial installed 0.4 app cannot discover updates; install 0.5 once to bootstrap the updater. App checks do not upload inventories or credentials. The download cache and macOS install log live under the existing Astral user-data directory, in `app-updates/`. A failed macOS startup restores the former app and retains the failed bundle alongside it. Do not delete retained backups until recovery is verified.

For Native, use the corresponding `native-vX.Y.Z` tag, build the Mac ZIP with `sh native/package-mac.sh` and the same `ASTRAL_MAC_SIGN_IDENTITY`, replace that draft's Mac ZIP, then pass `--native` to the publisher. The identifier and certificate checks also apply to Native.

This pipeline uses public GitHub Actions runners and GitHub Releases. It does not enable paid GitHub features, Cloudflare plans, Apple membership or code-signing subscriptions. CI builds default to an ad-hoc integrity signature for isolated checks. Those Mac ZIPs must be replaced with persistently signed local builds before publication. The manifest signature does not replace platform signing. Private certificate keys stay in the local signing setup, outside source control and CI.

macOS records an app's designated requirement for Keychain access. An ad-hoc requirement identifies one exact build, so rebuilding breaks earlier Always Allow approvals. See [Apple's code-signing identity documentation](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements/). Use the persistent certificate for installed development updates too; do not repeatedly replace the approved app with ad-hoc builds.
