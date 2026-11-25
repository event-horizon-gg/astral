# Astral Info

A small, movable FPS/latency bar and mount information in player tooltips. No dependency on EllesmereUI or another addon.

The bar starts below the usual top-right minimap position. It shows FPS and world latency, refreshes once a second, and shows both home/world latency when hovered. Low FPS and high latency change color. Position, size and options are saved with the UI profile.

## Commands

| Command | Action |
| --- | --- |
| `/ainfo unlock` | Drag the performance bar. |
| `/ainfo lock` | Lock its position. |
| `/ainfo size 16` | Set text size from 10 to 30. |
| `/ainfo latency world` | Show world latency, the default. |
| `/ainfo latency home` | Show home latency. |
| `/ainfo latency both` | Show separately labelled home/world latency. |
| `/ainfo hide` / `/ainfo show` | Hide/show the performance bar. Mount tooltips remain independent. |
| `/ainfo mounts off` / `/ainfo mounts on` | Disable/enable mount information. |
| `/ainfo source off` / `/ainfo source on` | Hide/show mount acquisition text. |
| `/ainfo reset` | Restore the default position and options. |

`/astralinfo` is an alias.

## Mount tooltips

Hover a player whose mount is represented by a readable mount aura. The standard unit tooltip gains its mount name, your collected/not-collected status, and the client's Mount Journal source text. Source information can include a vendor, boss, achievement or quest; it is the game's description, not an external walkthrough or a guarantee that an old mount is still obtainable.

No player inspection requests, mount-journal filter changes, external websites or embedded mount database are used. Repeated tooltip processing does not duplicate the lines. The addon leaves unrelated tooltip content and positioning alone.

Mount lookup is skipped in combat and when the required data is secret or unavailable. Shape-shift forms, disguises and mounts not exposed by the journal may not resolve. No source is invented when the journal has none.

The performance display supports the core APIs used by Retail and Forever. Mount tooltips detect available APIs: modern tooltip post-processing or a supported legacy unit-tooltip hook, modern aura data or legacy UnitBuff, and a mount journal. Without journal support, the performance display still works but mount details are omitted. Forever's actual mount-journal behavior still needs an in-game check.

## Installation and updates

Copy the `AstralInfo` directory into the chosen client's `Interface/AddOns` while WoW is closed, then enable it on the next login. A ZIP must contain `AstralInfo/AstralInfo.toc` at its top level.

Astral's source catalogue includes it as a bundled addon. It will be available to install/update through Astral after an approved app release containing this branch. It is not automatically installed or enabled by this change.

Runtime tests execute the real Lua modules with mocked modern/legacy APIs, including secret data and absent journal support. No live game files were changed during development.

## API sources

- [Blizzard performance bar source](https://github.com/Gethe/wow-ui-source/blob/live/Interface/AddOns/Blizzard_PerformanceBar/PerformanceBar.lua): GetFramerate and GetNetStats usage.
- [Blizzard mount-journal API declarations](https://github.com/Gethe/wow-ui-source/blob/live/Interface/AddOns/Blizzard_APIDocumentationGenerated/MountJournalDocumentation.lua): mount spell mapping, collection state and source text.
- [Blizzard aura API declarations](https://github.com/Gethe/wow-ui-source/blob/live/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitAuraDocumentation.lua): guarded unit-aura access.
- [Blizzard tooltip processor source](https://github.com/Gethe/wow-ui-source/blob/live/Interface/AddOns/Blizzard_SharedXMLGame/Tooltip/TooltipDataHandler.lua): post-call integration.

MIT License; part of Astral.
