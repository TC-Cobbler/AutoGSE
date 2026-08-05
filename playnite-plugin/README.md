# AutoGSE Playnite Plugin

Phase 11 §11.4. A thin Playnite `GenericPlugin` that shells out to the real
`autogse.exe` CLI — it does not reimplement any injection logic. Not a
polished/store-submitted extension, but genuinely live-verified end to end,
not just "compiles": loaded into a real running Playnite install (confirmed
via `playnite.log`'s `ExtensionFactory:Loaded plugin: AutoGSE Integration,
version 0.1.0`), and the full per-game context-menu flow (Check Status,
Inject, Revert, Reinject, including the App-ID-prompt fallback below) has
been exercised against a real game library and real installed games. See
`roadmap.md`'s Phase 11 §11.4 for the complete bug-fix history — several
real, non-obvious bugs (a cleared NuGet package-source list, a missing WPF
reference, a registry-view mismatch, `revert` erasing AutoGSE's own App-ID
memory, and Playnite storing `{InstallDir}`-style tokens instead of literal
paths) were only found by actually running this against real data, not by
reading the SDK docs alone.

## What it does

Per-game context menu items (only shown when the game has a resolvable,
existing `InstallDirectory`):

- **AutoGSE: Check Status** — looks the game up in `autogse list --json`'s
  output (matching any known target equal to or nested under the game's
  install folder, not just an exact path) and reports vanilla/injected/
  needs-update/Steam-update-reverted.
- **AutoGSE: Inject** — resolves the game's real Play-action executable (via
  `IPlayniteAPI.ExpandGameVariables`, not just the bare install folder) so
  AutoGSE's own discovery picks the correct `steam_api(64).dll` even when a
  bundled tool/mod-uploader elsewhere in the game's folder ships its own
  decoy copy. Reuses a Steam App ID from Playnite's own Steam-library data
  or AutoGSE's own memory when available; if neither has one and AutoGSE's
  automatic resolution can't confidently pick one either, prompts once for a
  manual App ID (with a hint on where to find it) instead of just failing.
- **AutoGSE: Revert** / **Reinject** — run the matching CLI verb against the
  same resolved executable path and show the result.

## Building

Requires the .NET SDK (net462 target, matching Playnite's own extension
host) and the `PlayniteSDK` NuGet package (fetched from nuget.org — not
vendored in this repo, unlike AutoGSE's own Goldberg tooling).

```
cd playnite-plugin
dotnet build -c Release
```

## Installing — recommended: a single `.pext` file

Playnite ships its own packaging tool (`Toolbox.exe`, found at
`%LocalAppData%\Playnite\Toolbox.exe` on any real Playnite install) that
packs an extension folder into a single `.pext` file — a real Playnite
mechanism, not a workaround: end users install it by double-clicking the
file (or via Playnite's Add-ons browser → gear icon → "Install add-on from
file"), no manual folder-copying needed.

```
"%LocalAppData%\Playnite\Toolbox.exe" pack bin\Release\net462 dist
```

Produces `dist\<Id>_<version-with-underscores>.pext` — rename it to
something readable (e.g. `AutoGSEPlaynitePlugin-0.1.0.pext`) before
distributing; the name itself doesn't matter to Playnite, which reads the Id/
version from the packed `extension.yaml`. `Toolbox.exe`'s separate `verify`
command checks against a *stricter* manifest schema (`AddonId`,
`ShortDescription`, `SourceUrl`, `InstallerManifestUrl`, ...) meant for
submitting to Playnite's official Add-on Database — confirmed live that
`pack` itself does **not** require any of those extra fields, so this
repo's existing minimal `extension.yaml` (Id/Name/Author/Version/Module) is
sufficient for a real, working, personally-distributed `.pext`. `dist/` here
is already covered by this repo's top-level `.gitignore` (`dist/` matches at
any depth), so packaged output isn't accidentally committed.

## Installing — manual/development

Alternatively, copy the build output (`AutoGSEPlaynitePlugin.dll`,
`extension.yaml`, and its dependency DLLs from `bin\Release\net462\`) into a
new folder under Playnite's `Extensions` directory, e.g.:

```
%AppData%\Playnite\Extensions\AutoGSEPlaynitePlugin\
```

Playnite loads extensions from that folder on startup (Extensions ->
Reload Extensions works without a full restart, per Playnite's own docs) —
this is the faster loop for iterating on the plugin itself; the `.pext` flow
above is for actually distributing/installing it.

## Configuration

`autogse.exe` is located automatically, in order: the `AUTOGSE_EXE`
environment variable if set (an explicit override) → the real install
location, read directly from the registry key AutoGSE's own Inno Setup
installer writes automatically (no configuration needed for a normal
install) → a bare `autogse.exe` on `PATH` as a last resort. Most users
installing AutoGSE the normal way need to set nothing at all; `AUTOGSE_EXE`
exists for anyone running a non-standard/portable AutoGSE copy.

## Contract this depends on

`AutoGseJsonTarget` in `AutoGsePlugin.cs` mirrors `engine::JsonTarget` in the
Rust CLI (`src/engine.rs`) field-for-field. If that Rust struct's fields
change, update this file to match — there is no shared schema file between
the two languages/toolchains.
