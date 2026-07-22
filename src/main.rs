mod acw;
mod appid;
mod appid_prompt;
mod backup;
mod cli;
mod credentials;
mod discovery;
mod elevate;
mod error;
mod goldberg;
mod index;
mod ini_patch;
mod log;
mod login_prompt;
mod manifest;
mod mods;
mod mutex_engine;
mod notify;
mod output;
mod pe;
mod preferences;
mod process_lock;
mod registry;
mod sanitize;
mod shortcut;
mod steam_api;
mod steamclient_mode;
mod update_check;
mod version_info;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;

use appid::AppIdContext;
use cli::{AddModArgs, Cli, Command, InjectMode, JoinArgs, ParseControllerVdfArgs, TargetArgs};
use error::AutoGseError;
use goldberg::AuthMode;
use login_prompt::DisclosureChoice;
use manifest::{BackedUpFile, GseManifest};
use mutex_engine::AutoGseLock;
use output::Output;

/// Named mutex wait timeout: long enough to let a concurrent inject/revert on
/// the same folder finish, short enough not to hang a user's click forever.
const LOCK_TIMEOUT_MS: u32 = 10_000;

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    // Best-effort (never fatal if it fails, e.g. LOCALAPPDATA unset): a
    // persistent record of what ran survives after the console/toast is
    // gone, directly addressing the Phases 3/5 debugging pain the roadmap
    // cites (§6.9) — every prior "exit code 1, cause unknown" incident only
    // had ephemeral output to go on.
    let _ = log::append(&format!("run: {}", raw_args.join(" ")));
    let cli = Cli::parse();
    let already_elevated = cli.elevated;

    match run(cli.command) {
        Ok(()) => {
            let _ = log::append("run: OK");
            ExitCode::SUCCESS
        }
        Err(err) => {
            if !already_elevated {
                if let AutoGseError::Io(io_err) = &err {
                    if elevate::is_permission_denied(io_err) {
                        let mut relaunch_args = raw_args;
                        relaunch_args.push("--elevated".to_string());
                        return match elevate::relaunch_elevated(&relaunch_args) {
                            Ok(code) => ExitCode::from(code),
                            Err(elev_err) => error::report_and_exit(elev_err.into()),
                        };
                    }
                }
            }
            error::report_and_exit(err.into())
        }
    }
}

fn run(command: Command) -> Result<(), AutoGseError> {
    match command {
        Command::Inject(args) => {
            let out = Output::new(args.silent);
            run_inject(&args, &out)
        }
        Command::Revert(args) => {
            let out = Output::new(args.silent);
            run_revert(&args, &out)
        }
        Command::InstallMenu => {
            registry::install_context_menu()?;
            // Required for toast notifications to display at all from this
            // unpackaged exe, not just cosmetic — see shortcut.rs.
            shortcut::install()?;
            println!("[AutoGSE] Explorer context menu entries installed.");
            Ok(())
        }
        Command::UninstallMenu => {
            registry::uninstall_context_menu()?;
            shortcut::uninstall()?;
            println!("[AutoGSE] Explorer context menu entries removed.");
            Ok(())
        }
        Command::Login => {
            let creds = login_prompt::capture_login_stdio()?;
            credentials::save(&creds)?;
            println!(
                "[AutoGSE] Logged in as {}. Future injections will include achievement data automatically.",
                creds.username
            );
            Ok(())
        }
        Command::Logout => {
            credentials::delete()?;
            println!("[AutoGSE] Logged out. Stored Steam credentials removed. Your anonymous preference, if any, is unchanged.");
            Ok(())
        }
        Command::ParseControllerVdf(args) => run_parse_controller_vdf(&args),
        Command::ConfigureOverlay(args) => run_configure_overlay(&args),
        Command::AddMod(args) => run_add_mod(&args),
        Command::Join(args) => run_join(&args),
        Command::Scan(args) => run_scan(&args),
        Command::List => run_list(),
        Command::Doctor => run_doctor(),
        Command::CheckUpdate => run_check_update(),
    }
}

fn run_doctor() -> Result<(), AutoGseError> {
    println!("=== AutoGSE Doctor ===");

    match goldberg::tools_root() {
        Ok(p) => println!("[OK]   generate_emu_config tools resolved: {}", p.display()),
        Err(e) => println!("[FAIL] generate_emu_config tools: {e}"),
    }
    match goldberg::parse_controller_vdf_root() {
        Ok(p) => println!("[OK]   parse_controller_vdf tools resolved: {}", p.display()),
        Err(e) => println!("[FAIL] parse_controller_vdf tools: {e}"),
    }
    match goldberg::lobby_connect_root() {
        Ok(p) => println!("[OK]   lobby_connect tools resolved: {}", p.display()),
        Err(e) => println!("[FAIL] lobby_connect tools: {e}"),
    }
    match goldberg::steamclient_experimental_root() {
        Ok(p) => println!("[OK]   steamclient_experimental tools resolved: {}", p.display()),
        Err(e) => println!("[FAIL] steamclient_experimental tools: {e}"),
    }
    match credentials::self_test() {
        Ok(()) => println!("[OK]   DPAPI credential store reachable"),
        Err(e) => println!("[FAIL] DPAPI credential store: {e}"),
    }

    match index::load_existing_injected() {
        Ok(targets) => println!("[OK]   {} known injected target(s) on this machine", targets.len()),
        Err(e) => println!("[FAIL] known-target index: {e}"),
    }

    match log::tail(20) {
        Ok(lines) if lines.is_empty() => println!("--- log: no entries yet ---"),
        Ok(lines) => {
            println!("--- log tail ({} line(s)) ---", lines.len());
            for line in lines {
                println!("{line}");
            }
        }
        Err(e) => println!("[FAIL] reading log: {e}"),
    }

    Ok(())
}

fn run_check_update() -> Result<(), AutoGseError> {
    match update_check::check_for_update()? {
        update_check::UpdateStatus::UpToDate => {
            println!("[AutoGSE] You're on the latest version ({}).", env!("CARGO_PKG_VERSION"));
        }
        update_check::UpdateStatus::UpdateAvailable { latest_version } => {
            println!(
                "[AutoGSE] A newer version is available: {latest_version} (you have {}). Visit the releases page to download it.",
                env!("CARGO_PKG_VERSION")
            );
        }
    }
    Ok(())
}

/// Status classification for one `scan --root` target (Phase 6 §6.8).
#[derive(Debug, PartialEq, Eq)]
enum ScanStatus {
    Vanilla,
    Injected,
    /// Manifest present but stale — either its schema version predates the
    /// running binary's, or a backed-up file's recorded SHA-256 no longer
    /// matches what's on disk (reusing the same hash-check
    /// `backup::restore_backup` already performs on revert, just without
    /// actually restoring).
    NeedsUpdate,
}

fn classify_target(tod: &Path) -> Result<ScanStatus, AutoGseError> {
    let Some(manifest) = manifest::load(tod)? else {
        return Ok(ScanStatus::Vanilla);
    };
    if manifest.version != manifest::MANIFEST_VERSION {
        return Ok(ScanStatus::NeedsUpdate);
    }
    for entry in &manifest.backed_up_files {
        let backup_path = tod.join(&entry.backup_path);
        if !backup_path.is_file() || backup::sha256_file(&backup_path)? != entry.sha256_hash {
            return Ok(ScanStatus::NeedsUpdate);
        }
    }
    Ok(ScanStatus::Injected)
}

fn run_scan(args: &cli::ScanArgs) -> Result<(), AutoGseError> {
    let targets = discovery::find_all_targets_under(&args.root)?;
    if targets.is_empty() {
        println!("[AutoGSE] No injectable targets found under {}.", args.root.display());
        return Ok(());
    }

    for target in &targets {
        let status = classify_target(&target.tod)?;
        let label = match status {
            ScanStatus::Vanilla => "vanilla",
            ScanStatus::Injected => "injected",
            ScanStatus::NeedsUpdate => "needs update",
        };
        println!("[{label}] {}", target.tod.display());
    }
    println!("[AutoGSE] {} target(s) found under {}.", targets.len(), args.root.display());
    Ok(())
}

fn run_list() -> Result<(), AutoGseError> {
    let targets = index::load_existing_injected()?;
    if targets.is_empty() {
        println!("[AutoGSE] No injected targets recorded on this machine.");
        return Ok(());
    }
    for tod in &targets {
        let mode = manifest::load(tod)?.map(|m| m.mode).unwrap_or_else(|| "regular".to_string());
        println!("[{mode}] {}", tod.display());
    }
    println!("[AutoGSE] {} injected target(s) known on this machine.", targets.len());
    Ok(())
}

// Deliberately does *not* acquire `AutoGseLock`: unlike `inject`/`revert`,
// this never mutates the target directory (lobby_connect only reads its own
// `steam_appid.txt`), and the tool's own interactive session can run for as
// long as the user is browsing lobbies — holding the mutex for that whole
// span would block a legitimate concurrent `revert` on the same target for
// no correctness reason.
fn run_join(args: &JoinArgs) -> Result<(), AutoGseError> {
    let resolution = discovery::resolve_target(&args.path, true)?;
    let arch = pe::read_bitness(&resolution.dll_path)?;

    println!("[AutoGSE] Launching lobby_connect for {} ({arch})...", resolution.tod.display());
    goldberg::run_lobby_connect(&resolution.tod, arch)
}

fn run_add_mod(args: &AddModArgs) -> Result<(), AutoGseError> {
    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(&args.path, false)?;
    let Some(mut manifest) = manifest::load(&resolution.tod)? else {
        return Err(AutoGseError::NotInjected(resolution.tod));
    };

    let request = mods::AddModRequest {
        id: args.id,
        title: args.title.clone(),
        description: args.description.clone(),
        primary_file: &args.file,
        preview_file: args.preview.as_deref(),
    };
    let written = mods::add_mod(&resolution.tod, &request)?;
    for path in written {
        if !manifest.injected_files.contains(&path) {
            manifest.injected_files.push(path);
        }
    }
    // mods.json itself is read-modify-written in place, not necessarily a
    // brand-new file — track it too so revert removes it even if this was
    // the first mod ever added to this target.
    let mods_json_rel = "steam_settings/mods.json".to_string();
    if !manifest.injected_files.contains(&mods_json_rel) {
        manifest.injected_files.push(mods_json_rel);
    }
    manifest::save(&resolution.tod, &manifest)?;

    println!("[AutoGSE] Added mod {} ({}) to {}.", args.id, args.title, resolution.tod.display());
    Ok(())
}

fn run_parse_controller_vdf(args: &ParseControllerVdfArgs) -> Result<(), AutoGseError> {
    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(&args.path, false)?;
    let Some(mut manifest) = manifest::load(&resolution.tod)? else {
        return Err(AutoGseError::NotInjected(resolution.tod));
    };

    let written = goldberg::run_parse_controller_vdf(&args.vdf, &resolution.tod)?;
    let added = written.len();
    for path in written {
        if !manifest.injected_files.contains(&path) {
            manifest.injected_files.push(path);
        }
    }
    manifest::save(&resolution.tod, &manifest)?;

    println!("[AutoGSE] Generated {added} controller action-set file(s) from the supplied .vdf.");
    Ok(())
}

/// Resolves which Steam access mode `run_inject` should use, per roadmap.md
/// Phase 5: login is the default once configured, `--anon` is always
/// honored as an explicit opt-out, and a first-run machine (neither
/// credentials nor an `anon_opt_in` preference on record) gets the
/// disclosure prompt on interactive runs or a silent, non-persisted
/// anonymous fallback plus a toast on non-interactive ones (context-menu
/// clicks, `--silent`) — a blocking prompt there would just hang the click.
fn resolve_auth_mode(args: &TargetArgs, interactive: bool, out: &Output) -> Result<AuthMode, AutoGseError> {
    if args.anon {
        return Ok(AuthMode::Anonymous);
    }

    if let Some(creds) = credentials::load()? {
        return Ok(AuthMode::Authenticated { username: creds.username, password: creds.password });
    }

    if preferences::load()?.anon_opt_in {
        return Ok(AuthMode::Anonymous);
    }

    if !interactive {
        notify::show(
            "AutoGSE",
            "Injected without achievement data (no Steam login configured). Run \"autogse login\" once to enable it.",
        );
        return Ok(AuthMode::Anonymous);
    }

    match login_prompt::prompt_disclosure_stdio() {
        DisclosureChoice::LogInNow => match login_prompt::capture_login_stdio() {
            Ok(creds) => {
                credentials::save(&creds)?;
                out.info(format!(
                    "Logged in as {}. Future injections will include achievement data automatically.",
                    creds.username
                ));
                Ok(AuthMode::Authenticated { username: creds.username, password: creds.password })
            }
            Err(e) => {
                out.warn(format!("Login failed: {e}. Continuing anonymously for this run."));
                Ok(AuthMode::Anonymous)
            }
        },
        DisclosureChoice::AnonForever => {
            preferences::set_anon_opt_in(true)?;
            Ok(AuthMode::Anonymous)
        }
        DisclosureChoice::AnonOnce | DisclosureChoice::Cancelled => Ok(AuthMode::Anonymous),
    }
}

/// Resolves and applies the persona (language / account name / SteamID64)
/// for this injection, per roadmap Phase 6 §6.1: an explicit CLI flag wins,
/// then a saved `preferences.json` default, then the emu's own generated
/// default is left alone entirely (no key is touched unless something
/// resolved). `--language` is validated against the target's own
/// `supported_languages.txt` when the merged tree includes one, rather than
/// letting the emu silently ignore an unsupported value.
fn apply_persona(tod: &Path, configs_user_ini: &Path, args: &TargetArgs, interactive: bool, out: &Output) -> Result<(), AutoGseError> {
    let prefs = preferences::load()?;

    let language = args.language.clone().or_else(|| prefs.default_language.clone());
    if let Some(lang) = &language {
        let supported_path = tod.join("steam_settings").join("supported_languages.txt");
        if supported_path.is_file() {
            let supported = std::fs::read_to_string(&supported_path)?;
            if !supported.lines().any(|l| l.trim().eq_ignore_ascii_case(lang)) {
                return Err(AutoGseError::UnsupportedLanguage(lang.clone()));
            }
        }
        ini_patch::set_key(configs_user_ini, "user::general", "language", lang)?;
    }

    let account_name = args.account_name.clone().or_else(|| prefs.default_account_name.clone());
    if let Some(name) = &account_name {
        ini_patch::set_key(configs_user_ini, "user::general", "account_name", name)?;
    }

    if let Some(steamid) = args.steamid {
        ini_patch::set_key(configs_user_ini, "user::general", "account_steamid", &steamid.to_string())?;
    }

    // Only offer to save when a CLI flag supplied something not already
    // matching the saved default — avoids re-nagging every single run once a
    // default is already on record.
    let language_is_new = args.language.is_some() && args.language != prefs.default_language;
    let account_name_is_new = args.account_name.is_some() && args.account_name != prefs.default_account_name;
    if interactive && (language_is_new || account_name_is_new) && prompt_save_as_default_stdio() {
        preferences::set_default_persona(args.account_name.clone(), args.language.clone())?;
        out.info("Saved as your default persona for future injections.");
    }

    Ok(())
}

/// Enables the experimental overlay and applies any saved notification
/// tuning (roadmap Phase 6 §6.3). The crash-risk warning is shown
/// unconditionally whenever `--overlay` is passed — `--silent` runs can't
/// block on a confirmation prompt, so this can't be gated behind one.
fn apply_overlay(configs_overlay_ini: &Path, out: &Output) -> Result<(), AutoGseError> {
    out.warn(
        "Experimental overlay enabled: the vendored tool's own docs warn this \"might cause \
         crashes or other problems\" — use at your own risk.",
    );
    ini_patch::set_key(configs_overlay_ini, "overlay::general", "enable_experimental_overlay", "1")?;

    let prefs = preferences::load()?.overlay_prefs;
    if let Some(v) = &prefs.pos_achievement {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "PosAchievement", v)?;
    }
    if let Some(v) = &prefs.pos_invitation {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "PosInvitation", v)?;
    }
    if let Some(v) = &prefs.pos_chat_msg {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "PosChatMsg", v)?;
    }
    if let Some(v) = prefs.duration_progress {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "Notification_Duration_Progress", &v.to_string())?;
    }
    if let Some(v) = prefs.duration_achievement {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "Notification_Duration_Achievement", &v.to_string())?;
    }
    if let Some(v) = prefs.duration_invitation {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "Notification_Duration_Invitation", &v.to_string())?;
    }
    if let Some(v) = prefs.duration_chat {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "Notification_Duration_Chat", &v.to_string())?;
    }
    if let Some(v) = prefs.notification_animation {
        ini_patch::set_key(configs_overlay_ini, "overlay::appearance", "Notification_Animation", &v.to_string())?;
    }
    Ok(())
}

/// Networking/compatibility presets (roadmap Phase 6 §6.4). Confirmed
/// against the real vendored `configs.main.ini`: the roadmap's original
/// premise that these all lived in `[main::misc]` was wrong — `offline`/
/// `disable_networking`/`disable_lobby_creation` are `[main::connectivity]`,
/// `new_app_ticket`/`steam_deck` are `[main::general]`, and only the three
/// `--compat-flag` names below are actually `[main::misc]`.
const COMPAT_FLAGS: &[(&str, &str)] = &[
    ("achievements_bypass", "main::misc"),
    ("disable_steamoverlaygameid_env_var", "main::misc"),
    ("enable_steam_preowned_ids", "main::misc"),
    ("new_app_ticket", "main::general"),
];

fn apply_network_compat(configs_main_ini: &Path, args: &TargetArgs) -> Result<(), AutoGseError> {
    // Validate every requested flag before writing anything, so a typo in
    // the 3rd of 4 requested flags doesn't leave the first two applied and
    // the rest silently skipped.
    for flag in &args.compat_flag {
        if !COMPAT_FLAGS.iter().any(|(name, _)| name == flag) {
            return Err(AutoGseError::InvalidCompatFlag(flag.clone()));
        }
    }

    if args.offline {
        ini_patch::set_key(configs_main_ini, "main::connectivity", "offline", "1")?;
        ini_patch::set_key(configs_main_ini, "main::connectivity", "disable_networking", "1")?;
        ini_patch::set_key(configs_main_ini, "main::connectivity", "disable_lobby_creation", "1")?;
    }

    if args.steam_deck {
        ini_patch::set_key(configs_main_ini, "main::general", "steam_deck", "1")?;
    }

    for flag in &args.compat_flag {
        let (name, section) = COMPAT_FLAGS.iter().find(|(name, _)| name == flag).expect("validated above");
        ini_patch::set_key(configs_main_ini, section, name, "1")?;
    }

    Ok(())
}

fn validate_overlay_position(value: &Option<String>) -> Result<(), AutoGseError> {
    if let Some(v) = value {
        if !preferences::VALID_OVERLAY_POSITIONS.contains(&v.as_str()) {
            return Err(AutoGseError::InvalidOverlayPosition(v.clone()));
        }
    }
    Ok(())
}

fn run_configure_overlay(args: &cli::ConfigureOverlayArgs) -> Result<(), AutoGseError> {
    validate_overlay_position(&args.pos_achievement)?;
    validate_overlay_position(&args.pos_invitation)?;
    validate_overlay_position(&args.pos_chat_msg)?;

    preferences::set_overlay_prefs(preferences::OverlayPrefs {
        pos_achievement: args.pos_achievement.clone(),
        pos_invitation: args.pos_invitation.clone(),
        pos_chat_msg: args.pos_chat_msg.clone(),
        duration_progress: args.duration_progress,
        duration_achievement: args.duration_achievement,
        duration_invitation: args.duration_invitation,
        duration_chat: args.duration_chat,
        notification_animation: args.notification_animation,
    })?;

    println!("[AutoGSE] Overlay preferences saved. They'll apply on future `inject --overlay` runs.");
    Ok(())
}

fn prompt_save_as_default_stdio() -> bool {
    print!("[AutoGSE] Save this as your default persona for future injections? [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

fn unix_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

/// Dispatches to a single-target or `--root` batch run (Phase 6 §6.8).
/// `TargetArgs::path`/`root` are mutually exclusive and one is required,
/// enforced by clap (`conflicts_with`/`required_unless_present`) — by the
/// time this runs, exactly one is `Some`.
fn run_inject(args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    if let Some(root) = &args.root {
        return run_inject_batch(root, args, out);
    }
    run_inject_single(args.path.as_deref().expect("clap guarantees path or root"), args, None, out)
}

/// Scans `root` (`discovery::find_all_targets_under`) and injects every
/// discovered target, resolving `AuthMode` **once** up front and threading
/// it through every target instead of re-resolving (and re-prompting for
/// login) per game.
fn run_inject_batch(root: &Path, args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    let targets = discovery::find_all_targets_under(root)?;
    if targets.is_empty() {
        out.info(format!("No injectable targets found under {}.", root.display()));
        return Ok(());
    }

    let auth_mode = resolve_auth_mode(args, !args.silent, out)?;

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for target in &targets {
        match run_inject_single(&target.tod, args, Some(auth_mode.clone()), out) {
            Ok(()) => succeeded += 1,
            Err(e) => {
                out.warn(format!("{}: {e}", target.tod.display()));
                failed += 1;
            }
        }
    }
    out.info(format!("Batch inject complete: {succeeded} succeeded, {failed} failed, out of {} target(s).", targets.len()));
    Ok(())
}

/// `preresolved_auth`: `Some` when called from `run_inject_batch` (already
/// resolved once for the whole batch); `None` for a normal single-target
/// `inject --path`, which resolves it itself via `resolve_auth_mode` below.
fn run_inject_single(path: &Path, args: &TargetArgs, preresolved_auth: Option<AuthMode>, out: &Output) -> Result<(), AutoGseError> {
    let interactive = !args.silent;

    // Lock on D_root (knowable directly from `path`, before any scanning)
    // rather than the post-discovery TOD. Two concurrent full-inject
    // invocations both mutate the very files discovery scans for
    // (ensure_backed_up renames the DLL mid-injection), so a second
    // invocation's *discovery* racing ahead of the lock is not actually
    // harmless — it can transiently see no DLL at all. Locking on D_root
    // first serializes discovery itself, closing that window. Do not
    // "simplify" this back to locking on the post-discovery TOD.
    let d_root = discovery::compute_d_root(path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(path, interactive)?;

    if manifest::exists(&resolution.tod) {
        out.info(format!(
            "{} is already injected; use `autogse revert` first.",
            resolution.tod.display()
        ));
        return Ok(());
    }

    let arch = pe::read_bitness(&resolution.dll_path)?;

    // Resolved (and validated) before anything below mutates the game
    // folder: a missing-vendored-tools failure must never happen *after*
    // ensure_backed_up has already renamed the original DLL away, which
    // would leave the game unable to launch at all (no steam_api(64).dll
    // present in any name) until the user reverts. `steamclient` mode
    // (Phase 6 §6.5) never swaps this DLL at all, so it skips resolving a
    // source for it entirely.
    let dll_src = if args.mode == InjectMode::Regular { Some(goldberg::dll_source_path(arch)?) } else { None };

    let appid_ctx = AppIdContext { tod: &resolution.tod, exe_hint: path, override_appid: args.appid, interactive };
    let app_id_resolution = appid::resolve_app_id(&appid_ctx)?;

    // `steamclient` mode stages an alternate `steamclient(64).dll` loader
    // fileset instead (see `steamclient_mode::stage`, called further below)
    // and leaves the game's real `steam_api(64).dll`, and therefore this
    // entire lock-check/backup/swap block, untouched.
    let backed_up_opt = if let Some(dll_src) = &dll_src {
        if process_lock::is_file_locked(&resolution.dll_path) {
            let folder_hint = resolution.tod.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let process_hint = process_lock::find_running_process_hint(&folder_hint);
            let msg = match process_hint {
                Some(name) => format!(
                    "{} is in use by '{name}' (likely the game is running); close it and try again",
                    resolution.dll_path.display()
                ),
                None => format!(
                    "{} is in use by another process (likely the game is running); close it and try again",
                    resolution.dll_path.display()
                ),
            };
            return Err(AutoGseError::ProcessRunning(msg));
        }

        let backed_up = backup::ensure_backed_up(&resolution.dll_path)?;

        // Real Goldberg emulator DLL (replaces the Phase 1/2 self-copy placeholder).
        backup::atomic_copy(dll_src, &resolution.dll_path)?;
        Some(backed_up)
    } else {
        None
    };

    let auth_mode = match preresolved_auth {
        Some(mode) => mode,
        None => resolve_auth_mode(args, interactive, out)?,
    };

    // Generate the per-game config via the real vendored tool, in an
    // isolated temp dir cleaned up automatically (RAII) once we're done
    // pulling what we need out of it.
    let gec_out = tempfile::Builder::new().prefix("autogse_gec_").tempdir()?;
    let gen_opts = goldberg::GenOptions { controller: args.controller, inventory: args.inventory };
    goldberg::run_generate_emu_config(app_id_resolution.app_id, gec_out.path(), &auth_mode, gen_opts)?;

    // Writes into gec_out's steam_settings/ (steam_interfaces.txt + .ini)
    // before the merge below, so the existing merge_steam_settings picks
    // them up automatically like any other generated file — no special
    // casing needed there. original_dll_path must be the real game DLL
    // AutoGSE just backed up, not anything generate_emu_config.exe produced.
    // `steamclient` mode skips this entirely — confirmed via the vendored
    // README: "You do not need to create a steam_interfaces.txt file for
    // the steamclient version of the emu".
    let interfaces_generated = if let Some(backed_up) = &backed_up_opt {
        let original_dll_path = resolution.tod.join(&backed_up.backup_path);
        goldberg::generate_interfaces(gec_out.path(), arch, &original_dll_path).unwrap_or(false)
    } else {
        false
    };

    if args.overlay {
        goldberg::deploy_overlay_assets(gec_out.path())?;
    }

    let existing_settings = resolution.tod.join("steam_settings");
    if existing_settings.is_dir() {
        if let Some(backed_up_dir) = backup::backup_existing_dir(&existing_settings)? {
            out.info(format!("Existing steam_settings/ backed up to {}.", backed_up_dir.display()));
        }
    }

    let mut injected_files = goldberg::merge_steam_settings(gec_out.path(), &resolution.tod)?;

    let configs_user_ini = resolution.tod.join("steam_settings").join("configs.user.ini");
    if configs_user_ini.is_file() {
        apply_persona(&resolution.tod, &configs_user_ini, args, interactive, out)?;
    }

    if args.overlay {
        let configs_overlay_ini = resolution.tod.join("steam_settings").join("configs.overlay.ini");
        if configs_overlay_ini.is_file() {
            apply_overlay(&configs_overlay_ini, out)?;
        }
    }

    let configs_main_ini = resolution.tod.join("steam_settings").join("configs.main.ini");
    if configs_main_ini.is_file() {
        apply_network_compat(&configs_main_ini, args)?;
    }

    if args.unlock_all_dlc {
        let configs_app_ini = resolution.tod.join("steam_settings").join("configs.app.ini");
        if configs_app_ini.is_file() {
            ini_patch::set_key(&configs_app_ini, "app::dlcs", "unlock_all", "1")?;
        }
    }

    // AutoGSE is the authoritative source for steam_appid.txt: Phase 2's
    // cascade already resolved and validated app_id, so we don't trust the
    // external tool's own (anonymous-login, best-effort) guess for this
    // one critical file.
    std::fs::write(resolution.tod.join("steam_appid.txt"), app_id_resolution.app_id.to_string())?;
    injected_files.push("steam_appid.txt".to_string());

    if args.mode == InjectMode::Steamclient {
        let game_exe = appid::pick_game_exe(path).ok_or_else(|| AutoGseError::NoGameExeFound(resolution.tod.clone()))?;
        let staged = steamclient_mode::stage(&resolution.tod, &game_exe.to_string_lossy(), app_id_resolution.app_id)?;
        injected_files.extend(staged);

        let loader_name = match arch {
            pe::Arch::X86 => "steamclient_loader_x32.exe",
            pe::Arch::X64 => "steamclient_loader_x64.exe",
        };
        out.info(format!(
            "steamclient mode: launch \"{loader_name}\" from {} to play — not the game exe directly.",
            resolution.tod.display()
        ));
    }

    // Achievement Watcher is a separate, already-installed application on
    // the user's machine — these writes go directly into its own data
    // folder (%APPDATA%\Achievement Watcher\), not anywhere AutoGSE's own
    // manifest/revert tracks. Best-effort: no-ops cleanly (Ok(false)) when
    // AW isn't installed or there's no -acw data (anonymous run).
    let acw_schema_deployed = acw::deploy_schema(gec_out.path()).unwrap_or(false);
    if acw_schema_deployed {
        let configs_user_ini = resolution.tod.join("steam_settings").join("configs.user.ini");
        let _ = acw::register_save_paths(&resolution.tod, &configs_user_ini);
    }

    if args.mode == InjectMode::Regular {
        if interfaces_generated {
            out.info("Generated steam_interfaces.txt for improved Goldberg interface-version compatibility.");
        } else {
            out.warn(
                "Could not generate steam_interfaces.txt (not fatal — the game will use Goldberg's default \
                 interface versions).",
            );
        }
    }
    if acw_schema_deployed {
        out.info("Deployed achievement schema and registered save path with Achievement Watcher.");
    } else if matches!(auth_mode, AuthMode::Authenticated { .. }) {
        out.warn("Could not deploy Achievement Watcher schema (Achievement Watcher may not be installed).");
    }
    if matches!(auth_mode, AuthMode::Anonymous) {
        out.warn(
            "No achievement data was generated (anonymous Steam access). Run `autogse login` to enable \
             achievement names, descriptions, and icons on future injections.",
        );
    }

    let display_title = app_id_resolution
        .game_title
        .clone()
        .unwrap_or_else(|| resolution.tod.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());

    let manifest = GseManifest {
        version: manifest::MANIFEST_VERSION.to_string(),
        timestamp: unix_timestamp(),
        target_directory: resolution.tod.to_string_lossy().into_owned(),
        backed_up_files: backed_up_opt.into_iter().collect(),
        app_id: Some(app_id_resolution.app_id),
        arch: Some(arch.to_string()),
        app_id_source: Some(app_id_resolution.source.as_str().to_string()),
        game_title: app_id_resolution.game_title,
        injected_files,
        mode: args.mode.to_string(),
    };
    manifest::save(&resolution.tod, &manifest)?;
    index::record(&resolution.tod)?;

    out.info(format!("Injection complete for {display_title} (AppID {}, {arch}).", app_id_resolution.app_id));
    notify::show(
        "AutoGSE: Injection Complete",
        &format!("Successfully injected {display_title} (AppID: {}).", app_id_resolution.app_id),
    );
    Ok(())
}

fn run_revert(args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    if let Some(root) = &args.root {
        return run_revert_batch(root, args, out);
    }
    run_revert_single(args.path.as_deref().expect("clap guarantees path or root"), args, out)
}

/// Reverts every target `discovery::find_all_targets_under(root)` finds.
/// Unlike inject, there's no shared login session to resolve up front — a
/// vanilla (never-injected) target in the batch is already a harmless no-op
/// via `run_revert_single`'s own "nothing to revert" early return.
fn run_revert_batch(root: &Path, args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    let targets = discovery::find_all_targets_under(root)?;
    if targets.is_empty() {
        out.info(format!("No targets found under {}.", root.display()));
        return Ok(());
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for target in &targets {
        match run_revert_single(&target.tod, args, out) {
            Ok(()) => succeeded += 1,
            Err(e) => {
                out.warn(format!("{}: {e}", target.tod.display()));
                failed += 1;
            }
        }
    }
    out.info(format!("Batch revert complete: {succeeded} succeeded, {failed} failed, out of {} target(s).", targets.len()));
    Ok(())
}

fn run_revert_single(path: &Path, args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    let interactive = !args.silent;

    let d_root = discovery::compute_d_root(path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(path, interactive)?;

    let Some(manifest) = manifest::load(&resolution.tod)? else {
        out.info(format!("Nothing to revert at {}.", resolution.tod.display()));
        return Ok(());
    };

    for entry in &manifest.backed_up_files {
        restore_one(&resolution.tod, entry)?;
    }

    for rel_path in &manifest.injected_files {
        let path = resolution.tod.join(rel_path);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AutoGseError::Io(e)),
        }
    }

    let settings_dir = resolution.tod.join("steam_settings");
    if settings_dir.is_dir() {
        std::fs::remove_dir_all(&settings_dir)?;
    }

    manifest::remove(&resolution.tod)?;
    index::forget(&resolution.tod)?;

    // steam_settings.bak_<timestamp> folders are a one-way safety net, never
    // auto-restored (see backup::backup_existing_dir) — just surface that
    // they exist so they're not a silent, forgotten artifact.
    let bak_count = std::fs::read_dir(&resolution.tod)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("steam_settings.bak_"))
        .count();
    if bak_count > 0 {
        out.info(format!("{bak_count} steam_settings.bak_* folder(s) left in place for manual review."));
    }

    out.info(format!("Rollback complete for {}.", resolution.tod.display()));
    // `steamclient` mode (Phase 6 §6.5) never swapped a DLL, so
    // `backed_up_files` is empty and there's nothing to say was "restored" —
    // only the staged loader fileset/configs were removed.
    let notify_body = match manifest.backed_up_files.first() {
        Some(entry) => format!("Restored original {} and removed emulator configs.", entry.original_path),
        None => "Removed the steamclient loader files and emulator configs.".to_string(),
    };
    notify::show("AutoGSE: Rollback Complete", &notify_body);
    Ok(())
}

fn restore_one(target_dir: &Path, entry: &BackedUpFile) -> Result<(), AutoGseError> {
    let original = target_dir.join(&entry.original_path);
    backup::restore_backup(&original, entry, target_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn target_args_from(extra: &[&str]) -> TargetArgs {
        let mut argv = vec!["autogse", "inject", "--path", "C:\\Games\\Foo"];
        argv.extend_from_slice(extra);
        let cli = Cli::parse_from(argv);
        let Command::Inject(args) = cli.command else { panic!("expected Inject") };
        args
    }

    fn write_configs_main_ini(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("configs.main.ini");
        std::fs::write(
            &path,
            "[main::general]\r\nnew_app_ticket=1\r\nsteam_deck=0\r\n\r\n[main::connectivity]\r\noffline=0\r\n\r\n[main::misc]\r\nachievements_bypass=0\r\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn apply_network_compat_is_a_noop_with_no_flags() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_configs_main_ini(dir.path());
        let before = std::fs::read_to_string(&path).unwrap();

        apply_network_compat(&path, &target_args_from(&[])).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn apply_network_compat_offline_sets_all_three_connectivity_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_configs_main_ini(dir.path());

        apply_network_compat(&path, &target_args_from(&["--offline"])).unwrap();

        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("offline=1"));
        assert!(result.contains("disable_networking=1"));
        assert!(result.contains("disable_lobby_creation=1"));
    }

    #[test]
    fn apply_network_compat_steam_deck_sets_general_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_configs_main_ini(dir.path());

        apply_network_compat(&path, &target_args_from(&["--steam-deck"])).unwrap();

        assert!(std::fs::read_to_string(&path).unwrap().contains("steam_deck=1"));
    }

    #[test]
    fn apply_network_compat_accepts_each_valid_compat_flag() {
        for flag in ["achievements_bypass", "disable_steamoverlaygameid_env_var", "enable_steam_preowned_ids", "new_app_ticket"] {
            let dir = tempfile::tempdir().unwrap();
            let path = write_configs_main_ini(dir.path());
            apply_network_compat(&path, &target_args_from(&["--compat-flag", flag])).unwrap();
            assert!(std::fs::read_to_string(&path).unwrap().contains(&format!("{flag}=1")));
        }
    }

    #[test]
    fn apply_network_compat_rejects_unknown_compat_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_configs_main_ini(dir.path());

        let result = apply_network_compat(&path, &target_args_from(&["--compat-flag", "not_a_real_flag"]));

        assert!(matches!(result, Err(AutoGseError::InvalidCompatFlag(_))));
    }

    #[test]
    fn apply_network_compat_rejects_before_writing_anything_on_a_later_invalid_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_configs_main_ini(dir.path());
        let before = std::fs::read_to_string(&path).unwrap();

        let result = apply_network_compat(&path, &target_args_from(&["--compat-flag", "achievements_bypass", "--compat-flag", "bogus"]));

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn classify_target_is_vanilla_when_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(classify_target(dir.path()).unwrap(), ScanStatus::Vanilla);
    }

    #[test]
    fn classify_target_is_injected_when_manifest_and_hashes_match() {
        let dir = tempfile::tempdir().unwrap();
        let backup_path = dir.path().join("steam_api64.dll.org");
        std::fs::write(&backup_path, b"original dll bytes").unwrap();
        let hash = backup::sha256_file(&backup_path).unwrap();

        let manifest = GseManifest {
            version: manifest::MANIFEST_VERSION.to_string(),
            timestamp: "unix:0".to_string(),
            target_directory: dir.path().to_string_lossy().into_owned(),
            backed_up_files: vec![BackedUpFile {
                original_path: "steam_api64.dll".to_string(),
                backup_path: "steam_api64.dll.org".to_string(),
                sha256_hash: hash,
            }],
            app_id: Some(480),
            arch: Some("x64".to_string()),
            app_id_source: None,
            game_title: None,
            injected_files: vec![],
            mode: "regular".to_string(),
        };
        manifest::save(dir.path(), &manifest).unwrap();

        assert_eq!(classify_target(dir.path()).unwrap(), ScanStatus::Injected);
    }

    #[test]
    fn classify_target_needs_update_when_backup_hash_mismatches() {
        let dir = tempfile::tempdir().unwrap();
        let backup_path = dir.path().join("steam_api64.dll.org");
        std::fs::write(&backup_path, b"original dll bytes").unwrap();

        let manifest = GseManifest {
            version: manifest::MANIFEST_VERSION.to_string(),
            timestamp: "unix:0".to_string(),
            target_directory: dir.path().to_string_lossy().into_owned(),
            backed_up_files: vec![BackedUpFile {
                original_path: "steam_api64.dll".to_string(),
                backup_path: "steam_api64.dll.org".to_string(),
                sha256_hash: "0".repeat(64),
            }],
            app_id: Some(480),
            arch: Some("x64".to_string()),
            app_id_source: None,
            game_title: None,
            injected_files: vec![],
            mode: "regular".to_string(),
        };
        manifest::save(dir.path(), &manifest).unwrap();

        assert_eq!(classify_target(dir.path()).unwrap(), ScanStatus::NeedsUpdate);
    }

    #[test]
    fn classify_target_needs_update_when_manifest_version_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(manifest::MANIFEST_FILENAME), r#"{"version": "0.0.1", "timestamp": "unix:0", "target_directory": "x", "backed_up_files": []}"#).unwrap();

        assert_eq!(classify_target(dir.path()).unwrap(), ScanStatus::NeedsUpdate);
    }
}
