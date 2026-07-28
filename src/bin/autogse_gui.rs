// Phase 7 §7.1/§7.2/§7.6's first real Slint binary: a scan-only dashboard
// (§7.2) plus a standalone "Resolve App ID" action (§7.6) that exercises the
// real Step 1-5 App ID cascade, including the visual disambiguation dialog
// when Step 4's automatic match isn't confident. Both prove the Phase 7 §7.0
// background-thread bridge (`std::thread::spawn` + `slint::invoke_from_event_loop`,
// not an async runtime). Inject/revert-from-GUI and every other §7.3-§7.5
// feature are deliberately not built here yet (see roadmap.md Phase 7).

slint::include_modules!();

#[path = "autogse_gui/achievements_view.rs"]
mod achievements_view;
#[path = "autogse_gui/config_editor_view.rs"]
mod config_editor_view;
#[path = "autogse_gui/doctor_view.rs"]
mod doctor_view;
#[path = "autogse_gui/drop_confirm_view.rs"]
mod drop_confirm_view;
#[path = "autogse_gui/gui_interaction.rs"]
mod gui_interaction;
#[path = "autogse_gui/login_view.rs"]
mod login_view;
#[path = "autogse_gui/native_drop_target.rs"]
mod native_drop_target;
#[path = "autogse_gui/multiplayer_view.rs"]
mod multiplayer_view;
#[path = "autogse_gui/overlay_settings_view.rs"]
mod overlay_settings_view;
#[path = "autogse_gui/save_sync_view.rs"]
mod save_sync_view;

use std::path::{Path, PathBuf};

use raw_window_handle::HasWindowHandle;
use windows::Win32::Foundation::HWND;

use autogse::appid::{self, AppIdContext};
use autogse::cli::JoinArgs;
use autogse::discovery;
use autogse::engine::{self, DashboardRow, ScanStatus};
use autogse::error::AutoGseError;
use autogse::header_cache;
use autogse::update_check;
use slint::Model;

use gui_interaction::GuiInteraction;
use native_drop_target::DropTargetGuard;

/// What the background thread computes, before any Slint type touches it —
/// deliberately holds a cached image *path*, not a `slint::Image`, so
/// nothing here needs `slint::Image` to be `Send`; the actual image decode
/// happens back on the UI thread inside `invoke_from_event_loop` instead.
struct RowData {
    path: String,
    status: &'static str,
    title: String,
    header_image_path: Option<PathBuf>,
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    let weak = ui.as_weak();
    ui.on_scan_requested(move |root| {
        let root = PathBuf::from(root.as_str());
        let weak_for_thread = weak.clone();

        if let Some(ui) = weak.upgrade() {
            ui.set_scanning(true);
            ui.set_status_message("Scanning...".into());
        }

        // Discovery, manifest classification, and the CDN header-art fetch
        // (a real network call, best-effort) all happen off the UI thread so
        // a large library root — or a slow/unreachable CDN — can't freeze
        // the window; `invoke_from_event_loop` hands the result back to the
        // thread that owns the Slint event loop, since UI property setters
        // (and `slint::Image` construction) aren't safe to call from here.
        std::thread::spawn(move || {
            let result = scan(&root);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak_for_thread.upgrade() else { return };
                match result {
                    Ok(rows) => {
                        let count = rows.len();
                        let scan_rows: Vec<ScanRow> = rows
                            .into_iter()
                            .map(|r| ScanRow {
                                path: r.path.into(),
                                status: r.status.into(),
                                title: r.title.into(),
                                header_image: r
                                    .header_image_path
                                    .and_then(|p| slint::Image::load_from_path(&p).ok())
                                    .unwrap_or_default(),
                            })
                            .collect();
                        ui.set_all_rows(slint::ModelRc::new(slint::VecModel::from(scan_rows.clone())));
                        ui.set_rows(slint::ModelRc::new(slint::VecModel::from(filter_rows(&scan_rows, ui.get_filter_text().as_str()))));
                        ui.set_status_message(format!("{count} target(s) found.").into());
                    }
                    Err(e) => {
                        ui.set_all_rows(slint::ModelRc::new(slint::VecModel::from(Vec::<ScanRow>::new())));
                        ui.set_rows(slint::ModelRc::new(slint::VecModel::from(Vec::<ScanRow>::new())));
                        ui.set_status_message(format!("Error: {e}").into());
                    }
                }
                ui.set_scanning(false);
            });
        });
    });

    let weak_filter = ui.as_weak();
    ui.on_filter_changed(move |text| {
        let Some(ui) = weak_filter.upgrade() else { return };
        // Fires directly on the UI thread (a `.slint` callback invocation,
        // not a background-thread hop), so `ui.get_all_rows()` can be read
        // synchronously — no `Weak`/`invoke_from_event_loop` dance needed
        // for this one, unlike the scan-completion handler above.
        let all: Vec<ScanRow> = ui.get_all_rows().iter().collect();
        ui.set_rows(slint::ModelRc::new(slint::VecModel::from(filter_rows(&all, text.as_str()))));
    });

    let weak_join = ui.as_weak();
    ui.on_join_lobby_requested(move |path| {
        trigger_join_lobby(weak_join.clone(), PathBuf::from(path.as_str()));
    });

    let weak_updates = ui.as_weak();
    ui.on_check_for_updates_requested(move || {
        let weak_for_thread = weak_updates.clone();
        if let Some(ui) = weak_updates.upgrade() {
            ui.set_update_status_message("Checking for updates...".into());
        }
        // A real (small) network call to GitHub's releases API — off the UI
        // thread for the same reason every other network call in this file
        // is, even though it's normally fast.
        std::thread::spawn(move || {
            let result = update_check::check_for_update();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak_for_thread.upgrade() else { return };
                let message = match result {
                    Ok(update_check::UpdateStatus::UpToDate) => format!("You're on the latest version ({}).", env!("CARGO_PKG_VERSION")),
                    Ok(update_check::UpdateStatus::UpdateAvailable { latest_version }) => {
                        format!("A newer version is available: {latest_version} (you have {}).", env!("CARGO_PKG_VERSION"))
                    }
                    Err(e) => format!("Error checking for updates: {e}"),
                };
                ui.set_update_status_message(message.into());
            });
        });
    });

    let weak_appid = ui.as_weak();
    ui.on_resolve_appid_requested(move |path| {
        trigger_resolve_appid(weak_appid.clone(), PathBuf::from(path.as_str()));
    });

    // Phase 7 §7.4's config editor: all local-disk INI work, no network call
    // and no `Interaction` prompt in this flow, so (unlike scan/App-ID
    // resolution) this runs synchronously on the UI thread rather than
    // spawning a background thread — see `config_editor_view.rs`'s own doc
    // comment. `config_editor_holder` keeps the dialog's one strong handle
    // alive for its whole open lifetime; opening a different target replaces
    // it, dropping (and closing) whatever was open before.
    let config_editor_holder: config_editor_view::DialogHolder = std::rc::Rc::new(std::cell::RefCell::new(None));
    let weak_config_editor = ui.as_weak();
    ui.on_open_config_editor_requested(move |path| {
        let Some(ui) = weak_config_editor.upgrade() else { return };
        let tod = PathBuf::from(path.as_str());
        match config_editor_view::open(&config_editor_holder, &tod) {
            Ok(()) => ui.set_config_editor_status_message(format!("Opened config editor for {}.", tod.display()).into()),
            Err(e) => ui.set_config_editor_status_message(format!("Error: {e}").into()),
        }
    });

    // Phase 7 §7.5's achievement viewer: local-disk read (+ a live filesystem
    // watch, not a network call) with no `Interaction` prompt in this flow,
    // same reasoning as the config editor for running directly on the UI
    // thread rather than spawning a background thread.
    let achievements_holder: achievements_view::DialogHolder = achievements_view::new_holder();
    let weak_achievements = ui.as_weak();
    ui.on_open_achievements_requested(move |path| {
        let Some(ui) = weak_achievements.upgrade() else { return };
        let tod = PathBuf::from(path.as_str());
        match achievements_view::open(&achievements_holder, &tod) {
            Ok(()) => ui.set_achievements_status_message(format!("Opened achievement viewer for {}.", tod.display()).into()),
            Err(e) => ui.set_achievements_status_message(format!("Error: {e}").into()),
        }
    });

    // Phase 7 §7.8.6's Overlay Settings dialog: a global (not per-target)
    // preference editor, same local-disk-only/no-thread-hop reasoning as
    // the config editor and achievement viewer.
    let overlay_settings_holder: overlay_settings_view::DialogHolder = overlay_settings_view::new_holder();
    ui.on_open_overlay_settings_requested(move || {
        let _ = overlay_settings_view::open(&overlay_settings_holder);
    });

    // Phase 7 §7.8.6's Doctor panel: same global, no-thread-hop reasoning.
    let doctor_holder: doctor_view::DialogHolder = doctor_view::new_holder();
    ui.on_open_doctor_requested(move || {
        let _ = doctor_view::open(&doctor_holder);
    });

    // Phase 7 §7.8.3's Steam Login dialog: same global, no-thread-hop
    // reasoning (plain DPAPI save/delete, no network call at this step).
    let login_holder: login_view::DialogHolder = login_view::new_holder();
    ui.on_open_steam_login_requested(move || {
        let _ = login_view::open(&login_holder);
    });

    // Phase 7 §7.8.8's Multiplayer & Virtual LAN dialog: same global,
    // no-thread-hop-to-open reasoning (the actual launch still spawns a
    // background thread inside `multiplayer_view::open`'s callback).
    let multiplayer_holder: multiplayer_view::DialogHolder = multiplayer_view::new_holder();
    ui.on_open_multiplayer_requested(move || {
        let _ = multiplayer_view::open(&multiplayer_holder);
    });

    // Phase 8's Save & Cloud Sync Manager: per-target, opened from a
    // per-card icon — real disk I/O and a possible first-time network fetch
    // (the Ludusavi manifest), so every operation inside runs on a
    // background thread (see `save_sync_view.rs`'s own doc comment).
    let save_sync_holder: save_sync_view::DialogHolder = save_sync_view::new_holder();
    ui.on_open_save_sync_requested(move |path| {
        let _ = save_sync_view::open(&save_sync_holder, &PathBuf::from(path.as_str()));
    });

    // Phase 7 §7.8.2's drop-confirm dialog — created here (not inside the
    // deferred drop-target setup below) so it's available to clone into
    // that closure once it's constructed further down.
    let drop_confirm_holder: drop_confirm_view::DialogHolder = drop_confirm_view::new_holder();

    ui.show()?;

    // Phase 7 §7.3's real drag-and-drop: registered on the window's actual
    // native HWND (obtained via `raw-window-handle`, which Slint exposes
    // through `Window::window_handle()`) rather than through Slint's own
    // drag-and-drop elements — see `native_drop_target.rs`'s module doc for
    // why those can't carry files at all in this Slint version.
    //
    // Registration is deferred and polled for, rather than done right here
    // after `show()` — confirmed live this needs more than "wait for one
    // event loop iteration" as the docs suggest: winit (Slint's backend
    // here) creates its *real* native window lazily, and
    // `WinitWindowAdapter::window_handle_06_rc()` returns `Unavailable`
    // (surfacing as `HandleError::NotSupported` by the time it reaches
    // here, since `Window::window_handle()` falls back to an unimplemented
    // default) until that real window actually exists — which a single
    // zero-duration `Timer::single_shot` fired too early to see. A short
    // repeating timer that stops itself once `native_hwnd` succeeds (or
    // after a two-second safety-valve cap, in case some backend genuinely
    // never supports this) is the robust version of the same wait.
    //
    // The guard and timer both live in `Rc`s (not plain locals) because
    // they're *created*/started inside this deferred setup but must stay
    // alive for the whole event loop below, not just this closure's first
    // call — dropping the guard unregisters the target and uninitializes
    // OLE; dropping the timer would stop it firing again.
    let drop_guard: std::rc::Rc<std::cell::RefCell<Option<DropTargetGuard>>> = std::rc::Rc::new(std::cell::RefCell::new(None));
    let poll_timer = std::rc::Rc::new(slint::Timer::default());
    {
        let drop_guard = drop_guard.clone();
        let poll_timer_handle = poll_timer.clone();
        let weak_for_setup = ui.as_weak();
        let drop_confirm_holder = drop_confirm_holder.clone();
        let mut attempts: u32 = 0;
        const MAX_ATTEMPTS: u32 = 40; // 40 * 50ms = 2s safety valve

        poll_timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(50), move || {
            attempts += 1;
            let Some(ui) = weak_for_setup.upgrade() else {
                poll_timer_handle.stop();
                return;
            };

            let Some(hwnd) = native_hwnd(&ui) else {
                if attempts >= MAX_ATTEMPTS {
                    poll_timer_handle.stop();
                    eprintln!("[AutoGSE] drag-and-drop unavailable: native window handle never became available after {attempts} attempts");
                    ui.set_appid_result_message("Drag-and-drop unavailable: could not obtain the native window handle.".into());
                }
                return;
            };

            poll_timer_handle.stop();
            let weak_for_drop = ui.as_weak();
            let drop_confirm_holder = drop_confirm_holder.clone();
            match DropTargetGuard::register(hwnd, move |paths| {
                // The OS may report several dropped items at once (a
                // multi-select drag); only the first is meaningful here —
                // "resolve an App ID for one target," same as the text
                // field this reuses.
                let Some(path) = paths.into_iter().next() else { return };
                if let Some(ui) = weak_for_drop.upgrade() {
                    ui.set_appid_target_path(path.display().to_string().into());
                }
                // Phase 7 §7.8.2: a real drop now opens the dedicated
                // confirm dialog (matching the mockup's dedicated screen)
                // instead of only filling the dashboard's text field —
                // `DropTargetGuard::register`'s callback already fires on
                // the UI thread (real Windows OLE drag-drop callbacks run on
                // whatever thread called `RegisterDragDrop`, which is this
                // one), so opening the dialog here directly is safe; the
                // actual App ID cascade still moves to a background thread
                // inside `open_and_resolve`.
                drop_confirm_view::open_and_resolve(&drop_confirm_holder, path);
            }) {
                Ok(guard) => {
                    eprintln!("[AutoGSE] native drag-and-drop target registered on HWND {:?} after {attempts} attempt(s)", hwnd.0);
                    *drop_guard.borrow_mut() = Some(guard);
                }
                Err(e) => {
                    eprintln!("[AutoGSE] drag-and-drop unavailable: RegisterDragDrop failed: {e}");
                    ui.set_appid_result_message(format!("Drag-and-drop unavailable: {e}").into());
                }
            }
        });
    }

    // `ComponentHandle::run()`'s own documented shape (`show` + this + `hide`)
    // — split apart so `show()` can happen before the deferred registration
    // above is scheduled, and `drop_guard` can outlive both.
    slint::run_event_loop()?;
    ui.hide()?;
    drop(drop_guard);
    Ok(())
}

/// This (background-thread cascade + UI-thread result handoff) is shared by
/// both the "Resolve App ID" button and a real file drop — a drop just fills
/// in the same path field and triggers the identical flow.
fn trigger_resolve_appid(weak: slint::Weak<MainWindow>, path: PathBuf) {
    if let Some(ui) = weak.upgrade() {
        ui.set_resolving_appid(true);
        ui.set_appid_result_message("Resolving...".into());
    }

    // Runs the *real* Step 1-5 cascade off the UI thread — Step 4 is a live
    // network call, and Step 5 (when reached) blocks this thread inside
    // `GuiInteraction::disambiguate_app_id` until the user answers the
    // dialog it opens on the UI thread. Must never run this on the UI thread
    // itself (see that function's own doc comment) — true whether triggered
    // from the button (already off-thread) or the drop target's callback
    // (which fires directly on the UI thread, so this spawn is what moves
    // the actual resolution work off of it).
    let weak_for_thread = weak.clone();
    std::thread::spawn(move || {
        let result = resolve_appid(&path);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak_for_thread.upgrade() else { return };
            match result {
                Ok((app_id, title)) => {
                    let message = match title {
                        Some(t) => format!("Resolved App ID {app_id} ({t})."),
                        None => format!("Resolved App ID {app_id}."),
                    };
                    ui.set_appid_result_message(message.into());
                }
                Err(e) => ui.set_appid_result_message(format!("Error: {e}").into()),
            }
            ui.set_resolving_appid(false);
        });
    });
}

/// Phase 7 §7.8.1's filter field: case-insensitive substring match against
/// title, path, or status — pure and client-side, no `engine` change needed
/// since `list_dashboard_targets` already returns everything unfiltered.
fn filter_rows(rows: &[ScanRow], filter_text: &str) -> Vec<ScanRow> {
    let needle = filter_text.trim().to_lowercase();
    if needle.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|r| r.title.to_lowercase().contains(&needle) || r.path.to_lowercase().contains(&needle) || r.status.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// Phase 7 §7.8.1's per-card "Join Lobby" icon: wraps the existing
/// `engine::run_join`/`goldberg::run_lobby_connect` (§6.7), CLI-only until
/// now. `run_lobby_connect` hands off to the vendored tool's own fully
/// interactive, no-timeout menu (inherited stdio, a real console window),
/// so this just launches it and reports success/failure once that process
/// exits — there's no progress to report mid-flight, unlike App ID
/// resolution's multi-step cascade.
fn trigger_join_lobby(weak: slint::Weak<MainWindow>, path: PathBuf) {
    if let Some(ui) = weak.upgrade() {
        ui.set_join_lobby_status_message(format!("Launching lobby_connect for {}...", path.display()).into());
    }
    std::thread::spawn(move || {
        let interaction = GuiInteraction;
        let result = engine::run_join(&JoinArgs { path: path.clone() }, &interaction);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let message = match result {
                Ok(()) => format!("lobby_connect closed for {}.", path.display()),
                Err(e) => format!("Error: {e}"),
            };
            ui.set_join_lobby_status_message(message.into());
        });
    });
}

fn native_hwnd(ui: &MainWindow) -> Option<HWND> {
    let handle = ui.window().window_handle();
    let raw = match handle.window_handle() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[AutoGSE] native_hwnd: HasWindowHandle::window_handle() failed: {e:?}");
            return None;
        }
    };
    match raw.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
        other => {
            eprintln!("[AutoGSE] native_hwnd: unexpected raw handle variant: {other:?}");
            None
        }
    }
}

/// Runs the same `discovery::resolve_target` + `appid::resolve_app_id`
/// cascade `engine::run_inject_single` uses — no separate/mocked App ID
/// logic for the GUI. `discovery::resolve_target` is passed `None` (not
/// `Some(&GuiInteraction)`): its own non-standard-DLL-name prompt
/// (`Interaction::pick_dll`) isn't built yet (§7.6 is specifically about App
/// ID disambiguation, not that), and `GuiInteraction::pick_dll` stubs to
/// `None` anyway, so passing `None` directly here is the honest signal
/// rather than routing through a stub that can't actually resolve anything.
fn resolve_appid(path: &Path) -> Result<(u64, Option<String>), AutoGseError> {
    let resolution = discovery::resolve_target(path, None)?;
    let interaction = GuiInteraction;
    let ctx = AppIdContext { tod: &resolution.tod, exe_hint: path, override_appid: None, interaction: Some(&interaction) };
    let resolved = appid::resolve_app_id(&ctx)?;
    Ok((resolved.app_id, resolved.game_title))
}

/// Phase 7 §7.8.2's drop-confirm dialog needs more than the plain
/// `resolve_appid` tuple (arch, TOD) — same cascade, just also reading back
/// the PE bitness `run_inject_single` itself resolves before backing up a
/// DLL, so the "detected" card can show `x64`/`x86` like the mockup.
pub(crate) struct DetectedGame {
    pub tod_display: String,
    pub title: Option<String>,
    pub app_id: u64,
    pub arch: String,
}

pub(crate) fn resolve_appid_detailed(path: &Path) -> Result<DetectedGame, AutoGseError> {
    let resolution = discovery::resolve_target(path, None)?;
    let arch = autogse::pe::read_bitness(&resolution.dll_path)?;
    let interaction = GuiInteraction;
    let ctx = AppIdContext { tod: &resolution.tod, exe_hint: path, override_appid: None, interaction: Some(&interaction) };
    let resolved = appid::resolve_app_id(&ctx)?;
    Ok(DetectedGame { tod_display: resolution.tod.display().to_string(), title: resolved.game_title, app_id: resolved.app_id, arch: arch.to_string() })
}

/// Reuses `engine::list_dashboard_targets` (on-disk scan + known-injected
/// targets outside `root`, merged) and best-effort CDN header art per row
/// with a resolved App ID — no reimplementation of either.
fn scan(root: &Path) -> Result<Vec<RowData>, AutoGseError> {
    let targets = engine::list_dashboard_targets(root)?;
    Ok(targets.into_iter().map(row_data).collect())
}

fn row_data(target: DashboardRow) -> RowData {
    let status = match target.status {
        ScanStatus::Vanilla => "vanilla",
        ScanStatus::Injected => "injected",
        ScanStatus::NeedsUpdate => "needs update",
    };
    let title = target
        .game_title
        .clone()
        .unwrap_or_else(|| target.tod.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    // Best-effort: a network failure/timeout here just means this row shows
    // no art, never a fatal dashboard error (same convention as
    // `acw::deploy_schema`'s call site in `engine::run_inject_single`).
    let header_image_path = target.app_id.and_then(|id| header_cache::cached_header_path(id).ok());

    RowData { path: target.tod.display().to_string(), status, title, header_image_path }
}
