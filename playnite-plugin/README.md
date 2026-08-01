# AutoGSE Playnite Plugin (skeleton)

Phase 11 §11.4. A thin Playnite `GenericPlugin` that shells out to the real
`autogse.exe` CLI — it does not reimplement any injection logic. This is a
scaffold, not a polished/store-submitted extension: structurally complete and
compiles against the PlayniteSDK, but only genuinely proven correct once
built and loaded into a real running Playnite install against a real game
library (see `roadmap.md`'s Phase 11 notes for why that verification is
flagged, not assumed done).

## What it does

Per-game context menu items (only shown when the game has a resolvable,
existing `InstallDirectory`):

- **AutoGSE: Check Status** — looks the game's install folder up in
  `autogse list --json`'s output and reports vanilla/injected/needs-update/
  Steam-update-reverted.
- **AutoGSE: Inject** / **Revert** / **Reinject** — run the matching CLI verb
  against the game's install folder and show the result.

## Building

Requires the .NET SDK (net462 target, matching Playnite's own extension
host) and the `PlayniteSDK` NuGet package (fetched from nuget.org — not
vendored in this repo, unlike AutoGSE's own Goldberg tooling).

```
cd playnite-plugin
dotnet build -c Release
```

## Installing into a real Playnite instance

Copy the build output (`AutoGSEPlaynitePlugin.dll`, `extension.yaml`, and any
dependency DLLs from `bin/Release/net462/`) into a new folder under
Playnite's `Extensions` directory, e.g.:

```
%AppData%\Playnite\Extensions\AutoGSEPlaynitePlugin\
```

Playnite loads extensions from that folder on startup (Extensions ->
Reload Extensions works without a full restart, per Playnite's own docs).

## Configuration

The plugin resolves `autogse.exe` via the `AUTOGSE_EXE` environment
variable if set, otherwise assumes it's on `PATH`. AutoGSE's own installer
does not add itself to `PATH` (Phase 4 §4.2 registers Explorer context-menu
verbs, not a PATH entry), so most users will need to set `AUTOGSE_EXE` to
their real install location — this is a real, documented limitation of the
skeleton, not an oversight; a settings UI for picking the path is a natural
fast-follow, not built here.

## Contract this depends on

`AutoGseJsonTarget` in `AutoGsePlugin.cs` mirrors `engine::JsonTarget` in the
Rust CLI (`src/engine.rs`) field-for-field. If that Rust struct's fields
change, update this file to match — there is no shared schema file between
the two languages/toolchains.
