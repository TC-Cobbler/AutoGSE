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
- [x] No dedicated "who's logged in" query exists on AutoGSE's side (`doctor` only checks DPAPI store *reachability*, not a stored username) — the Settings status line can only reflect this session's last login/logout action, not state read back on app start. Real, current limitation, not a bug in this UI.

### 2.5 Exit criteria — met, including the manual pass
- [x] End-to-end anon-mode round trip confirmed live via the bridge module directly (2.1) — scan → preview → inject → scan (shows injected) → preview → revert → scan (shows vanilla again).
- [x] All new/changed JS (`main.js`, `preload.js`, both new `index.html` script blocks) syntax-checked cleanly (`node --check`, plus targeted extraction+check for the two new inline `<script>` IIFEs).
- [x] **Click-through in the running Cheevos UI — confirmed successful by the user manually**, on a real desktop session (this session's own sandboxed shell can't launch Electron GUI processes at all — `ELECTRON_RUN_AS_NODE=1` is set there, a deliberate boundary, same category of gap as Phase 1's UAC-blocked packaged-launch step — so this had to be a human pass, not something confirmable from here). Not yet re-confirmed in detail per sub-step (Preview text readability, dashboard pickup timing, etc.) — flag anything odd if it turns up.
- [ ] Regression pass on existing `watched-folders.js` flows (already-configured games, config-deletion guard, onboarding skip-all) — still open; the confirmed click-through covered the new Inject/Revert path, not a full regression sweep of the pre-existing folder flows.

## Phase 3 — Session Reuse (kill repeated Steam Guard prompts)

This directly answers the question that started this integration effort — reading Steam's `loginusers.vdf` was investigated and ruled out (confirmed by reading a real one on this machine: it only carries `AccountName`/`PersonaName`/`RememberPassword`/`AutoLogin`/`Timestamp` — account metadata, no reusable session/auth material). The real fix is session reuse AutoGSE's own login flow already produces but currently discards on every run.

- [ ] Root-cause exactly how `generate_emu_config.exe` derives `refresh_tokens.json`'s write path (Phase 5's original finding: written beside the tool's own `.exe`, unwritable under `Program Files` in a real install) — confirm by reading the vendored tool's own behavior/`--help`, not by guessing.
- [ ] Relocate that cache to a writable AutoGSE-owned directory (working-directory override, an env var the tool respects, or a junction from its expected path to `%LOCALAPPDATA%\AutoGSE\`) and re-enable the `-tok` flag AutoGSE currently deliberately never passes.
- [ ] Exit criterion, confirmed live: two consecutive authenticated `inject` runs against different App IDs, only the first ever prompts for Steam Guard.

## Phase 4 — Extend AutoGSE's CLI Contract for Companion Use

- [ ] Add `--json` to every subcommand Cheevos needs structured feedback from and doesn't have it yet: `inject`, `revert`, `repair`, `reinject`, `login`/`logout` (success/failure + username, never the password), `doctor`. Follow the existing pattern exactly — one `#[derive(serde::Serialize)]` struct per command (see `JsonTarget`/`AuditFinding`/`AchievementExportRow` in `src/engine.rs`), success/failure already carried by the process exit code.
- [ ] Write `CONTRACT.md` in the AutoGSE repo documenting every JSON struct shape and stable string token (e.g. `ScanStatus::as_json_str()`'s `"vanilla"`/`"injected"`/`"needs_update"`/`"update_reverted"`) — the one place both repos' maintainers (present or future) can check instead of reading Rust source from the Node side.
- [ ] Version the contract meaningfully in `autogse --version`; have `utils/autogse-bridge.js` assert a minimum compatible version at startup with a clear error, rather than fail deep inside a JSON-parse call.

## Phase 5 — Surface AutoGSE's Unique Safety/Diagnostic Features

- [ ] `repair`/`audit` (AutoGSE Phase 12 §12.2/§12.3): expose as a "Fix"/"Check Library" action in Cheevos's dashboard — real capability with no Cheevos equivalent at all.
- [ ] `export-achievements` (AutoGSE Phase 13): evaluate against Cheevos's own achievement views before wiring it in — if Cheevos's own dashboard/rarity/analytics already cover the same need better, skip this rather than duplicate it.

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
- [ ] every time the app is opened it starts a "starting background service" regenerating configs, achievements, schemas and rescan of watched folders etc. that makes the app laggy and unresponsive for potentially minutes on a large library (this is also a bug from the pserban93/achievements repo)
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

## Open questions (resolve before Phase 2 starts)

- **Bundled vs. detected AutoGSE**: does Cheevos ship `autogse.exe` inside its own installer (one download, one version pinned), or detect/require a separately-installed AutoGSE? *Recommendation: bundle it* — avoids "which AutoGSE version is this user actually running" drift between the two repos.
- **Shared vs. separate data directory**: does Cheevos read/write through AutoGSE's existing `%LOCALAPPDATA%\AutoGSE\` state (credentials, known-targets index), or keep its own? *Recommendation: share it* — one source of truth, avoids duplicate-login and duplicate-index bugs.

---

## Appendix — AutoGSE's current CLI surface (for reference while wiring the bridge)

Visible subcommands: `inject`, `revert`, `login`, `logout`, `ra-login`, `ra-logout`, `join`, `scan`, `list`, `reinject`, `repair`, `audit`, `doctor`, `check-update`, `sync-saves`, `configure-overlay`, `backup-achievements`, `list-backups`, `restore`, `lan`, `export`, `import`, `export-achievements`.

Hidden subcommands (real, tested, just not marketed as primary UX — fine to call from Cheevos): `install-menu`, `uninstall-menu`, `parse-controller-vdf`, `add-mod`, `deploy-real-glyphs`, `rpcs3-trophies`.

`--json` already supported on: `scan`, `list`, `audit`, `rpcs3-trophies` (plus `export-achievements --format json`). Everything else is plain human-readable console output today — see Phase 4.
