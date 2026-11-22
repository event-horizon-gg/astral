# Collection workflow review

Branch: `feat/collection-workflows`. Keep unmerged and unpublished until the owner approves the UI at home. Do not replace the installed app or deploy the cloud change as part of review.

The branch contains a separate baseline commit for the previously queued Electron refactor and setup sharing (`e44248a`). Subsequent changes implement the approved workflow prototype.

## Changes

- Library and Discover remain the primary destinations. The active profile is directly accessible beside the game selector; management and the existing signed-out flow remain available.
- Profile review shows actual changed settings, addon choices, required packages and the applicable bar policy. Failed application retains its review and requires a fresh preview token before another apply attempt.
- Conflicts compare validated local settings with the effective cloud apply result, including profile rules and files that stay local. Technical file/character details expand only when needed. No Lua is evaluated for the comparison.
- Conflict decisions retain the reviewed cloud revision, local hash, root, account and profile. Changing the selected context dismisses stale dialogs; changed copies must be reviewed again.
- Manual addons always show their update status. Addon details uses shared dialog focus handling, layering, Escape and focus restoration.
- Errors persist until dismissed or resolved. Matching failures appear inside protected dialogs, while unrelated notifications remain below them. Normal automatic sync reviews do not also display a redundant error notification.
- Motion is limited to existing operations and short overlay transitions; reduced-motion behavior is preserved.

The cloud current-snapshot response adds an optional `deviceLabel` from the owned snapshot. Older deployed responses remain usable: the UI says the computer was not recorded instead of inventing a label. This server change is not deployed yet.

## Verification

- Full default test suite: 149 passed, none skipped.
- Production build and lint passed.
- Headless Chrome exercised the actual App with a fake desktop bridge: Library/Discover navigation, profile preview/cancel/apply, apply failure and fresh-review retry, menu and drawer focus handling, manual status, persistent download error, unrelated-success handling, retry, cloud choices, inline sync failure, automatic conflict resolution, context changes, profile management, narrow-screen navigation and running-game restrictions.
- Captures cover dark/light desktop, 1024/820/390px layouts and mobile conflict recovery. Reduced motion was checked.
- Independent code and design re-reviews passed after their material findings were fixed.
- The source detector still flags old declarations in style.css/editorial.css. Their overlay animations and one-sided toast border are overridden by workflows.css; actual browser captures and computed layering were checked.

Review evidence is in the task workspace's `work/design-skills/implementation/` directory, with the final test/build logs in `work/design-skills/`. `prototypes/implementation/` is an ignored, sample-data harness for browser checks, not an installed application or packaged feature.

## Owner review

1. Check that Library and Discover remain easy to reach.
2. Open the profile picker; cancel a switch, then inspect the change review.
3. Compare local/cloud copies and expand a changed-file or character detail.
4. Inspect a manual addon and use the drawer with the keyboard.
5. Review the failed-download and failed-profile screenshots, including the mobile action controls.

Installed-app and live-account checks remain pending owner approval. These automated checks never opened or modified live WoW, the installed Astral app, or deployed sync data.
