# AutoGSE

A Windows CLI (with an Explorer right-click menu on top) that injects the [Goldberg Steam Emulator](https://github.com/alex47exe/gse_fork) (`gse_fork` by alex47exe) into non-Steam game installs, so your offline, cracked, or otherwise DRM-free games can still track achievements locally — and show up in Achievement Watcher.

Right-click a game's `.exe` or its folder and pick **Inject** or **Revert**. AutoGSE works out which `steam_api(64).dll` you need, figures out the Steam App ID, runs the Goldberg config tooling, and writes a manifest as it goes so `Revert` can always put things back exactly the way they were.

## Standalone vs. with Cheevos

You don't need anything else to use AutoGSE — install it and the Explorer context menu is all there is to it. `Inject`/`Revert` just call `autogse.exe` directly, no other app involved, no network requirement beyond Steam itself.

If you'd rather manage a whole library instead of one right-click at a time, [Cheevos](https://github.com/TC-Cobbler/Cheevos) (a companion fork of PSerban93/Achievements) drives AutoGSE as a subprocess and wraps it in a dashboard — scanning, injecting, auditing, and exporting achievement data across your whole games folder from one UI. It's entirely optional and goes through the same `autogse.exe` CLI documented below; AutoGSE itself has no idea Cheevos exists. See [`roadmap-cheevos-integration.md`](roadmap-cheevos-integration.md) if you're curious how that split came about.

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

Run the installer (built from the Inno Setup script at `installer/autogse.iss`). It installs to `Program Files`, registers the context-menu entries, and creates the Start Menu shortcut that toast notifications need to work. Uninstalling takes all three back out cleanly.

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

AutoGSE asks for your actual Steam username and password instead of a Web API key. That's on purpose, not laziness — here's why:

- Full achievement data (names, descriptions, icons, and Achievement Watcher schemas) comes from the vendored `generate_emu_config` tool's `-acw`/`-tok` flags, which talk to **Steam's internal Connection Manager protocol** (the real Steam client login protocol, over TCP port 27017), not the public HTTPS Web API. That data just isn't reachable from an anonymous session or a plain API key — only a real authenticated login gets you there.
- On top of that, the public `ISteamApps/GetAppList/v2` endpoint a Web API key would normally unlock is deprecated anyway. AutoGSE's App ID fuzzy-matching sidesteps the need for a key entirely by hitting the unauthenticated `store.steampowered.com/api/storesearch` endpoint instead, so a key wouldn't buy you anything there either.
- Your credentials are DPAPI-encrypted and never leave your machine (`%LOCALAPPDATA%\AutoGSE\credentials.dat`), tied automatically to your Windows user and this PC. They're never written to disk in plaintext, never sent anywhere but Steam, and the `login` subcommand has no `--username`/`--password` flags on purpose, so they can't end up sitting in your shell history or Explorer's "Recent commands."
- Don't want to log in at all? `--anon` (or just declining the first-run prompt) skips all of this. Injection, DLC/mod config, and everything else that doesn't need achievement metadata still works fine anonymously.

## How it works

1. **Discover** — resolve the target directory and DLL from the path you gave it, verify 32/64-bit via the PE header.
2. **Identify** — work out the Steam App ID, trying `steam_appid.txt`, then PE metadata, then a fuzzy name match, then falling back to an interactive prompt if nothing else worked.
3. **Back up** — rename `steam_api(64).dll` to `.org` and hash it.
4. **Generate** — run the vendored `generate_emu_config` tooling (anonymous or authenticated) to build `steam_settings/` (`configs.*.ini`, achievements, interfaces).
5. **Inject** — drop the Goldberg emulator DLL into place, write `steam_appid.txt` and `steam_settings/`.
6. **Record** — write `.gse_manifest.json` listing every backed-up and injected file along with hashes, so `revert` can undo it exactly, every time.

## Building from source

Requires the Rust toolchain (`x86_64-pc-windows-msvc`) and Inno Setup (for the installer only).

```bash
cargo build --release --workspace   # --workspace is required: the plain-package build alone
                                     # skips the shell-ext member and won't produce autogse_shell.dll
cargo test --workspace
```

The release binary targets < 15 MB and statically links the MSVC CRT. `installer/autogse.iss` packages both binaries together with the vendored `alex47exe-gse_fork`/Steamless tooling (see `installer/ATTRIBUTION.txt`).

## Project status

`v1.0.0` — the core engine, full GSE feature coverage (controller/mods/DLC, LAN/multiplayer, save sync, portable export/import, anti-cheat scanning, SteamStub unpacking), shell integration, and the Cheevos companion `--json` contract are all done and considered stable. New user-facing features (overlays, playtime tracking, rarity metrics, store/launcher integrations, any GUI) aren't planned for AutoGSE itself anymore — that surface belongs to Cheevos now. See [`roadmap-cheevos-integration.md`](roadmap-cheevos-integration.md) for how that split came about.

## Credits

AutoGSE bundles and wraps [`gse_fork`](https://github.com/alex47exe/gse_fork) by alex47exe, itself built on [Mr_Goldberg's Steam emulator](https://gitlab.com/Mr_Goldberg/goldberg_emulator). AutoGSE doesn't modify or relicense that tooling — see `installer/ATTRIBUTION.txt` — and you should check `gse_fork`'s own license before redistributing anything.
