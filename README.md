# AutoGSE

A zero-GUI Windows CLI and Explorer context-menu tool that automates injecting the [Goldberg Steam Emulator](https://github.com/alex47exe/gse_fork) (`gse_fork` by alex47exe) into non-Steam game installs, so offline/cracked/DRM-free games can track achievements locally (and via Achievement Watcher).

Right-click a game's `.exe` or its folder → **Inject** or **Revert**. AutoGSE finds the right `steam_api(64).dll`, figures out the Steam App ID, runs the Goldberg config tooling, and writes an atomic manifest so the revert is always a clean, deterministic rollback.

## Features

- **Explorer context menu** — `Inject`/`Revert` entries on both files and folders, no elevation needed to install/use.
- **Recursive target discovery** — BFS scan (up to 6 levels deep) for `steam_api.dll`/`steam_api64.dll`, handling deeply nested engine layouts (UE4/5, Unity, RE Engine, custom launchers) and PE bitness detection.
- **Automatic Steam App ID resolution** — cascading pipeline: local manifest files → PE version-resource strings → sanitized folder-name fuzzy match against the Steam store → interactive manual pick as a last resort.
- **Steam login (optional)** — DPAPI-encrypted credential storage so achievement names/descriptions/icons and Achievement Watcher schemas can be generated; falls back to anonymous mode (no achievement metadata) with an explicit opt-out.
- **Atomic inject/revert** — originals are backed up and SHA-256 hashed before anything is touched; revert restores byte-for-byte from the backup and removes everything AutoGSE added, tracked via a per-folder `.gse_manifest.json`.
- **Desktop toast notifications** — success/error/rollback feedback via native Windows toasts, including for silent/context-menu-triggered runs.
- **Single self-contained binary** — no installed runtime dependencies, Windows Defender-clean, unsigned (SmartScreen will warn on first run).

## Installing

Run the InnoSetup installer (`dist/AutoGSE-Setup-*.exe`, built from `installer/autogse.iss`). It installs to `Program Files`, registers the context-menu entries, and creates the Start Menu shortcut toast notifications require. Uninstalling removes all three cleanly.

## Usage

```bash
# Right-click in Explorer, or from a terminal:
autogse inject --path "D:\Games\SomeGame"          # auto-detect everything
autogse inject --path "D:\Games\SomeGame.exe" --appid 1234560   # force an App ID
autogse inject --path "D:\Games\SomeGame" --anon    # skip Steam login for this run
autogse inject --path "D:\Games\SomeGame" --silent  # no console, toast only

autogse revert --path "D:\Games\SomeGame"           # roll back to vanilla

autogse login    # store Steam credentials (DPAPI-encrypted, this PC only)
autogse logout   # remove stored credentials
```

On first use (if no credentials are stored yet and no anonymous preference is set), AutoGSE explains the login/anonymous tradeoff and asks once — the "don't ask again" choice is remembered in `%LOCALAPPDATA%\AutoGSE\preferences.json`.

## How it works

1. **Discover** — resolve the target directory and DLL from the path given, verify 32/64-bit via the PE header.
2. **Identify** — resolve the Steam App ID via `steam_appid.txt` → PE metadata → fuzzy name match → interactive prompt.
3. **Back up** — rename `steam_api(64).dll` → `.org`, hash it.
4. **Generate** — invoke the vendored `generate_emu_config` tooling (anonymous or authenticated) to build `steam_settings/` (`configs.*.ini`, achievements, interfaces).
5. **Inject** — copy the Goldberg emulator DLL into place, write `steam_appid.txt` and `steam_settings/`.
6. **Record** — write `.gse_manifest.json` listing every backed-up and injected file with hashes, so `revert` is exact and idempotent.

## Building from source

Requires the Rust toolchain (`x86_64-pc-windows-msvc`) and Inno Setup (for the installer only).

```bash
cargo build --release
cargo test
```

The release binary targets < 15 MB and statically links the MSVC CRT. `installer/autogse.iss` packages the binary together with the vendored `alex47exe-gse_fork` tooling (see `installer/ATTRIBUTION.txt`).

## Project status

Phases 1-5 (core engine, discovery, GSE integration, polish/delivery, Steam login) are implemented and tested. See [`roadmap.md`](roadmap.md) for detailed, checkbox-tracked progress against the [PRD](AutoGSE_Product_Requirement_Document.md), including Phase 6's planned work (deeper GSE feature coverage, batch/library operations, diagnostics).

## Credits

AutoGSE bundles and wraps [`gse_fork`](https://github.com/alex47exe/gse_fork) by alex47exe, itself built on [Mr_Goldberg's Steam emulator](https://gitlab.com/Mr_Goldberg/goldberg_emulator). AutoGSE does not modify or relicense that tooling — see `installer/ATTRIBUTION.txt`. Review `gse_fork`'s own license before redistributing.
