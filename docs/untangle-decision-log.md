# Electron Astral: untangle decision log

Scope: the Electron product (`desktop`, `src`, `cloud`, release scripts and first-party addons). The Rust port remains paused. Preserve the existing UI, storage formats, service API, signed-update trust and live WoW setup. Comments and tests cleanup were explicitly selected. Baseline: 106 tests and the production renderer build pass. A source snapshot and dependency graph are retained under workspace `work/untangle/`.

## Decisions and problems they solve

1. **Give persistence, addon metadata, version comparison and settings encoding stable owners.**
   - `desktop/engine.cjs:12–20` exports persistence to 17 dependants despite also owning installation and inventory.
   - Settings gzip/hash/schema logic is copied into `settings`, `local-profiles`, `profile-policy` and `setup-sharing`; `inspect` decompresses twice.
   - `updater` re-exports installer version comparison; `main` consequently depends on scheduling to compare versions.
   - Selection and library data validation are entangled with disk/package operations.
2. **Use one package compatibility policy for review and installation.**
   - Ellesmere exclusions, Raider.IO module selection, client TOC selection, interface/file validation and build-info parsing are independently implemented in `engine` and `projects`.
   - Inventory, addon defaults and sharing know their own TOC conventions.
3. **Make preserving addon choices part of installing setup packages.**
   - `sync`, local profiles, sharing and cloud restore each construct a selection-only snapshot and supply an `afterInstall` callback.
   - The required call order is implicit, so a new installation route can enable addons in the wrong profile.
4. **Declare commands, reviews and selected setup transitions once.**
   - IPC names occur in handlers, preload and a separate mutation set.
   - Project/profile/share review maps have inconsistent expiry/context/consume/cleanup conventions.
   - Profile ID validation, legacy scopes and reset fields repeat in main, clients and local profiles.
5. **Keep Electron main as composition rather than every feature's implementation.**
   - It combines lifecycle, account reconciliation, state assembly, OAuth transport/polling, cloud workflows and background jobs.
   - `state()` selects/persists an account during what appears to be a read.
6. **Unify addon operations and GitHub provider policy.**
   - Main, daily updates, suite installs and synced packages separately compose validation/install/activity/progress.
   - Deferral/rate-limit control flow depends on error wording.
   - Engine, project review and discovery duplicate repository validation and GitHub transport conventions.
   - `library-sync.managed` has no callers; cloud conflict decisions live in the filesystem/client module.
7. **Give renderer requests, scoped reads, navigation and dialogs explicit owners.**
   - `main.jsx` guesses action-specific effects and app-state results; feature components bypass it and strip IPC errors independently.
   - CloudPanel, SharedLibrary, SyncProfiles and DiscoveryPanel implement separate stale-result rules; refresh paths can update a different scope.
   - Main's raw dialogs duplicate Escape handling but lack the shared focus/busy lifecycle.
   - ConsolePort ownership and default profile identities are reimplemented in JSX instead of consuming backend capability metadata.
   - The unreferenced `forge.css` and `lunar.css` are runtime surplus. Retain historical prototypes and preserve the active CSS cascade.
8. **Separate service outcomes from wording and remove unused presentation resources.**
   - The login page recognizes success by comparing a worker message string.
   - The Worker serves an unused Cinzel font/license route solely checked by a legacy test.
   - Worker and controls duplicate DB/clock/JSON request boundaries and pass transport callbacks positionally.
9. **Define the release contract once.**
   - Publication, signed manifest generation and desktop verification separately pin repository and asset conventions.
10. **Express addon transitions and repeated configuration as domain operations/data.**
    - AstralProfiles repeats switch/detach/save/verify/hide choreography across manual and managed triggers.
    - Forever style repeats eight bar configurations. Keep its in-place merge and live-table identity behavior.
11. **Clean comments and tests after behavior is protected.**
    - A sharing test searches encoded file data for a plaintext token, so an encoded leak could pass.
    - A credentials heartbeat eventually firing does not prove non-blocking behavior.
    - A client-selection test asserts object identity instead of unchanged setup data.
    - Lua profile/bar behavior checks exist but are not invoked by the test gate.

## Requirements represented by remaining special cases

Platform process/credential/scheduler/update code; Forever's legacy folder and Regular cloud scopes; provider version formats; curated module/locale/TOC compatibility; Regular/Handheld policy; OAuth and background polling/expiry; optimistic revision retry; optional artwork fallback; activity logging that cannot invalidate an install; data-only sharing validation and rollback. These are external requirements, not code to delete merely because it branches.

## Verification and closure

Each owner records the resulting design and behavioral evidence below. Run the complete test suite and production build, regenerate the dependency graph, then perform independent structural reviews. Preserve security, filesystem, rollback, revision/quota/isolation and signed-update checks. Do not operate installed apps, live game files, credentials or deployed services for this refactor.


## Implementation record

- Persistence now belongs to `desktop/storage.cjs`; missing files have explicit defaults, corrupt/unreadable state propagates, and failed atomic writes clean up temporary files. Installer/inventory compatibility exports retain their existing interface without owning this implementation.
- `setup-format.cjs`, `addon-selection-data.cjs` and `library-data.cjs` own the existing wire/data contracts. Snapshot inspection decompresses once. Local profile planning decodes its current snapshot once.
- `addon-metadata.cjs` owns client TOC preference, build-info interpretation, curated module filters and package compatibility/file validation for both review and installation. `addon-inventory.cjs` owns read/registry metadata; `library-inventory.cjs` keeps snapshot capture independent of installer orchestration.
- `setup-application.cjs` owns package installation while preserving the selected setup. Sync, local profiles, sharing and cloud restore use it. Local profile commit now executes inside the existing settings transaction, including rollback of newly created files.
- `addon-operations.cjs`, `addon-versions.cjs` and `github.cjs` own update outcomes, version policy and provider transport. Scheduling uses stable error codes instead of parsing user-facing text; daily versus individual history destinations remain distinct.
- Shared command/profile/release declarations now serve their producers and consumers. Main is composition around application state, sessions, cloud/sharing handlers, background jobs and owned reviews. Sandbox preload receives the actual command list from the main window rather than requiring unsupported local files.
- The renderer has feature-owned screens/controllers and shared application requests, navigation and dialog mechanics. A separate browser bootstrap mounts the application. Stale reads/reviews are scoped to their actual inputs; cancelled chooser results do not become app state. Runtime-unused forge/lunar themes are removed; the active cascade stays intact to preserve appearance.
- Service page outcomes are explicit, request/storage boundaries are shared, and unused font serving is removed. Profile defaults/limits come from the same pure shared declaration. Database routes, ownership/revision/quota/OAuth and legacy scope semantics are retained.
- Packaging metadata moved intact to `electron-builder.config.cjs`, including shared runtime dependencies. Packaging is checked using the builder's real file matcher and actual reachable relative dependencies, without installing/signing an app.
- First-party addon transitions/bar configuration are consolidated while preserving combat guards and live table identities. The Lua behavior harness is now part of the default Node test gate through a dev-only runtime.
- Optional cleanup removed implementation narration, the ineffective credential heartbeat claim and a client object-identity assertion. Credential exclusion checks inspect decoded share contents. Meaningful package, rollback, cloud, signature, IPC, DOM, stale-request and Lua checks are retained or strengthened.

## Independent review follow-ups

The second pass checked the revised owners and found bounded integration issues: an unreadable saved profile was still treated as missing; sandbox preload could not read local contract files; shared runtime files needed packaging inclusion; a cancelled ZIP chooser could clobber renderer state; Discovery and profile cache/review responses needed complete input identity; and closed root dialogs needed to avoid evaluating null content. These were closed under the grouped boundaries above with behavioral checks, rather than separate feature work.

The unused Electron signing-shell helper was removed only after confirming zero repository callers; the paused native repo's separate helper is untouched. Historical prototypes/art assets and intentional active CSS layers remain; changing their presentation is outside this behavior-preserving refactor.


## Final verification — 22 November 2026

- All **140 tests pass**, with zero failures or skipped tests, including the installed-app composition under a fake native shell, sandbox preload/IPC handoff, real App DOM journey, cloud constraints, package/filesystem rollback, scoped confirmations and Lua runtime behavior.
- Focused undefined-symbol/JSX lint and production Vite build pass.
- The three active CSS files are byte-identical to the pre-refactor snapshot; the built CSS remains `index-DfEAVgsh.css`.
- The regenerated static dependency graph has no cycles or unresolved relative imports. The stable persistence/data/codec modules no longer import installer orchestration.
- Independent desktop, renderer and service/tooling review follow-ups are closed. Cloud restore pins the revision at confirmation-open and clears context-bound confirmations; ZIP cancellation does not replace app state; stale query/profile/cache responses are discarded.
- Evidence: workspace `work/untangle/final-tests.log`, `final-build.log`, `dependencies-after.json`, `verification.json`; baseline archive and hashes remain available there.

The logged structural refactor is complete. This does not claim an installed-app visual comparison or a release deployment: installed apps, live WoW data, deployed services and signing identities were not changed. Required protocol/platform/provider/controller exceptions listed above remain intentional.
