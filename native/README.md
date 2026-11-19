# Astral Native

Control icons use Lucide (ISC), matching Electron. Their license is included in
the executable and can be read with `astral-native --licenses`. Regenerate the
SVGs from the installed Electron dependency with `node scripts/sync-native-icons.cjs`.

Native ZIP selection uses RFD (MIT), with its notice also available through `--licenses`.

Experimental GPUI desktop client on the `rust-gpui` branch. The Electron app remains the reference for behavior. The native client uses the existing WoW installation and public GitHub releases. Sign in separately in each app.

On macOS and Windows, the tray menu provides **Open Astral** and **Quit Astral**. Closing the window hides it and leaves the existing timers running. Reopen it from the tray or the installed app. Windows also opens it on a left click of the tray icon. A login/background launch keeps the window hidden; on macOS it also hides the Dock icon. Read-only comparison fixtures do not create a tray icon.

Run with Rust and Xcode on macOS:

```sh
cargo run --locked --manifest-path native/Cargo.toml
```

The Library detects the standard Retail and Forever folders. Settings remembers custom folders and appearance. Check for updates compares installed addons with the latest release from each connected GitHub repository.

Open **Review release** from Discover or an installed addon's details to check a catalog release against the selected WoW build. **Update from ZIP** checks a local package for a supported third-party addon. Close WoW before installing or restoring addon files. Astral stores copies of replaced addon folders under `Interface/.astral-native-backups`. Use **Tools → Activity & backups** to restore earlier versions or recover an interrupted operation. SavedVariables and other WoW settings live outside these operations.

Choose **Uninstall…** in a Library addon's details to move its folders into a restorable backup. Astral checks whether other installed addons require those folders before removing them.

Open the profile button at the top right, then use the **Profile** picker on **Profiles & sync** to switch between Regular and Handheld while WoW is closed. Astral starts with the current setup as Regular. The first switch copies that setup to Handheld. Changes you make in WoW are saved to the active profile when you switch away. Each installation and account has its own profiles.

Open the profile button at the top right, choose **Profiles & sync**, then **Connect Battle.net** and finish sign-in in your browser. The Account page lists your connected devices and can remove their access. The Rust preview uses the existing Astral service and saves its session in macOS Keychain or Windows Credential Manager.

Background sign-in lookups on macOS do not open authentication dialogs. If Keychain access is unavailable, Astral reports it without waiting for a prompt. Explicit sign-in still uses the normal credential-storage flow.

Use **Profiles & sync → Manage profiles** to create, rename, delete, or select profiles shared across computers. Close WoW before switching. On first use, a profile starts from a copy of the current local setup. Choose **Sync now** to fetch its cloud settings.

With WoW closed, choose **Account → Sync now** to sync the active Regular or Handheld profile for the selected game client. If both copies differ, review the cloud backup date and choose which version to keep. Astral installs missing addons from connected sources before applying the cloud snapshot. If a required addon needs a manual download, Astral lists it and keeps the current settings in place. Cooldown Manager layouts are included in local backups. The cross-app sync format currently covers the same layout files as Electron.

Choose **Account → Shared addons → Sync addons** to sync the addon lists for both installed game clients without restoring a profile. Each client keeps its own shared list. Newly installed packages stay disabled in the current setup until a profile or the user enables them.

After Battle.net sign-in, Astral checks the selected client's settings when it starts, after WoW closes, and every five minutes while WoW is closed. A new computer can upload its first backup or receive settings for an empty account automatically. Conflicting changes still need your review. After switching profiles, choose **Sync now** once to review that profile's cloud settings before automatic sync resumes. The earlier native preview's separate automatic-sync switch is no longer used.

**Account → Cloud backups** lists the active profile’s saved revisions. Close WoW before restoring a revision. Astral backs up the local settings it replaces and makes the restored revision current in the cloud. **Account → Cloud data** can delete cloud backups and shared lists and sign out all devices. Local WoW files and backups stay on this computer.

Open **Manage profiles** to add up to six custom profiles or rename a profile. A new profile starts from the setup active on its first switch. Delete an inactive custom profile to move its saved settings to backups.

Choose **Apply Handheld preset** to enable gamepad input and use controller action bars. Install EllesmereUI and Astral Profiles 1.2.0 or newer first. Retail also needs ConsolePort and its Action Bar module. The preset enables ConsolePort in Retail Handheld and disables it in Retail Regular. For Forever, select **Gamepad** in WoW’s Gameplay options. Astral Profiles hides the four desktop Ellesmere bars once the controller bars are available.

Profiles and backups include keybindings, controller settings, Blizzard Edit Mode and Cooldown Manager layouts, enabled addons, and supported addon settings. Astral only changes gamepad and soft targeting options in `Config.wtf` so graphics and display preferences stay local. Older profiles and backups without controller settings leave the current controller options in place when loaded.

Backups stay on this computer and are listed separately for each WoW installation and account. Before a restore, Astral saves the current settings it will replace. If Astral reports an interrupted switch, close WoW and choose **Recover previous setup**.

After **Check for updates**, choose **Update all** to apply the available versions. Astral checks each package before installing, stops on the first failure, and shows completed and pending updates in the Library. **Retry remaining updates** starts with the failed addon. To leave an addon out of scheduled updates, turn off **Include in daily updates** in its Library details.

Choose **Install** in Discover for an Astral addon included with the app. On Retail, **Astral Talents** opens with `/astraltalents`. The native app checks bundled addon versions alongside GitHub releases. Turn on **Settings → Daily addon updates** to update installed addons and refresh Astral Talents when WoW is closed. On supported systems, turning on daily updates also schedules checks after quitting Astral Native on macOS or while signed in on Windows. Keep automatic addon updates enabled in only one Astral app.

Choose **Check now** under **Daily addon updates** to check and update both installed clients immediately with WoW closed. This works with scheduled updates off and respects each client’s excluded addons.

**Activity & backups** shows which addon versions changed and which need attention. Earlier Astral backups for the selected WoW installation appear alongside new backups. Restoring an earlier backup saves the current addon files so you can undo the restore. The earlier Raider.IO archive can be opened from the same page.

With Forever selected, **Discover → Forever essentials** offers to install the missing addons from the same suite as Electron. Manual sources such as TomTom link to their download pages from Discover and Library.

`cargo run --locked --manifest-path native/Cargo.toml -- --scan` emits the detected inventories as JSON lines. On macOS, `--check keyui` runs one release check against the detected installation.

For side-by-side UI testing, use a disposable directory containing `_retail_/Interface/AddOns` and/or `_forever_/Interface/AddOns` and optional test accounts under each client's `WTF/Account`. Pass the same absolute fixture path to both apps from the repository root.

```sh
cargo run --locked --manifest-path native/Cargo.toml -- --ui-fixture /path/to/fixture
npm run build
npm start -- --ui-fixture /path/to/fixture
```

The Electron reference starts in a signed-out state with fresh app data under the fixture directory. It reads the sample inventory and lets you switch clients. The Rust preview stores its preferences under the fixture and skips system startup changes, credential access, automatic sync, and background updates. Its manual package and profile actions can still change the sample files. Both windows open at 1250 × 840.

On this Mac, use the installed, already-approved **Astral** and **Astral Native Preview** apps for visual comparisons. Update the existing app in place; do not generate or launch temporary comparison app bundles or new identities. Those triggered repeated macOS permission prompts and stalled unattended checks. Disposable fixture files remain suitable for automated tests.

Append `--retail` or `--forever` and `--light` or `--dark` to both fixture commands to start in the same client and theme. The default is dark with Retail selected when present. Append `--sync-conflict` or `--sync-missing` to open the same sync dialog without a cloud session. The conflict buttons dismiss the sample dialog.

Append `--signed-in` to compare the account and profile manager with the same fake account and three sample profiles. Both fixture clients block cloud writes and credential access; Electron returns only canned cloud reads. This never signs in to the service.

Append `--profile-review` to compare the profile-switch confirmation using the shared Retail or Forever sample. Applying this sample review is blocked in both clients; it never switches a profile or installs an addon.

Append `--notice` or `--notice-error` to compare success and error notifications. These fixture notices remain visible until dismissed; normal notifications expire after 6.5 seconds. They use no cloud account or network action.

Append `--compact` to target a 1000 × 720 content area in both apps. The default fixture targets 1250 × 840. Fixture sizes target the content area; macOS may constrain a wide window to the available desktop height.

Add one of `--library`, `--discover`, `--account`, `--activity`, `--sources`, or `--settings` to choose the opening page.

**Settings → Astral updates** checks the latest stable Astral release for a signed native manifest and verifies the package before staging it. With WoW closed, **Update & restart** installs the package and waits for the new app window. If startup fails, it restores the previous version.

Build the Mac ZIP with `sh native/package-mac.sh` and place `Astral Native.app` in a writable folder. On Windows x64, build the ZIP with `powershell -File native/package-windows.ps1` and extract `astral-native.exe` into `%LOCALAPPDATA%\Programs\Astral Native` for the first install. Later updates replace the executable in that location.

For installed Mac updates and published releases, save the persistent certificate's public fingerprint with `node scripts/save-mac-certificate.mjs <fingerprint>`. Packaging reuses the ignored local setting automatically; `ASTRAL_MAC_SIGN_IDENTITY` can explicitly override it. Use `sh native/package-mac.sh --persistent` for installed builds: it refuses a missing certificate before compiling. Unconfigured CI builds still use an ad-hoc signature for isolated checks. Do not repeatedly install those builds: their per-build identity invalidates earlier Keychain approvals. The publisher requires the persistent certificate's fingerprint and rejects ad-hoc Mac packages. See [release signing](../docs/releases.md).

Native uses `native-vX.Y.Z` tags in the Astral repository. The public GitHub workflow builds and verifies both ZIPs, then creates a draft. Run `node scripts/publish-release.mjs --native /path/to/local-private-key.pem` locally to sign and publish that draft. The key stays local. Electron continues to use the repository’s latest Electron release, while native looks for its own signed release.

On Windows, run `cargo test --locked --manifest-path native/Cargo.toml signed_windows_update` with WoW closed. This checks a signed update against temporary executables, including ZIP tampering, successful acknowledgement, and rollback. The test uses Windows PowerShell and an ephemeral signing key.

The ignored Windows scheduler integration check is restricted to GitHub Actions. It registers, queries and deletes a uniquely named verification task using the production XML generator, checking the 15-minute interval, least-privilege interactive principal, battery behavior and missed-start setting. It never replaces the installed updater task or starts an updater. Its trigger is set a day ahead and cleanup also runs after an assertion failure. Ordinary local `cargo test` skips this check; the Windows verification workflow includes it.

To check the native HTTP client against the actual Worker handlers locally, use Node 24 or newer:

```sh
ASTRAL_TEST_ELECTRON_CODEC=node cargo test --locked --manifest-path native/Cargo.toml cloud::worker_tests
node --test tests/cloud.test.mjs tests/ui-fixture.test.mjs
```

The HTTP check binds an ephemeral port on `127.0.0.1`, uses fake sessions and an in-memory SQLite database, and kills the server on completion. It covers both game clients, independent profiles, shared libraries, revision conflicts, backup restore/retention and session revocation. It does not access WoW, Battle.net or Cloudflare. Without `ASTRAL_TEST_ELECTRON_CODEC`, the native HTTP check does not run. This validates HTTP/JSON compatibility with the service code in Node; it does not validate the deployed Cloudflare runtime or live cross-device sync.

For logical interface checks without opening an operating-system window:

```sh
cargo test --locked --manifest-path native/Cargo.toml --features headless-ui-tests ui_tests::
```

These checks instantiate the actual Astral view on GPUI's `TestPlatform`, using `TestWindow`, `NoopTextSystem` and a temporary fixture directory. They exercise click routing, focus traversal and complete key-down/key-up events. They do not launch installed apps, use live credentials or access a game installation. GPUI's test-support feature adds optional test dependencies; normal builds leave it disabled. These are not screenshot, font-rendering or real-platform dialog tests. Windows verification runs them after the ordinary package checks.

Folder-selection tests inject the pending result future and use temporary game folders. They verify operation ownership and completion, not the native chooser: `TestPlatform::prompt_for_paths` is unimplemented and must not be called by these tests.

The Cloudflare API and in-game Lua addons are shared compatibility contracts for this migration. See [remaining parity work](PARITY.md).
