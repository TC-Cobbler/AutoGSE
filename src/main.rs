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
mod login_prompt;
mod manifest;
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
mod version_info;

use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;

use appid::AppIdContext;
use cli::{Cli, Command, TargetArgs};
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
    let cli = Cli::parse();
    let already_elevated = cli.elevated;

    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
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
    }
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

fn unix_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

fn run_inject(args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    let interactive = !args.silent;

    // Lock on D_root (knowable directly from args.path, before any
    // scanning) rather than the post-discovery TOD. Two concurrent
    // full-inject invocations both mutate the very files discovery scans
    // for (ensure_backed_up renames the DLL mid-injection), so a second
    // invocation's *discovery* racing ahead of the lock is not actually
    // harmless — it can transiently see no DLL at all. Locking on D_root
    // first serializes discovery itself, closing that window. Do not
    // "simplify" this back to locking on the post-discovery TOD.
    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(&args.path, interactive)?;

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
    // present in any name) until the user reverts.
    let dll_src = goldberg::dll_source_path(arch)?;

    let appid_ctx = AppIdContext { tod: &resolution.tod, exe_hint: &args.path, override_appid: args.appid, interactive };
    let app_id_resolution = appid::resolve_app_id(&appid_ctx)?;

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
    backup::atomic_copy(&dll_src, &resolution.dll_path)?;

    let auth_mode = resolve_auth_mode(args, interactive, out)?;

    // Generate the per-game config via the real vendored tool, in an
    // isolated temp dir cleaned up automatically (RAII) once we're done
    // pulling what we need out of it.
    let gec_out = tempfile::Builder::new().prefix("autogse_gec_").tempdir()?;
    goldberg::run_generate_emu_config(app_id_resolution.app_id, gec_out.path(), &auth_mode)?;

    // Writes into gec_out's steam_settings/ (steam_interfaces.txt + .ini)
    // before the merge below, so the existing merge_steam_settings picks
    // them up automatically like any other generated file — no special
    // casing needed there. original_dll_path must be the real game DLL
    // AutoGSE just backed up, not anything generate_emu_config.exe produced.
    let original_dll_path = resolution.tod.join(&backed_up.backup_path);
    let interfaces_generated = goldberg::generate_interfaces(gec_out.path(), arch, &original_dll_path).unwrap_or(false);

    let existing_settings = resolution.tod.join("steam_settings");
    if existing_settings.is_dir() {
        if let Some(backed_up_dir) = backup::backup_existing_dir(&existing_settings)? {
            out.info(format!("Existing steam_settings/ backed up to {}.", backed_up_dir.display()));
        }
    }

    let mut injected_files = goldberg::merge_steam_settings(gec_out.path(), &resolution.tod)?;

    // AutoGSE is the authoritative source for steam_appid.txt: Phase 2's
    // cascade already resolved and validated app_id, so we don't trust the
    // external tool's own (anonymous-login, best-effort) guess for this
    // one critical file.
    std::fs::write(resolution.tod.join("steam_appid.txt"), app_id_resolution.app_id.to_string())?;
    injected_files.push("steam_appid.txt".to_string());

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

    if interfaces_generated {
        out.info("Generated steam_interfaces.txt for improved Goldberg interface-version compatibility.");
    } else {
        out.warn(
            "Could not generate steam_interfaces.txt (not fatal — the game will use Goldberg's default \
             interface versions).",
        );
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
        backed_up_files: vec![backed_up],
        app_id: Some(app_id_resolution.app_id),
        arch: Some(arch.to_string()),
        app_id_source: Some(app_id_resolution.source.as_str().to_string()),
        game_title: app_id_resolution.game_title,
        injected_files,
    };
    manifest::save(&resolution.tod, &manifest)?;

    out.info(format!("Injection complete for {display_title} (AppID {}, {arch}).", app_id_resolution.app_id));
    notify::show(
        "AutoGSE: Injection Complete",
        &format!("Successfully injected {display_title} (AppID: {}).", app_id_resolution.app_id),
    );
    Ok(())
}

fn run_revert(args: &TargetArgs, out: &Output) -> Result<(), AutoGseError> {
    let interactive = !args.silent;

    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(&args.path, interactive)?;

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
    let dll_name = manifest.backed_up_files.first().map(|e| e.original_path.as_str()).unwrap_or("the emulator DLL");
    notify::show("AutoGSE: Rollback Complete", &format!("Restored original {dll_name} and removed emulator configs."));
    Ok(())
}

fn restore_one(target_dir: &Path, entry: &BackedUpFile) -> Result<(), AutoGseError> {
    let original = target_dir.join(&entry.original_path);
    backup::restore_backup(&original, entry, target_dir)
}
