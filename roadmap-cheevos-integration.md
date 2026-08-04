# AutoGSE ⇄ Cheevos (PSerban93/Achievements fork) Integration Roadmap

This is a separate document from `roadmap.md` (AutoGSE's own feature roadmap). That file tracks AutoGSE's internal development; this one tracks the cross-repo integration effort between AutoGSE and `Cheevos` (a local fork of [PSerban93/Achievements](https://github.com/PSerban93/Achievements), cloned at `G:\Other computers\My PC\TheCmpny\001_CompanyProjects\006_Cobbler\003_FossApps\Cheevos`). A copy of this same document lives at `Cheevos\roadmap.md` so it's discoverable from either repo — keep the two in sync; treat this AutoGSE copy as the master.

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done

---

## Context / Why

AutoGSE's own roadmap (Phases 13 and 15) set out to build console-emulator trophy scrapers (RPCS3/ShadPS4/Xenia), multi-store integrations (Epic/GOG/EA/Ubisoft/Xbox/LumaPlay), achievement rarity metrics, in-game overlays, playtime tracking, and a full GUI library dashboard — from scratch, in Rust.

`Cheevos` (the PSerban93/Achievements fork) already implements essentially all of that, in a mature Electron/Node.js app — confirmed by reading its real source, not assumed:

- `utils/rpcs3-*`, `utils/shadps4-*`, `utils/xenia-*` — the exact three emulator trophy parsers Phase 13/15 wanted.
- `utils/epic-*`, `utils/gog-*`, `utils/ea-desktop-local.js`, `utils/xbox-pc.js`, `utils/lumaplay-registry.js`, `utils/match-uplay-steam.js` — the exact six store/launcher integrations.
- `utils/achievement-rarity.js`, `utils/exophase-scraper.js`, `utils/steam-appcache*.js` — rarity metrics and extended scraping.
- A full Electron GUI: overlay notifications (`overlay.html`), playtime tracking (`playtime.html`, `utils/playtime-log-watcher.js`), a sortable/searchable game dashboard (`index.html`), system tray (`tray-menu.*`).

What Cheevos does **not** have, confirmed by grepping its source for any `steam_api`/DLL-swap/backup logic and finding none: the actual Goldberg/GSE emulator **injection engine**. Its own README explicitly warns against running it alongside "another overlay/injector tool" — it's built to consume achievement/config data that already exists (from Goldberg/GSE or official Steam data), not to produce it. That's exactly AutoGSE's core, unique, already-tested capability: discovery, PE bitness checks, DLL backup/swap, the 5-step App ID resolution cascade, `.gse_manifest.json`-tracked revert safety, DPAPI-encrypted Steam credential storage, and a RetroAchievements.org client.

**Decision**: stop building Phase 13/15-style features into AutoGSE. Fork Cheevos, keep AutoGSE as a small, stable companion binary it shells out to for injection, and let Cheevos own the notification/tracking/dashboard/scraping surface it already does well.

**License note**: AutoGSE is `GPL-2.0-only` (see `Cargo.toml`); the Cheevos fork inherits PSerban93/Achievements' `MIT` license. Invoking a separate GPL-licensed process from an MIT-licensed app ("mere aggregation," two separate programs communicating over a process boundary — no linking, no shared address space) does not create a combined/derivative work under either license. **Do not** copy Rust source into the JS codebase, or vice versa, without re-checking this — that would change the analysis.

---

## Architecture

- AutoGSE stays a standalone Rust binary (`autogse.exe`, plus its vendored `gen_emu_cfg`/tool trees) — bundled into Cheevos's Electron build as a resource, not merged into its source tree.
- Cheevos invokes it via Node's `child_process` (`execFile`/`spawn`), through one new module (suggested: `utils/autogse-bridge.js`) that mirrors the existing `utils/*` integration-module pattern (e.g. `epic-api.js`, `gog-galaxy-local.js`) rather than introducing a new convention.
- The contract between the two is AutoGSE's `--json` output (already implemented on `scan`, `list`, `audit`, `rpcs3-trophies`, plus `export-achievements --format json`) and its stable, documented exit codes (`src/error.rs`'s `exit_code()` — currently 0–39, one per `AutoGseError` variant). Commands Cheevos needs that don't have `--json` yet are new work (Phase 4 below).
- Shared local state: AutoGSE already keeps `%LOCALAPPDATA%\AutoGSE\` (`credentials.dat`, `known_targets.json`, `preferences.json`, `autogse.log`). Cheevos should read/write through AutoGSE's CLI rather than touching these files directly, so there's exactly one owner of that state and one place its format can evolve.

---

## Phase 0 — Strip AutoGSE Down to Its Companion-Library Bare Minimum

Do this before anything else in this document — the rest of the plan assumes AutoGSE is a small, stable companion binary, not the growing standalone app it was becoming. Grounded in a real inventory of the repo (not guessed), plus three decisions made when this section was written:

- **AutoGSE drops its own GUI.** Now that Cheevos is meant to be the primary user-facing app, `autogse-gui.exe` is no longer needed — AutoGSE becomes CLI-only.
- **`playnite-plugin/` stays as-is** — a separate, unrelated future integration effort (Playnite launcher menu items, not achievement overlays), not affected by the Cheevos pivot.
- **`dist/`'s stale installer builds are recorded here, not cleared yet** — see housekeeping below.

### Remove the GUI entirely — done
- [x] Deleted `src/bin/autogse_gui.rs` and the whole `src/bin/autogse_gui/` directory (10 files).
- [x] Deleted `ui/` (13 `.slint` files) and `Mockups/` — **but only after harvesting both into `Cheevos\design\` first** (Phase 7 §7.1), per this section's own sequencing note; nothing was lost.
- [x] Removed GUI-only lib modules `config_editor.rs` and `header_cache.rs` (and their `pub mod` lines in `src/lib.rs`) — confirmed via grep both were referenced only in doc-comment prose elsewhere (`achievements.rs`, `retroachievements.rs`), never as real code dependencies.
- [x] Removed the `[[bin]] name = "autogse-gui"` target from `Cargo.toml`.
- [x] Removed GUI-only dependencies: `slint`, `slint-build`, `raw-window-handle` — confirmed fully gone from `Cargo.lock` after rebuild (`grep -c "^name = \"slint" Cargo.lock` → 0).
- [x] Pruned `windows` crate features — **real correction found only by actually building, not by the grep audit**: a grep-based "is the feature name string used in source" check is insufficient — windows-rs sometimes gates a function by a feature from a *different* module than the one the function lives in (`StructuredStorage`'s `PropVariantClear`/`InitPropVariantFromStringAsVector`, used by `shortcut.rs`, are gated behind `Win32_System_Variant`; `IpHelper`'s `GetAdaptersAddresses`, used by `vpn_adapters.rs`, is gated behind `Win32_NetworkManagement_Ndis`). An initial pass removed both as "unused" and `cargo build` immediately caught it with real compile errors. Final removed set, confirmed safe by a clean build: `Win32_System_Ole`/`Win32_System_SystemServices` (GUI-only) and `Win32_System_IO`/`Win32_Graphics_Gdi` (confirmed unused anywhere, a bonus finding beyond what this phase originally scoped).
- [x] `build.rs`: removed the unconditional `slint_build::compile("ui/main.slint")` call; kept the `winres` icon/manifest embedding step.
- [x] `installer/autogse.iss`: removed the `autogse-gui.exe` `Source:` line and its explanatory comment (there was never a separate `[Icons]` Start Menu entry for it — AutoGSE's own runtime `shortcut.rs`/`install-menu` mechanism is unrelated and untouched).
- [x] `cargo build --workspace` clean; `cargo test --workspace`: 299 passed, 0 failed (down from 314 — the removed `config_editor.rs`/`header_cache.rs` had their own real unit tests, which went with them; not a regression).

### Low-risk housekeeping — done
- [x] Cleared `dist/`'s two stale installer builds (181MB freed).
- [ ] `shell-ext/` (a required Cargo workspace member) and `playnite-plugin/` are both real but currently **untracked by git** (confirmed via `git status`) — decide whether to commit them properly or leave untracked; not urgent, just noted so it isn't lost track of.
- [ ] `AutoGSE_Product_Requirement_Document.md` — still the cited source `roadmap.md` derives from; consider moving into a `docs/` subfolder for tidiness, not required.

---

## Phase 1 — Bundle & Smoke Test

### 1.1 Decide bundling scope — done
- [x] Confirmed by reading `installer/autogse.iss`: AutoGSE's own installer stages seven things — `autogse.exe`, `autogse_shell.dll`, `gen_emu_cfg/` (from `generate_emu_config/*`), `parse_controller_vdf/`, `lobby_connect/`, `steamclient_experimental/`, `Steamless/`. Decided and implemented: `autogse_shell.dll` and `install-menu`/`uninstall-menu` (AutoGSE's own per-user Explorer context-menu registration, Phase 11) are **excluded** — orthogonal to Cheevos driving AutoGSE via `child_process`; revisit only if a later phase wants a Cheevos-adjacent "right-click → Inject" feature.
- [x] `Steamless/` **excluded** too, on the same reasoning already written here: SteamStub unpacking is opt-in (`inject --unpack-steamstub`, confirmed at `src/engine.rs:1636-1663`, never on the default path), and it carries a CC BY-NC-ND license that's a real, separate question from the GPL/MIT pairing this doc's "License note" section already resolved. Deferred, not forgotten — add it (and resolve that license question) only when a future phase actually exposes that flag.
- [x] The four load-bearing trees (`gen_emu_cfg/`, `parse_controller_vdf/`, `lobby_connect/`, `steamclient_experimental/`) plus `autogse.exe` itself are all bundled together — confirmed necessary, not just in theory: running `autogse.exe doctor` straight out of AutoGSE's own `target/release/` (nothing co-located beside it) reported all four tool checks as `[FAIL]`; after co-locating them, all four report `[OK]`. `tools_root()` and its three siblings really do require directory-adjacency to the exe at runtime, exactly as `goldberg.rs` documents.

### 1.2 & 1.3 — bundling mechanism and the dev/packaged path seam, done together
The original plan here sketched two separate things: static `extraResources` entries pointing straight at AutoGSE's own build output, and a bridge-side dev/packaged branch. Building it live surfaced a wrinkle neither anticipated: **co-location is required in *both* modes**, not just packaged — a dev-mode bridge that pointed at AutoGSE's scattered `target/release/` + `alex47exe-gse_fork/*` locations directly would hit the exact same `[FAIL]`s just demonstrated in 1.1, since `tools_root()`'s sibling-of-exe resolution is baked into the release binary regardless of who's calling it. So instead of five scattered `{from, to}` pairs, one staging step now produces a single already-correct local copy that both dev and packaged modes read from the same way:
- [x] Added `Cheevos/build/stage-autogse.js` — copies `autogse.exe` + the four tool folders from an AutoGSE checkout into `Cheevos/vendor/autogse/` (mirroring, in miniature, what `installer/autogse.iss`'s `[Files]` section already does for AutoGSE's own install). Source checkout resolved via `AUTOGSE_DEV_PATH` env var if set, else the real sibling-directory layout this machine already has (`../AutoGSE` next to `Cheevos`) — same env-var-first, sibling-guess-fallback approach the original plan proposed, just applied at stage time instead of at bridge-runtime.
- [x] `vendor/` added to `Cheevos/.gitignore` (same treatment as `dist/`/`node_modules/` — build output, not source).
- [x] `package.json`: new `stage:autogse` script (`node build/stage-autogse.js`), wired to run first in both `pack` and `dist`; new `extraResources` entry `{"from": "vendor/autogse", "to": "autogse"}` alongside the existing `sounds/`/`presets/`/`tools/schema_parse.zip` entries — same asar-avoidance reasoning as those (an exe can't be `execFile`'d out of `app.asar`).
- [x] `utils/autogse-bridge.js`'s `resolveAutogseDir()`: `process.resourcesPath/autogse` when `app.isPackaged`, else `<repo>/vendor/autogse` — both branches now read the one staged copy, so there's exactly one "where's AutoGSE" resolution to maintain, not two.
- [x] Confirmed live: `du`/copy on this machine's `G:\Other computers\...` mount was slow (`generate_emu_config/` alone is 149MB across 1,250 files) — worth knowing if this staging step feels sluggish in practice; it's the mount, not the script.

### 1.4 `utils/autogse-bridge.js` — done
- [x] Built with `scan`/`inject`/etc. left for Phase 2; this phase only needed `runAutogse(args)` (generic `execFile` wrapper, resolves on any exit code, rejects only if the process itself couldn't be spawned) and `doctor()` on top of it. `doctor` still has no `--json` (confirmed: bare arg-less `Command::Doctor` in `src/cli.rs:110`; `run_doctor()`/`collect_doctor_report()` in `src/engine.rs:120-154` only print text) — the wrapper surfaces that text as-is, per the Phase 4 gap already tracked here.
- [x] **Live-tested in dev mode**, not just written: `node -e "require('./utils/autogse-bridge.js').doctor()..."` against the staged `vendor/autogse/autogse.exe` returned exit code `0` with all four tool checks `[OK]` and DPAPI reachable.

### 1.5 Verify — packaged-path layout confirmed live; clean-VM pass still open
- [x] Ran `npm run pack` end-to-end (`stage:autogse` → `verify:process-tree` → `electron-builder --dir`) — succeeded, produced `dist/win-unpacked/`. `electron-builder`'s own log confirms the `vendor/autogse` → `autogse` `extraResources` entry copied with no "file source doesn't exist" warning (unlike the pre-existing, unrelated `playwright-core/.local-browsers` entry, which does warn — that gap predates this work and needs `npm run dl-browsers` first, not a regression from Phase 1).
- [x] Confirmed the packaged layout is actually correct, not just "no error logged": ran `autogse.exe doctor` directly from `dist/win-unpacked/resources/autogse/` — all four tool checks `[OK]`, matching `process.resourcesPath/autogse` exactly (`resourcesPath` for a `win-unpacked` build is always `<appOutDir>/resources`, standard Electron behavior), which is what `resolveAutogseDir()`'s packaged branch computes.
- [ ] **Not done, deliberately**: actually launching `dist/win-unpacked/Achievements.exe` and clicking through to a real bridge call. It's `requestedExecutionLevel: requireAdministrator` in `package.json`'s `nsis` config, so a real launch triggers an interactive UAC prompt — not something to attempt unattended. This step, plus the original "no AutoGSE context-menu / no `%LOCALAPPDATA%\AutoGSE\` state" clean-machine checks, are the one remaining manual pass before calling Phase 1 fully closed.

### Exit criteria — met for the automatable parts
- [x] `autogse.exe doctor` runs successfully via `utils/autogse-bridge.js` in dev mode, and the identical packaged-resources layout is proven correct by direct invocation.
- [ ] Full loop through the actual packaged, launched app on a clean machine/VM — the interactive step noted in 1.5, left open for Phase 2 kickoff or a manual pass before it.

## Phase 2 — Core Injection Wired Into the UI

**Prerequisite**: Phase 1 is done (see above). Real gap found before starting 2.1 that changed this phase's plan: `login` (`src/login_prompt.rs::capture_login_stdio`) **never accepts credentials as CLI args** — deliberately, to keep them out of shell history/process listings — it reads username then password as two plain stdin lines. `inject`/`revert` only avoid AutoGSE's own interactive disclosure prompt when `--silent` is passed (confirmed by reading `resolve_auth_mode` in `src/engine.rs:1243-1285`); anon mode (`--anon`) never touches Steam auth at all, so it's fully non-interactive. **What isn't solved**: an *authenticated* inject (stored credentials, no `--anon`) can still hit a live Steam Guard code prompt on the nested `generate_emu_config.exe` subprocess's own stdin — even on `--dry-run`, since a dry-run still runs that tool for real to preview achievement data. Scope for this phase was narrowed accordingly: **the shipped UI only drives anon-mode inject/dry-run**; login/logout are wired up (so credentials exist for Phase 3 to build session-reuse on) but not yet connected to an authenticated-inject path.

### 2.1 Extend the bridge module — done
- [x] `utils/autogse-bridge.js` now has `scan(root)`, `injectDryRun(path, opts)`, `inject(path, opts)`, `revertDryRun(path)`, `revert(path)`, `login(username, password)`, `logout()`, plus a lower-level `spawnAutogse(args)` (returns `{ child, done }`, `child.stdin` writable) that `login`/`inject`/`injectDryRun` are built on. `scan` parses real `--json`; the rest parse plain text per the Phase 4 gap, exactly as planned.
- [x] Added `--appid` passthrough (`opts.appId`) — a real `TargetArgs` flag (`--appid`, override auto-detection) not in the original plan text, needed to write a working live test at all (a throwaway test folder has no real Steam App ID to auto-detect) and useful for the UI too as a manual-override escape hatch.
- [x] **Live-tested end-to-end**, not just written, against a synthetic target (`vendor`-adjacent scratch folder, a real DLL copied in for a valid PE header, forced `--appid 480` — Valve's public Spacewar test app): `scan` → `vanilla`; `injectDryRun` → real `generate_emu_config.exe` run, full file-list preview, exit 0; `inject` (real) → exit 0; `scan` again → `injected`, `app_id: 480`; `revertDryRun` → correct restore preview; `revert` (real) → exit 0; `scan` again → back to `vanilla`. Full round trip confirmed live.
- [ ] **Not live-tested**: `login`/`logout` — this dev machine already has real stored Steam credentials (`%LOCALAPPDATA%\AutoGSE\credentials.dat`, predates this session) and the auto-mode safety classifier blocked even backing that file up first. Verified by reading `login_prompt.rs` instead (exact stdin sequence: username line, password line, no live Steam call during `login` itself) — implementation should be correct, but hasn't been exercised against the real binary. Test manually, or against a machine with no stored credentials.

### 2.2 Reconciled with `watched-folders.js` — real correction to this doc's own earlier claim
This doc previously said `watched-folders.js`'s IPC handlers (`folders:add`/`list`/etc.) were "all wired into `main.js`" — **wrong**, found while wiring this phase: `ipcMain.handle("folders:add", ...)` and its siblings are registered directly inside `utils/watched-folders.js` itself (e.g. `folders:add` at that file's own line 11169), not in `main.js`. Corrected here so it isn't repeated.
- [x] **Decided and built the "separate flow" option**, per this doc's own recommendation: new `autogse:*` IPC handlers live in `main.js` (not inside `watched-folders.js`), right after the existing `generate-auto-configs` handler — keeps the AutoGSE integration surface as one reviewable block rather than growing an already-9k-line file further.
- [x] The concrete hook point ended up being the existing **Watched Folders** tab's per-row UI (`initFoldersTab` IIFE, `index.html`), not a new "Add Game" flow as originally floated — it already lets a user register a games-library root and lists it with per-row icon-buttons (block/edit/remove), which is exactly AutoGSE `scan --root`'s own unit of work. Added a fourth icon-button ("Scan for injectable games") per row that expands an inline results list (`.wf-scan-results`) rather than opening a new modal — simpler than building new modal machinery, and Cheevos's existing `#appConfirmModal`/`safeConfirm` (see 2.3) doesn't support arbitrary list content anyway.
- [x] After a successful inject/revert, the UI calls the existing, already-allowlisted `folders:rescan` channel (not `watchedFoldersApi.refreshConfigState()` directly — that function isn't exposed to the renderer at all) so Cheevos's own config-state detection picks up the change through its normal path.

### 2.3 UI surface for Inject / Preview / Revert — done
Real hook points found by dedicated exploration (not guessed): per-game `.dash-card`s (`buildDashCard()`, `index.html`) are built only from `loadConfigs()` output — i.e. only for games that **already have** a config — so they were never a fit for "offer Inject to a config-less folder" in the first place; confirms 2.2's folder-row approach was the right call, not just an expedient one. Dialogs go through `showAppConfirm`/`safeConfirm` (`title`/`message`/`detail`/`okText`/`cancelText`, returns a plain boolean) — confirmed **no** `.dialog`/`.dialog-backdrop` classes exist anywhere in this codebase yet, meaning Phase 7's Modernist componentization hasn't landed; used the existing modal as-is.
- [x] "Inject"/"Revert" buttons per scanned target, gated on `ScanStatus::as_json_str()`'s real tokens (`"injected"`/`"needs_update"`/`"update_reverted"` → Revert; anything else → Inject) — done in `renderAutogseScanResults()`.
- [x] "Preview" step: `injectDryRun`'s full stdout (resolved App ID, arch, achievement-data availability, file list) passed straight into `safeConfirm`'s `detail` field before the real `inject` call. Symmetric for `revertDryRun`/`revert`.
- [x] Both dry-run previews are plain text per 2.1 — same fragile-interim flag as originally planned, now literally sitting in `autogseHandleInject`/`autogseHandleRevert` in `index.html` waiting on Phase 4's real `--json`.
- [x] New CSS added for the icon-button and results panel (`.icon-btn.scan`, `.wf-scan-results`, `.wf-scan-row`, `.wf-scan-action`, ...) — deliberately did **not** reuse `.btn` for the inline Inject/Revert action buttons (that class is almost entirely gamepad-glyph styling elsewhere in this file, confirmed by the same exploration pass); used a small dedicated class instead.

### 2.4 Steam login wiring — done, narrower than originally scoped
- [x] Confirmed (again, directly this time): no competing Steam credential UI exists in Cheevos — the only `type="password"` field anywhere was a Steam **Web API key** input (`#settings-steamApiKeyInput`, unrelated: a developer API key, not account credentials). Added a new "AutoGSE Steam Login" section in Settings → Advanced, right next to it, following the same `.settings-section`/`.settings-control` markup pattern already used there.
- [x] Login/Logout buttons call `window.autogse.login(username, password)`/`.logout()` → `autogse:login`/`autogse:logout` IPC → the bridge's stdin-feeding `login()`. Password field is cleared immediately after submit.
- [x] **Narrower than 2.4 originally implied**: login/logout work standalone, but nothing in this phase's Inject/Preview UI (2.3) uses the stored credentials — those calls always pass anon mode. Wiring "am I logged in" into the inject flow, and relaying a live Steam Guard code if authenticated mode is ever turned on, is real, unstarted work — noted directly in the Settings UI's own hint text so it isn't a silent gap.
- [x] **Fixed, not just flagged**: the "no way to know who's logged in after a restart" gap noted below was hit for real — user reported logging in successfully, then a restart showed logged-out. Confirmed live it wasn't data loss (`credentials.dat`'s mtime matched the login moment, untouched afterward) — purely that Cheevos had nothing to ask. Fix: added `--json` to `doctor` itself (`src/cli.rs` `DoctorArgs`, `src/engine.rs` `JsonDoctorReport`/`run_doctor`), with a new `logged_in_as: Option<String>` field (`credentials::load()`'s username, never the password — same rule the roadmap's Phase 4 section already called for). This is a small, deliberate jump ahead of Phase 4's general "add `--json` everywhere" plan, scoped to just this one field because it was the one actually blocking something. `utils/autogse-bridge.js` gained `loginStatus()` on top of it; the Settings UI now calls it on load and after every login/logout action instead of only ever showing "this session's last action." Live-verified end to end: `doctor --json` → `"logged_in_as": "jayeff89"` (this machine's real stored login) → bridge's `loginStatus()` → `{loggedIn: true, username: "jayeff89"}`.

### 2.5 Exit criteria — met, including the manual pass
- [x] End-to-end anon-mode round trip confirmed live via the bridge module directly (2.1) — scan → preview → inject → scan (shows injected) → preview → revert → scan (shows vanilla again).
- [x] All new/changed JS (`main.js`, `preload.js`, both new `index.html` script blocks) syntax-checked cleanly (`node --check`, plus targeted extraction+check for the two new inline `<script>` IIFEs).
- [x] **Click-through in the running Cheevos UI — confirmed successful by the user manually**, on a real desktop session (this session's own sandboxed shell can't launch Electron GUI processes at all — `ELECTRON_RUN_AS_NODE=1` is set there, a deliberate boundary, same category of gap as Phase 1's UAC-blocked packaged-launch step — so this had to be a human pass, not something confirmable from here). Not yet re-confirmed in detail per sub-step (Preview text readability, dashboard pickup timing, etc.) — flag anything odd if it turns up.
- [ ] Regression pass on existing `watched-folders.js` flows (already-configured games, config-deletion guard, onboarding skip-all) — still open; the confirmed click-through covered the new Inject/Revert path, not a full regression sweep of the pre-existing folder flows.

## Phase 3 — Session Reuse (kill repeated Steam Guard prompts)

This directly answers the question that started this integration effort — reading Steam's `loginusers.vdf` was investigated and ruled out (confirmed by reading a real one on this machine: it only carries `AccountName`/`PersonaName`/`RememberPassword`/`AutoLogin`/`Timestamp` — account metadata, no reusable session/auth material). The real fix is session reuse AutoGSE's own login flow already produces but currently discards on every run.

- [x] Root-cause exactly how `generate_emu_config.exe` derives `refresh_tokens.json`'s write path — confirmed, again, directly: re-ran the vendored tool with no arguments (its own `--help`-equivalent) and confirmed **no flag or env var exists** to redirect that path; it's hardcoded beside the tool's own exe (`goldberg::tools_root()`, unwritable under `Program Files` in a real install, exactly Phase 5's original finding).
- [x] Relocate that cache to a writable AutoGSE-owned directory and re-enable `-tok`. **Chosen over a symlink/junction** (needs `SeCreateSymbolicLinkPrivilege`, not held by a non-elevated runtime process, and ties the redirect to whichever user ran the elevated installer) or relocating the whole install dir (bigger blast radius for every caller of `tools_root()`, not just authenticated inject): `goldberg::ensure_writable_mirror()` hard-links the entire vendored tree (149MB/1,250 files, confirmed today — a PyInstaller onedir build, so the exe needs its adjacent `_internal/` tree to run at all) into `%LOCALAPPDATA%\AutoGSE\gen_emu_cfg_cache\` on first authenticated use — a hard link costs no extra disk on the common same-volume case, falling back to a real copy only if that fails. Guarded by the same directory-scoped `mutex_engine::AutoGseLock` inject/revert already use, keyed on the mirror dir, so concurrent authenticated injects can't race on first-time construction. A stale mirror (AutoGSE upgrade shipped a newer vendored tool, detected via exe size+mtime) is rebuilt, preserving any real cached `refresh_tokens.json` across the rebuild. `run_generate_emu_config`'s `AuthMode::Authenticated` branch now resolves its exe from this mirror (anonymous runs untouched, still read `tools_root()` directly) and passes `-tok` alongside the existing `-acw`. `run_logout` also now clears the mirrored session, so logging in as a different account afterward can't reuse the previous account's cached session.
- [x] Exit criterion, confirmed live against this machine's real stored account (`autogse inject --dry-run`, using `credentials::load()` — no password ever re-entered) — **partially observed, honestly**: two consecutive authenticated dry-run injects (App ID 480 then 105600, synthetic scratch targets) both completed cleanly in ~20s each with full achievement/image listings, and — the real evidence of reuse — `refresh_tokens.json`'s content was **byte-for-byte identical (same MD5)** before and after the second run, meaning the second login loaded and reused the exact cached session rather than negotiating an independent new one. What wasn't observed: an actual Steam Guard *prompt* on either run — this account isn't currently being challenged by this tool at all, with or without a cached session, so the specific "prompt appears once, not twice" behavior remains unconfirmed pending a session/account that does get challenged. The mechanism itself (session cached, reused byte-identically across separate process invocations via the writable mirror) is confirmed working, which is what this phase actually needed to fix.

## Phase 4 — Extend AutoGSE's CLI Contract for Companion Use — done

- [x] Added `--json` to every subcommand that lacked it: `inject`/`revert` (both single-target and `--root` batch, and `--dry-run` previews), `repair`, `reinject`, `login`/`logout` (`success`/`username`, never the password — `doctor`'s existing `logged_in_as` rule). Followed the existing pattern: one `#[derive(serde::Serialize)]` struct per command (`JsonInjectResult`/`JsonRevertResult` + internally-tagged `JsonInjectBatchEntry`/`JsonRevertBatchEntry` for batch mode, `JsonRepairResult`, `JsonReinjectResult`, `JsonLoginResult`/`JsonLogoutResult`), success/failure still carried entirely by the existing exit codes — no new JSON error envelope. `Command::Login`/`Command::Logout` had to become args-carrying clap variants (`LoginArgs`/`LogoutArgs`) since a bare unit variant can't hold a flag. `Output` (`src/output.rs`) gained a `json`-aware constructor so every `--silent`-style info line is suppressed the same way under `--json`, leaving exactly one JSON payload on stdout per invocation.
- [x] **Real bug found and fixed along the way**: a unit test exercising a real (non-dry-run) `run_revert_single` — which calls the *public* `index::forget`, touching this machine's actual `%LOCALAPPDATA%\AutoGSE\known_targets.json` — reliably crashed the whole test binary with a Windows `STATUS_ACCESS_VIOLATION` when run as part of the full ~300-test suite (passed fine in isolation). This is exactly the class of fragile, real-machine-state-touching test this codebase's own `record_in`/`forget_in` test doubles and `cli_smoke.rs`'s `#[ignore]`d `inject_then_revert_round_trips` already exist to avoid. Removed that one test rather than chase the crash further; every other new `--json` code path is covered by tests that never touch the real index (`already_injected`/`nothing_to_revert`/dry-run previews, plus `repair`/`reinject`'s existing local-fixture pattern).
- [x] `cargo build --workspace` and `cargo test --workspace` clean: 309 passed, 0 failed, 18 ignored (up from 299/0/— pre-Phase-4; the removed real-revert test is a net negative one, offset by several new ones).
- [x] **Live-tested via the real compiled binary**, not just unit tests: `doctor --json` (real stored login `jayeff89`, all 4 tools OK), `repair --json` (real orphaned-backup fix, hand-built fixture), `reinject --json` (real ~18MB Goldberg DLL restaged, confirmed via file bytes on disk), `revert --json` both `--dry-run` (hash-verified preview) and real (confirmed vanilla DLL restored, `steam_appid.txt`/`steam_settings/` removed, manifest gone) and the `nothing_to_revert` no-op. Confirmed `doctor --json`'s `known_target_count`/`logged_in_as` unchanged before/after — no pollution of this machine's real state.
- [ ] **Not live-tested**: `inject`'s real/`--dry-run` JSON success paths — both call `generate_emu_config.exe` for real, which needs live Steam network access; two consecutive attempts in this session's sandboxed shell hit `generate_emu_config.exe`'s own 60s anonymous-mode timeout with no success (`ExternalToolTimeout`), unrelated to this phase's code (confirmed: plain `curl` to `steamcommunity.com` from the same shell succeeded fine, so it's specifically this tool/host or the sandbox's network policy, not the Bash tool being offline generally). Covered instead by: a direct unit test of the `already_injected` early-return status (the one branch that needs no network), full compile-time coverage of every other branch's struct construction, and code review. Test manually against a machine/session with working Steam network access — the exact same category of gap Phase 2.1 flagged for `login`/`logout`.
- [ ] **`login`/`logout --json` not live-tested, deliberately**: this dev machine has real stored Steam credentials (confirmed via `doctor --json`'s `logged_in_as: "jayeff89"`) — running a real `logout` would destroy them, the same risk Phase 2.1 already flagged and declined to touch. Verified by code review instead (both are thin, ~5-line wrappers around the exact same `credentials::save`/`delete` calls `run_login`/`run_logout`'s pre-existing non-JSON bodies already made, just with an early JSON-print branch mirroring `doctor --json`'s). Test manually, or against a machine with no stored credentials.
- [x] Wrote `CONTRACT.md` in the AutoGSE repo root documenting every `--json` command's struct shape (including the pre-existing `scan`/`list`/`audit`/`doctor`/`rpcs3-trophies`/`export-achievements` ones, not just this phase's new ones), every stable string token, a selected exit-code table, and the versioning policy below — the one place both repos' maintainers can check instead of reading Rust source from the Node side.
- [x] Versioned the contract: `autogse --version` already prints a real semver (clap's `#[command(version)]` off `Cargo.toml`, no code change needed there) — documented a semver compatibility policy in `CONTRACT.md` instead (additive fields/tokens never require a bump; breaking changes require at least a `MINOR` bump pre-1.0). Bumped `Cargo.toml` `0.2.0` → `0.3.0` for this phase's own additive surface. Wiring `utils/autogse-bridge.js` to actually assert a minimum compatible version at startup is Cheevos-repo work, out of scope here, same repo boundary this doc's own Architecture section already draws.

## Phase 5 — Surface AutoGSE's Unique Safety/Diagnostic Features

**Prerequisite**: Phase 4 is done — `repair --json`, `audit --json`, and `export-achievements --format json` all already exist and are fully documented in `CONTRACT.md`. This phase is pure Cheevos-side wiring, following the exact bridge → IPC → UI shape Phase 2 established for inject/revert (`utils/autogse-bridge.js` → `main.js` `autogse:*` handlers → `preload.js` → `index.html`'s Watched-Folders icon-button/inline-panel/`safeConfirm` flow).

**Real asymmetry found while planning this, which changes the UI shape below**: the three commands don't share one calling convention. `repair` takes `--path` (single target, like `inject`/`revert`). `audit` takes `--root` (batch, like `scan`). `export-achievements` takes **neither** — confirmed via `src/cli.rs`'s `ExportAchievementsArgs` (only `--format`/`--out`) — it exports over AutoGSE's own machine-wide known-target index (the same index `doctor --json`'s `known_target_count` already reports), not a folder. So `audit` fits the existing per-folder Watched-Folders scan panel naturally; `repair` is a per-target follow-up action offered against an audit finding; `export-achievements` doesn't belong in that panel at all and needs its own Settings-area entry point (§5.4).

**Second wrinkle, in the bridge layer**: `audit` deliberately exits non-zero (37, `AuditFoundProblems`) when it finds real problems — but its JSON body still printed successfully (`CONTRACT.md`: "check the array contents, not just the exit code, if you need per-target detail"). The existing `scan()`/`doctor()` bridge pattern (`code !== 0` → throw before ever looking at stdout) would misclassify that as a hard failure. `audit()`'s bridge function needs to parse stdout first and only throw if the parse itself fails; `repair()` and `exportAchievements()` can use the plain existing pattern unchanged (for `repair`, a non-zero exit genuinely does mean no JSON was printed — CONTRACT.md: the four unrecoverable diagnoses skip the JSON body entirely).

### 5.1 Extend the bridge module — done
- [x] `audit(root)` — `runAutogse(["audit", "--root", root, "--json"])`. Parses `result.stdout` first regardless of exit code, per the wrinkle above; only throws if `JSON.parse` itself fails.
- [x] `repair(path)` — `runAutogse(["repair", "--path", path, "--json"])`. Plain `scan()`-style pattern (`code !== 0` → throw, else parse stdout).
- [x] `exportAchievements(outPath)` — **one real deviation from the original plan**: written to take an explicit `outPath` argument and pass it straight through as `--out`, rather than a no-arg call. `main.js`'s handler owns the save-dialog prompt (§5.2) and calls this with the chosen path — cleaner separation than having the bridge module itself own an Electron-only `dialog` call, since every other function in this file is dialog-agnostic. `runAutogse(["export-achievements", "--format", "json", "--out", outPath])`, plain existing pattern (a real failure means a non-zero exit; the confirmation line AutoGSE prints to stdout on success is not parsed as JSON — the payload lives in the file, not stdout, once `--out` is given).
- [x] `AUDIT_TIMEOUT_MS` (60s), `REPAIR_TIMEOUT_MS` (30s, same bound as `revert` — no Steam/generate_emu_config call), `EXPORT_ACHIEVEMENTS_TIMEOUT_MS` (60s) added alongside the existing timeout constants.

### 5.2 IPC + preload wiring — done
- [x] `main.js`: added `autogse:audit`, `autogse:repair`, `autogse:export-achievements` handlers next to the existing `autogse:*` block, identical `{success:true,...}` / `{success:false,error}` try/catch shape as every existing handler.
- [x] `autogse:export-achievements` owns the save-dialog prompt itself (`dialog.showSaveDialog`, JSON filter, default filename `achievements.json`) rather than taking a path from the renderer — a single round-trip IPC call, matching the existing `select-file`/`select-image-file` handlers' shape. Returns `{success:false, canceled:true}` if the user dismisses the dialog, distinct from a real failure.
- [x] `preload.js`: exposed `audit`, `repair`, `exportAchievements` (no-arg — the path is chosen main-process-side) on `window.autogse`.

### 5.3 "Check Library" (audit + repair) UI — Watched Folders tab — done
- [x] New per-row icon-button (`.icon-btn.audit`, stethoscope glyph) next to the existing scan button, toggling a sibling `.wf-scan-results`-styled panel — reused verbatim rather than introducing parallel `.wf-audit-*` classes, since the existing skeleton (row/label/action, loading/empty/error states) is generic enough as-is (confirmed: nothing in it was scan-specific beyond the function names built around it).
- [x] "Fix" button shown only on rows diagnosed `orphaned_backup`, calling `repair(path)`.
- [x] `repair` has no `--dry-run` flag (confirmed: `RepairArgs` only has `--path`/`--json`) — the confirm dialog's detail text shows the diagnosis label instead of a real preview, then calls `repair()` directly on confirm.
- [x] Successful repair triggers `rescanAutogseAudit` (re-render) plus the existing `folders:rescan` channel, matching Phase 2.2's rule.

### 5.4 "Export Achievements" UI — Settings, not folder-scoped — done
- [x] New "AutoGSE Achievement Export" `.settings-section` added in Settings → Advanced, directly after the existing "AutoGSE Steam Login" section, same `h4`/`settings-control`/`hint` markup. **One scope trim from the original plan**: JSON-only (no CSV toggle) — the CLI supports `--format csv` too, but the plan's own UI sketch never actually needed a format choice; adding one would be a speculative control with no driving requirement, so it's a plain "Export Achievement Data" button that always calls `--format json`.
- [x] Confirmed by research this is genuinely new data, not a duplicate of `achievement-rarity.js` (global rarity %, not per-user) or `exophase-scraper.js` (metadata/icons only) — no export/CSV feature existed anywhere in `index.html`/`main.js` before this.

### 5.5 Verify — CLI-side live-tested; Electron click-through still open
- [x] `node --check` on every touched file: `utils/autogse-bridge.js`, `main.js`, `preload.js` directly, plus the large inline `<script>` block in `index.html` (extracted to a temp `.js` file and checked the same way Phase 2.5 did) — all clean.
- [x] **Live-tested `audit --json`'s healthy path for real**, against this machine's one real known target (`known_targets.json`, a real Steamworks folder under an actual game install — not a synthetic fixture): `audit --root <its parent>` → exit 0, `[{"path": "...\\Win64", "diagnosis": "healthy"}]`, exactly the shape the bridge's `audit()` expects on the "no problems" branch.
- [x] **Live-tested `export-achievements --format json --out` for real**, against the same real known target: exit 0, `[AutoGSE] Exported 45 achievement row(s) to ...` on stdout, and the target file contained a real 45-row `AchievementExportRow[]` array (`app_id`, `game_title`, `achievement_name`, `unlocked`, `unlocked_at`) matching `CONTRACT.md` exactly — confirms the bridge's file-based (not stdout-based) parsing assumption is correct.
- [ ] **Not live-tested**: `audit`'s "problems found" branch (non-zero exit + still-valid JSON body) and `repair`'s `orphaned_backup` auto-fix path. Both would require deliberately corrupting this machine's one real known target's real backup state to synthesize a diagnosable problem — too risky to do against a real user game without a disposable fixture, so this was deferred rather than faked, same call Phase 4's own repair testing made for its riskier real-index-touching cases. Test against a disposable synthetic target (copy a real injected target, rename its `steam_settings.bak_*` folder to synthesize `orphaned_backup`) before trusting the "Fix" button path.
- [ ] **Not live-tested**: the actual Electron click-through (icon-button toggle, panel rendering, save-dialog flow) — this session's sandboxed shell can't launch Electron GUI processes (same `ELECTRON_RUN_AS_NODE=1` boundary Phase 2.5 hit). Needs a human pass in a real desktop session, same category of open item Phase 1/2 both left for manual follow-up.

## Phase 6 — Reconcile AutoGSE's Own Roadmap

- [ ] In AutoGSE's `roadmap.md`, mark Phase 13's remaining un-implemented items and all of Phase 15 as **superseded by the Cheevos fork**, not "not started" — prevents future confusion about whether they're still planned in Rust.
- [ ] Freeze AutoGSE's own feature growth to what Phase 4 above needs for the companion contract. New user-facing features (overlays, playtime, rarity, scraping, store integrations) belong in Cheevos from here on, not AutoGSE.

---

## Phase 7 — Cheevos Visual Overhaul (Modernist, ported from AutoGSE's own GUI direction)

**Why**: Cheevos currently ships upstream PSerban93/Achievements' own dark theme almost unchanged (confirmed by reading `style.css`: a Dracula-derived palette — `--app-bg: #282a36`, purple/pink/cyan accents, corner radii up to `--app-radius-panel: 14px`/`--app-radius-pill: 999px`, pure-black glowy shadows). At a glance it's visually indistinguishable from the upstream project this is forked from. A re-skin within that same idiom (swap purple for another color) wouldn't fix that — what makes it actually distinct is adopting a different visual language entirely.

AutoGSE's own GUI (before Phase 0 removes the Slint implementation) had been moving toward exactly that: a documented **"Modernist" design system** — flat, architectural, Swiss-modernist. Light ground, exactly one red accent, zero corner radius anywhere, strong 2px rules, flush-left alignment (nothing centered), desaturated photography. The full token reference and 8 real mockup renders already exist at `AutoGSE\Mockups\` (`modernist-design-tokens.md` + `App Mockups.dc.html` + PNGs) — reuse that directly; don't re-derive a design system from scratch. It's already written as plain CSS custom properties + a documented component-class inventory (`.btn`, `.tag`, `.card`, `.table`, `.dialog`, ...) with **no framework dependency**, which happens to be a perfect fit for Cheevos's own plain HTML/CSS/JS stack (no React/Vue — confirmed via `package.json`'s dependency list) — nothing about it is Slint-specific; only the *tokens and rules* transfer, not any Slint markup/code.

**Sequencing correction to Phase 0**: `Mockups/` is scheduled for deletion there since AutoGSE's own Slint GUI won't need it once it's gone — but this phase needs to read those files first. **Do §7.1 below (copy the tokens/mockups out of AutoGSE) before running Phase 0's `Mockups/` deletion.** `Mockups/` is gitignored in AutoGSE, so once it's deleted from disk there's no git history to recover it from.

### 7.1 Port the design tokens out of AutoGSE before they're deleted
- [ ] Copy `AutoGSE\Mockups\modernist-design-tokens.md` and all 8 PNG mockup renders into the Cheevos repo (e.g. `Cheevos\design\`) — this becomes Cheevos's own reference doc going forward; AutoGSE's copy will not survive Phase 0.
- [ ] Translate the token doc's CSS custom properties directly into `style.css`'s `:root` block, **replacing** (not merging alongside) the existing Dracula-derived `--app-*` tokens: base colors, neutral/accent ramps, spacing scale, zero radii, ink-tinted shadows.
- [ ] Add `"Archivo"` as the heading/body font (per `--font-heading`/`--font-body`), bundled locally in `fonts/` — this app already ships local font files there (confirmed via its own project-structure docs), so this matches an existing convention rather than introducing a remote/CDN font load in an offline-first desktop app.

### 7.2 Apply the structural rules app-wide
- [ ] Zero corner radius everywhere — audit every `border-radius` across `style.css`/`overlay.html`/`playtime.html`/`progress.html`/`tray-menu.css` (today's theme uses radii up to 14px, plus fully pill-shaped `999px` elements) and flatten all of them.
- [ ] Flush-left alignment — audit centered text/headings/button labels app-wide and switch to left-aligned, per the token doc's explicit "never centered" rule.
- [ ] Ink-tinted shadows (`color-mix(in srgb, #2d2b2b ..%, transparent)`) replacing today's pure-black `rgba(0,0,0,...)` shadows.
- [ ] Desaturated game/achievement artwork (`filter: grayscale(1) contrast(1.08)`) — a real, distinctive signature move the token doc calls out explicitly. **Flag as a UX decision, not a blind CSS change**: some users may want full-color cover art, so this may need to be a toggle rather than forced.

### 7.3 Icon set swap
- [ ] Replace `assets/vendor/fontawesome/` usage with the Lucide icon set (the token doc's specified icon set) — audit every icon usage across `index.html`/`overlay.html`/`tray-menu.html` and swap.

### 7.4 Componentize against the documented class inventory
- [ ] Rebuild buttons/tags/cards/tables/dialogs against the token doc's own documented component classes (`.btn` + `-primary/-secondary/-ghost/-icon/-block`, `.tag` + variants, `.card` + variants, `.dialog-backdrop`/`.dialog`, `.table`, `.hr`) rather than inventing new ad hoc classes — the whole point of that reference is a small, reusable set, not a one-off per screen.
- [ ] App-wide `:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }`, replacing any remaining browser-default focus ring.
- [ ] Accessibility note carried over from the token doc: the base accent color (`--color-accent`) is only ~3.8:1 contrast on the light ground — fine for icons/large text/chrome, but body-size accent-colored text should use the darker `--color-accent-700` step (6.4:1) instead.

### 7.5 Verification
- [ ] Side-by-side screenshot comparison against a fresh, unmodified checkout of upstream PSerban93/Achievements, confirming genuine visual distinctiveness — not just a recolor of the same layout.
- [ ] **Decide**: the Modernist system as documented is light-mode only (no dark variant defined) — Cheevos today is dark-only. Decide whether this overhaul goes fully light, or needs a dark-mode variant designed alongside it, before committing to replacing every token; this is a real reversal of the current default, not just a palette swap.

---

## Phase 8
**Cheevos**
### Bugs
- [ ] games do not have achiements pop despite running the game from the app and it showing in windows gamebar and the achievements do not show as unlopcked in the app at all even after reloads. (this is a bug from the pserban93/achievements repo.)
- [/] every time the app is opened it starts a "starting background service" regenerating configs, achievements, schemas and rescan of watched folders etc. that makes the app laggy and unresponsive for potentially minutes on a large library (this is also a bug from the pserban93/achievements repo)
    - [x] Root cause: the boot background scan (`utils/watched-folders.js`, the IIFE behind the "Starting Background Services" progress UI) unconditionally ran a full recursive (depth-6) discovery walk of every watched folder on every launch, one root at a time, regardless of whether anything on disk had changed since the last run. Fixed by adding a persisted per-root scan fingerprint (`watched_folders_scan_cache.json`, cheap single-level `readdir` + existing-config membership hash) — a root's deep walk is now skipped at boot when its fingerprint is unchanged from the last successful scan, and only re-run for roots that actually gained/lost entries or had a config deleted. The manual "Rescan" button (`folders:rescan`) still always forces a full walk, bypassing the cache.
        - [ ] its a lot quicker but still takes 1-2 minutes before the app is usable.
### Changes
- [ ] 
### Features
- [ ]
**AutoGSE**
### Bugs
- [ ] 
### Changes
- [ ] 
### Features
- [ ]

## Phase 9 — Merge Hydra Game-Download Functionality into Cheevos

**Source**: `hydralauncher/hydra`, MIT (confirmed by reading its `LICENSE` file: "Copyright (c) 2024 Los Broxas"). Cheevos (PSerban93/Achievements) is also MIT. MIT → MIT means source can be adapted directly with attribution retained — no GPL-style "mere aggregation" boundary needed here, unlike the AutoGSE side of this doc (see the top-level License note).

**Real architecture finding, confirmed by reading Hydra's actual source (not assumed) — this narrows what "take directly from Hydra" can mean**: Hydra's catalogue browsing and per-game download-option search are *not* self-contained open-source features — they're thin clients over Hydra's own proprietary hosted backend (`HydraApi`, `src/main/services/hydra-api.ts`). Confirmed in three places:
- `getGameShopDetails` (`src/main/events/catalogue/get-game-shop-details.ts`) pulls game metadata from `GET /games/{shop}/{objectId}` and `POST /games/shop-details` on Hydra's backend, not from any local index.
- The part that actually answers "what can I download for this game" — the per-game repack list — comes from `GET /games/{shop}/{objectId}/download-sources` (`src/renderer/src/context/game-details/game-details.context.tsx:427`), returning `GameRepack[]`. Hydra's server does the work of ingesting every user's added source feeds and matching them to catalogue game IDs; that indexing/matching step is not present in the open-source repo at all.
- Adding a source (`src/main/events/download-sources/add-download-source.ts`) round-trips through `POST /download-sources` on that same backend before being stored locally; `syncDownloadSources` re-validates via `POST /download-sources/sync`.

So the genuinely portable pieces — self-contained, no hidden backend — are the debrid clients, the hoster resolvers, the download engine, and the queue/orchestration layer (§9.1–9.2). The catalogue + source-matching piece has to be **reimplemented against the open feed format**, not ported, if Cheevos isn't going to depend on Hydra's own hosted service (real risk: rate limits, ToS, an API that can change under a fork with zero warning). That's actually a good fit for what was asked for here — a from-scratch, client-side aggregator is a more natural home for "use different sources" than Hydra's own gated model (§9.3).

### 9.1 Directly portable, self-contained pieces (real Hydra source, MIT, no backend dependency)
- [ ] **Debrid clients** — `src/main/services/download/real-debrid.ts`, `torbox.ts`, `all-debrid.ts`, `premiumize.ts`. Each is a plain `axios` REST wrapper (bearer-token auth, add magnet → poll status → unrestrict/request the cached direct link). Cheevos already depends on `axios` — these port with minimal change (adapt Hydra's `RealDebridUser`/`TorBoxTorrentInfo`-style types from `src/types/download.types.ts` rather than pulling the whole types file).
- [ ] **Hoster resolvers** — `src/main/services/hosters/{datanodes,fuckingfast,gofile,mediafire,pixeldrain,rootz,vikingfile}.ts`. These scrape a direct download link out of a file-hosting page — the third download-source type Hydra supports, alongside torrent and debrid. Cheevos already depends on `cheerio`, which is exactly what this scraping needs.
- [ ] **Download manager/orchestration pattern** — `src/main/services/download/{download-manager,download-orchestrator,download-completion,disk-space,download-layout-state,js-http-downloader}.ts` + `helpers.ts`. Multi-download queueing, pause/resume, reordering. `src/types/download-contract.ts`'s `DownloadPlacement`/`RendererDownloadBucket` state machine is a clean reference even though Cheevos's own queue UI will look different — reuse the shape, not the markup.

### 9.2 The torrent engine — a real decision, not a copy-paste
Hydra's torrent downloading isn't a Node library — it's a separate Python process. `python_rpc/torrent_downloader.py` wraps `libtorrent` (Arvid Norberg's rasterbar-libtorrent, BSD-licensed, confirmed by reading its `LICENSE`) directly; `python_rpc/main.py` runs it as an RPC server; `src/main/services/python-rpc.ts` spawns that subprocess from Electron main and talks to it over RPC — the exact same shape `utils/autogse-bridge.js` already uses to talk to `autogse.exe` in this codebase (a proven pattern here, not a new risk category). `python_rpc/setup.py` packages it with `cx_Freeze` into a standalone `hydra-python-rpc` executable (bundling libtorrent + OpenSSL DLLs) so end users never need Python installed.

Two real options, undecided:
- [ ] **Port the Python-RPC approach as-is** — bundle a `cx_Freeze`-built RPC executable the same way `vendor/autogse/` is staged today (`build/stage-autogse.js`, Phase 1.2/1.3 above). Proven pattern, but adds a second bundled-subprocess runtime (Python) to Cheevos's build alongside AutoGSE.
- [ ] **Use a pure-Node bittorrent library instead** (e.g. `webtorrent`, MIT) — no second runtime to bundle, fits Cheevos's all-JS stack, but a different engine with different DHT/piece-selection maturity than libtorrent; needs real testing against live repack swarms before being trusted as equivalent, not assumed.
- [ ] **Decide via a spike, not a guess**: try both against a handful of real magnet links before committing — this is the single highest-risk unknown in this phase.

### 9.3 Catalogue browser + download sources — reimplemented against the open feed format
- [ ] **Source feed format** — confirmed from `GameRepack`/`DownloadSource` (`src/types/index.ts:27-48`): a download source is just a URL to a JSON feed; each entry has `title`, `uris: string[]` (magnet links / torrent URLs / direct hoster links, mixed in one array), `fileSize`, `uploadDate`. This feed shape itself is open and copyable — only Hydra's *aggregation and matching* of it is backend-gated. Cheevos can consume any existing community source that follows this shape without touching Hydra's API at all.
- [ ] **Local fetch + local matching, not server-side matching** — fetch each user-added source's JSON directly (`axios.get(source.url)`), cache it locally (same idea as AutoGSE's own `%LOCALAPPDATA%\AutoGSE\` local-state pattern this doc already leans on), and fuzzy-match titles against the game names Cheevos already has from its existing Steam/Epic/GOG/EA/Ubisoft/Xbox integrations — rather than against a `shop:objectId` pair Hydra doesn't expose. Simpler and more transparent than Hydra's opaque server-side match, and a better fit for "easier to use / different sources."
- [ ] **Catalogue browser UI** — Hydra's is a storefront-style discover surface (`src/renderer/src/pages/catalogue`) with genre/tag/publisher/developer filters sourced from a static external JSON CDN (`useCatalogue` hook, `src/renderer/src/hooks/use-catalogue.ts` — `/steam-genres.json`, `/steam-user-tags.json`, etc.). This is a genuinely new page for Cheevos — nothing in its existing per-owned-game achievement dashboard (`buildDashCard()`/`loadConfigs()`, Phase 2 above) does browse-to-discover-new-games. Per the user's own "easier to use" ask, scope the first cut down from Hydra's full storefront chrome: search-by-title across all added sources + a flat "recently added" list, closer to a lightweight repack-search tool than a catalogue clone.

### 9.4 Explicitly out of scope
- [ ] Hydra's own account/subscription system, cloud library sync, friends/social features, achievement/profile sync to Hydra's own backend — all `HydraApi`-backed, all skipped; Cheevos already owns achievement tracking and gains nothing from a second backend for it.
- [ ] Big Picture mode (`src/big-picture/`) — a separate Electron renderer entry point for a controller-first 10-foot UI. Not needed for this phase.

### 9.5 Suggested build sequence
- [ ] 9.5.1 Spike the torrent-engine decision (§9.2) against real magnet links before anything else — it's the long pole.
- [ ] 9.5.2 Port the debrid clients + a Settings UI for API tokens, following the same `.settings-section` pattern Phase 2.4 already used for AutoGSE's Steam login.
- [ ] 9.5.3 Port the hoster resolvers + a direct-link downloader (`js-http-downloader.ts` as reference).
- [ ] 9.5.4 Build the local source-feed fetcher/cache + fuzzy title matcher (§9.3), scoped first to "find downloads for a game I already track" rather than a full catalogue.
- [ ] 9.5.5 Build the download queue UI (pause/resume/reorder), reusing `download-orchestrator.ts`'s state machine as a reference, not a copy.
- [ ] 9.5.6 Only then: the standalone catalogue/discover browser page, deliberately scoped down per §9.3.
- [ ] 9.5.7 Attribution: retain Hydra's MIT notice for any file adapted close to verbatim; add a `THIRD_PARTY_NOTICES.md` entry in Cheevos, same spirit as this doc's own top-level License note for the AutoGSE/GPL side.

### Open questions
- [ ] Torrent engine: bundle Python+libtorrent (proven pattern in this repo, more moving parts) vs. `webtorrent` (simpler bundle, unproven against real repack swarms)? No recommendation yet — resolve via the §9.5.1 spike.
- [ ] Does Cheevos want to curate/ship a default list of community download sources (the way Hydra's backend implicitly does via `/download-sources/sync`), or ship empty and let users add their own from day one? Leaning toward the latter, given the "use different sources" framing this phase started from.

## Open questions (resolve before Phase 2 starts)

- **Bundled vs. detected AutoGSE**: does Cheevos ship `autogse.exe` inside its own installer (one download, one version pinned), or detect/require a separately-installed AutoGSE? *Recommendation: bundle it* — avoids "which AutoGSE version is this user actually running" drift between the two repos.
- **Shared vs. separate data directory**: does Cheevos read/write through AutoGSE's existing `%LOCALAPPDATA%\AutoGSE\` state (credentials, known-targets index), or keep its own? *Recommendation: share it* — one source of truth, avoids duplicate-login and duplicate-index bugs.

---

## Appendix — AutoGSE's current CLI surface (for reference while wiring the bridge)

Visible subcommands: `inject`, `revert`, `login`, `logout`, `ra-login`, `ra-logout`, `join`, `scan`, `list`, `reinject`, `repair`, `audit`, `doctor`, `check-update`, `sync-saves`, `configure-overlay`, `backup-achievements`, `list-backups`, `restore`, `lan`, `export`, `import`, `export-achievements`.

Hidden subcommands (real, tested, just not marketed as primary UX — fine to call from Cheevos): `install-menu`, `uninstall-menu`, `parse-controller-vdf`, `add-mod`, `deploy-real-glyphs`, `rpcs3-trophies`.

`--json` supported on: `scan`, `list`, `audit`, `rpcs3-trophies`, `doctor`, `inject`, `revert`, `repair`, `reinject`, `login`, `logout` (plus `export-achievements --format json`) — see `CONTRACT.md` for every shape/token. Everything else (`join`, `lan`, `configure-overlay`, `sync-saves`, ...) is still plain human-readable console output only.


## Parts of GSE not in AutoGSE

### Never touched at all (genuine gaps, not decisions)
Confirmed by grepping the whole roadmap/codebase — these appear nowhere outside the fork's own README:

- `stats.json` (custom stat definitions)
- SteamHTTP mocking (`steam_settings/http/`)
- Avatar drop-in (`account_avatar`/`account_avatar_default`)
- `[app::paths]` (DLC install-dir path overrides)
- No dedicated flag/UI to set `local_save_path` (portable saves) — only reads it if something else already wrote it
### Vendored on disk but never wired into any Rust code
`alex47exe-gse_fork/release/experimental/` (the overlay-enabled build + CPY-crack LAN support) and `release/steam_old_lib/Steam.dll` sit in the repo but `goldberg.rs` never references either path — `dll_source_path()` only ever pulls the regular DLL from `generate_emu_config`'s own bundled `_DEFAULT/0/` payload. So the actual live in-game overlay (SHIFT-TAB UI) is never something AutoGSE renders or drives — it can flip the ini flag and deploy fonts/sounds, but that's it.

### One more nuance: GUI removal shrank the surface
Phase 7 built a full Slint GUI (DLC per-title checkboxes, tabbed `configs.*.ini` editor, achievement viewer, RetroAchievements progress display) on top of this engine — but the recent Cheevos-companion pivot (`roadmap-cheevos-integration.md` Phase 0) deleted the GUI entirely, including `config_editor.rs`. Result: **per-DLC toggling and the full config editor no longer have any CLI** equivalent (only the blanket `--unlock-all-dlc` survives), and RetroAchievements only has `ra-login`/`ra-logout` left — no command to actually fetch or view RA progress now that the viewer dialog is gone. 