# AutoGSE Playnite Plugin

Phase 11 §11.4. A thin Playnite `GenericPlugin` that shells out to the real
`autogse.exe` CLI — it does not reimplement any injection logic. Not a
polished/store-submitted extension, but past the "skeleton" stage: it builds
clean (`dotnet build -c Release`, 0 warnings/0 errors) and has been live-
loaded into a real running Playnite install with a real game library —
confirmed via `playnite.log`'s own `ExtensionFactory:Loaded plugin: AutoGSE
Integration, version 0.1.0` line, not just assumed from a successful compile.
See `roadmap.md`'s Phase 11 §11.4 for the full verification history,
including two real environment bugs found and fixed only by actually
building this (a cleared NuGet package-source list, and a missing WPF
`PresentationFramework` reference `MessageBoxResult` needs). The one thing
still not verified is the actual per-game context-menu click-through (right-
click a real game, confirm all four menu items appear and work) — that needs
real mouse interaction inside the Playnite window, not something automatable
from here.

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
their real install location — this is a real, documented limitation, not an
oversight; a settings UI for picking the path is a natural fast-follow, not
built here.

## Contract this depends on

`AutoGseJsonTarget` in `AutoGsePlugin.cs` mirrors `engine::JsonTarget` in the
Rust CLI (`src/engine.rs`) field-for-field. If that Rust struct's fields
change, update this file to match — there is no shared schema file between
the two languages/toolchains.
