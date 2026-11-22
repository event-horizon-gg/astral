# Product interface

Keep Astral's approved Editorial style: light/dark neutrals, cobalt accents, bold sans typography and the collection layout.

Cut decorative taglines, helper paragraphs and labels that repeat nearby controls. Use direct page names. If normal operation needs a paragraph to explain it, first improve grouping, labels and actions. Put optional detail at the point where it helps a decision.

Preserve accessible names, actual status information, compatibility/dependency results, instructions for disabled actions, and confirmations explaining destructive or conflicting changes. Removing visible labels must not remove accessible labels.

Keep developer and hosting details out of the normal app. Do not replace deleted filler with differently worded filler. Review empty, loading, error and signed-out states as well as populated screens.

The spending limit is $0. Release signing stays local; never commit or upload the private release key.

## Visual checks on this Mac

Use the installed, already-approved Astral and Astral Native Preview apps for visual checks. Do not create or launch disposable comparison app bundles or new app identities: they repeatedly stalled unattended work on macOS permission prompts. Update the existing approved app in place when needed. Installed Mac updates must reuse a persistent signing certificate. Save its public fingerprint once with scripts/save-mac-certificate.mjs and build with --persistent; ASTRAL_MAC_SIGN_IDENTITY is an explicit override. Do not keep ad-hoc re-signing the approved apps: that invalidates Keychain Always Allow approvals. Until the certificate is configured, continue code and isolated tests without replacing installed apps. Keep disposable files and fake service accounts for automated tests.

Before starting unattended work, establish whether the user will be away. While they are away, avoid checks that require them to clear a prompt; continue independent code and automated checks instead. Never operate the screen while they are playing WoW.
