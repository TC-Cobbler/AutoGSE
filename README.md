# AutoGSE

A Windows CLI and Explorer context-menu tool that automates injecting the [Goldberg Steam Emulator](https://github.com/alex47exe/gse_fork) (`gse_fork` by alex47exe) into non-Steam game installs, so offline/cracked/DRM-free games can track achievements locally (and via Achievement Watcher).

Right-click a game's `.exe` or its folder → **Inject** or **Revert**. AutoGSE finds the right `steam_api(64).dll`, figures out the Steam App ID, runs the Goldberg config tooling, and writes an atomic manifest so the revert is always a clean, deterministic rollback.

## Standalone vs. with Cheevos

AutoGSE works entirely on its own — install it and the Explorer context menu is all you need. It has no dependency on any other app: `Inject`/`Revert` call `autogse.exe` directly, with no network requirement beyond Steam itself.

Separately, [Cheevos](https://github.com/TC-Cobbler/Cheevos) (a companion fork of PSerban93/Achievements) can drive AutoGSE as a subprocess for a full library dashboard — scanning, injecting, auditing, and exporting achievement data across your whole games folder from one UI, instead of one right-click at a time. It's optional: everything Cheevos does goes through the same `autogse.exe` CLI documented below, and nothing in AutoGSE requires Cheevos to function. See [`roadmap-cheevos-integration.md`](roadmap-cheevos-integration.md) if you want that.

## Features

- **Explorer context menu** — dynamic `Inject`/`Revert` entries (via a real `IExplorerCommand` shell extension) on both files and folders, no elevation needed to install/use.
- **Recursive target discovery** — BFS scan (up to 6 levels deep) for `steam_api.dll`/`steam_api64.dll`, handling deeply nested engine layouts (UE4/5, Unity, RE Engine, custom launchers) and PE bitness detection.
- **Automatic Steam App ID resolution** — cascading pipeline: local manifest files → PE version-resource strings → sanitized folder-name fuzzy match against the Steam store → interactive manual pick as a last resort.
- **Steam login (optional)** — DPAPI-encrypted credential storage so achievement names/descriptions/icons and Achievement Watcher schemas can be generated; falls back to anonymous mode (no achievement metadata) with an explicit opt-out. See "Steam credentials" below for why a login is needed at all.
- **Atomic inject/revert** — originals are backed up and SHA-256 hashed before anything is touched; revert restores byte-for-byte from the backup and removes everything AutoGSE added, tracked via a per-folder `.gse_manifest.json`. `repair`/`audit` diagnose and fix common integrity problems (orphaned backups, stale manifests) across a whole library.
- **Controller, inventory, mods & DLC** — automatic controller config download or manual `.vdf` import, inventory (`items.json`) generation, Steam Workshop mod folder scaffolding, DLC unlock-all.
- **`steamclient` (experimental) injection mode** — an alternate, non-DLL-swap injection path for games that need it, alongside the default DLL-swap mode.
- **LAN & multiplayer** — custom broadcast-IP peer list management, listen-port configuration, VPN adapter detection (Tailscale/ZeroTier/Radmin), and a wrapper around the vendored `lobby_connect` peer-discovery tool.
- **Save management** — Steam ↔ Goldberg save migration, Ludusavi-manifest-based save path discovery, and local (optionally cloud-folder-synced) achievement-progress backup/restore.
- **Portable export/import** — package an injected game's config into a single file and deploy it onto another vanilla copy fully offline, no network calls.
- **Anti-cheat/anti-tamper advisory scan** — flags EasyAntiCheat/BattlEye/VMProtect before injecting (advisory only, never blocks).
- **SteamStub unpacking (opt-in)** — wraps the vendored Steamless tool to strip SteamStub DRM before injection when needed.
- **RetroAchievements.org client** — separate from Goldberg/Steam entirely: login, game-progress fetch, and unlock-diff toast notifications for emulated console achievements.
- **Achievement data export** — `export-achievements` dumps every unlocked/locked achievement with timestamps, library-wide, as CSV or JSON.
- **Companion `--json` contract** — every data-producing command has a documented, stable `--json` output shape and exit-code table (see [`CONTRACT.md`](CONTRACT.md)), for Cheevos or any other external tool to drive AutoGSE without scraping console text.
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

autogse scan --root "D:\Games"                      # find every injectable game under a library root
autogse audit --root "D:\Games" --json               # library-wide integrity check
autogse doctor                                        # vendored-tool/DPAPI/login-state self-check
autogse export-achievements --format csv --out achievements.csv
```

On first use (if no credentials are stored yet and no anonymous preference is set), AutoGSE explains the login/anonymous tradeoff and asks once — the "don't ask again" choice is remembered in `%LOCALAPPDATA%\AutoGSE\preferences.json`.

See [`CONTRACT.md`](CONTRACT.md) for the complete subcommand list (including hidden ones like `join`, `parse-controller-vdf`, `add-mod`), every `--json` shape, and the exit-code table.

## Steam credentials: why not just an API key?

AutoGSE asks for your actual Steam username/password rather than a Steam Web API key, and that's a real, deliberate constraint, not an oversight:

- Full achievement data (names, descriptions, icons, and Achievement Watcher schemas) comes from the vendored `generate_emu_config` tool's `-acw`/`-tok` flags, which talk to **Steam's internal Connection Manager protocol** (the real Steam client login protocol, TCP port 27017) — not the public HTTPS Web API. That data is genuinely unavailable to an anonymous session or a plain API key; only a real authenticated login unlocks it.
- Separately, Valve's public `ISteamApps/GetAppList/v2` endpoint (which a Web API key *would* unlock) is deprecated. AutoGSE's own App ID fuzzy-matching step avoids needing a key at all by using the unauthenticated `store.steampowered.com/api/storesearch` endpoint instead — so a Web API key wouldn't even help there.
- Credentials are DPAPI-encrypted and stored only on your machine (`%LOCALAPPDATA%\AutoGSE\credentials.dat`), tied to your Windows user + machine automatically. They're never written to a plaintext file, never sent anywhere except to Steam itself, and the `login` subcommand deliberately has no `--username`/`--password` flags so they can't end up in shell history or Explorer's "Recent commands."
- `--anon` (or declining the first-run prompt) skips all of this — injection, DLC/mod config, and everything else that doesn't need achievement metadata works fully anonymously.

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
cargo build --release --workspace   # --workspace is required: the plain-package build alone
                                     # skips the shell-ext member and won't produce autogse_shell.dll
cargo test --workspace
```

The release binary targets < 15 MB and statically links the MSVC CRT. `installer/autogse.iss` packages both binaries together with the vendored `alex47exe-gse_fork`/Steamless tooling (see `installer/ATTRIBUTION.txt`).

## Project status

`v1.0.0` — the core engine, full GSE feature coverage (controller/mods/DLC, LAN/multiplayer, save sync, portable export/import, anti-cheat scanning, SteamStub unpacking), shell integration, and the Cheevos companion `--json` contract are all done and considered stable. See [`roadmap.md`](roadmap.md) for AutoGSE's own phase-by-phase history against the [PRD](AutoGSE_Product_Requirement_Document.md), and [`roadmap-cheevos-integration.md`](roadmap-cheevos-integration.md) for the Cheevos companion-app story. New user-facing features (overlays, playtime tracking, rarity metrics, store/launcher integrations, any GUI) are no longer planned in AutoGSE itself — that surface belongs to Cheevos now; `roadmap.md`'s own top-of-file notice explains why.

## Credits

AutoGSE bundles and wraps [`gse_fork`](https://github.com/alex47exe/gse_fork) by alex47exe, itself built on [Mr_Goldberg's Steam emulator](https://gitlab.com/Mr_Goldberg/goldberg_emulator). AutoGSE does not modify or relicense that tooling — see `installer/ATTRIBUTION.txt`. Review `gse_fork`'s own license before redistributing.
