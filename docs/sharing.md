# Share a setup

In the Electron app, choose **Tools → Export setup**. Name the setup, choose a character, and select UI/addon settings, keybindings and optional controller settings. Save the `.astral` file and send it to another person. Battle.net sign-in and hosting are not required.

The recipient selects the matching game client and local WoW account, then chooses **Tools → Import setup**. They choose their character and review changed files, addon choices and required installations. Import applies to the current Astral profile. WoW must be closed. Existing settings are backed up before being replaced.

Retail and Forever files are separate. Addon packages are resolved through the recipient's connected Astral sources rather than copied into the share file. Unknown/manual sources remain visible and must be connected or installed before applying. Graphics, display settings, Astral credentials and Blizzard chat/history SavedVariables are excluded. Addon SavedVariables can still contain character names and addon history; they are an explicit export option.

Character-specific file paths and common `Name - Realm`, `Name-Realm` and `Realm - Name` profile references map to the recipient's chosen character. Account-wide addon settings are replaced, so this is a setup import, not a merge of arbitrary addon databases. Addons with other identity formats may need their own profile selector.

## Format and validation

Schema 1 uses the `astral.setup` marker, client, name, source character, a gzip/base64 settings bundle and SHA-256 of the compressed payload. The checksum detects damage; it does not identify or authenticate an author. Import allows at most 36 MiB on disk, 24 MiB of expanded settings data and 8 MiB per settings file, with allowlisted paths and case-insensitive duplicate detection.

SavedVariables are parsed as data assignments, tables and scalar literals. Functions, calls and other executable Lua are rejected. Imported global variables must be declared by the installed addon's TOC before any settings are written. Desktop code never evaluates the Lua. Import reviews expire after five minutes and bind to the selected root, account, profile and current settings/package fingerprint. A failed settings write or preferences commit rolls back overwritten and newly created settings. Imported changes require explicit cloud review before automatic sync resumes.

`tests/setup-sharing.test.mjs` exercises two temporary accounts/characters, Retail/Forever isolation, category choices, installing a missing connected addon, missing sources, unsafe/undeclared data, stale previews, game-start rollback, preferences failure rollback and backup contents. It does not operate a live WoW installation.
