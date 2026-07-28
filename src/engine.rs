//! Orchestration layer (Phase 7 §7.0): every `run_*` function here used to
//! live directly in the CLI's `main.rs`. Moved into the lib so a future GUI
//! binary can call the same inject/revert/scan/etc. workflows instead of
//! duplicating them — the CLI (`src/main.rs`) is now a thin dispatcher over
//! this module, passing its own `interaction::StdioInteraction` wherever a
//! prompt might be needed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::acw;
use crate::appid::{self, AppIdContext};
use crate::backup;
use crate::backup_manager;
use crate::cli::{
    AddModArgs, BackupAchievementsArgs, CliNetworkPreset, ConfigureOverlayArgs, DeployRealGlyphsArgs, InjectMode, JoinArgs, LanAction, LanArgs,
    ParseControllerVdfArgs, RestoreArgs, ScanArgs, SyncDirection, SyncSavesArgs, TargetArgs,
};
use crate::controller_glyphs;
use crate::credentials;
use crate::discovery;
use crate::error::AutoGseError;
use crate::goldberg::{self, AuthMode};
use crate::index;
use crate::ini_patch;
use crate::interaction::Interaction;
use crate::lan::{self, NetworkPreset};
use crate::login_prompt::DisclosureChoice;
use crate::manifest::{self, BackedUpFile, GseManifest};
use crate::mods;
use crate::mutex_engine::AutoGseLock;
use crate::notify;
use crate::output::Output;
use crate::pe;
use crate::preferences;
use crate::process_lock;
use crate::registry;
use crate::save_sync;
use crate::shortcut;
use crate::steamclient_mode;
use crate::update_check;

/// Named mutex wait timeout: long enough to let a concurrent inject/revert on
/// the same folder finish, short enough not to hang a user's click forever.
const LOCK_TIMEOUT_MS: u32 = 10_000;

pub fn run_install_menu() -> Result<(), AutoGseError> {
    registry::install_context_menu()?;
    // Required for toast notifications to display at all from this
    // unpackaged exe, not just cosmetic — see shortcut.rs.
    shortcut::install()?;
    println!("[AutoGSE] Explorer context menu entries installed.");
    Ok(())
}

pub fn run_uninstall_menu() -> Result<(), AutoGseError> {
    registry::uninstall_context_menu()?;
    shortcut::uninstall()?;
    println!("[AutoGSE] Explorer context menu entries removed.");
    Ok(())
}

pub fn run_login(interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    let creds = interaction.capture_login()?;
    credentials::save(&creds)?;
    println!(
        "[AutoGSE] Logged in as {}. Future injections will include achievement data automatically.",
        creds.username
    );
    Ok(())
}

pub fn run_logout() -> Result<(), AutoGseError> {
    credentials::delete()?;
    println!("[AutoGSE] Logged out. Stored Steam credentials removed. Your anonymous preference, if any, is unchanged.");
    Ok(())
}

/// Phase 7 §7.7's RetroAchievements login — a separate credential store from
/// Steam's `run_login` above, captured via the CLI's own masked-input
/// `login_prompt::capture_ra_login_stdio` rather than threaded through
/// `Interaction` (that trait's `capture_login` is Steam-`Credentials`-typed
/// specifically, and RA login isn't part of the inject/revert prompt
/// cascade any `Interaction` implementation has to answer).
pub fn run_ra_login() -> Result<(), AutoGseError> {
    let creds = crate::login_prompt::capture_ra_login_stdio()?;
    crate::retroachievements::save(&creds)?;
    println!("[AutoGSE] RetroAchievements login saved for {}.", creds.username);
    Ok(())
}

pub fn run_ra_logout() -> Result<(), AutoGseError> {
    crate::retroachievements::delete()?;
    println!("[AutoGSE] RetroAchievements credentials removed.");
    Ok(())
}

/// One vendored-tool-resolution result (Phase 7 §7.8.6's Doctor panel needs
/// structured data to bind to, not `println!` output).
pub struct ToolCheck {
    pub name: &'static str,
    pub ok: bool,
    /// The resolved path on success, or the error message on failure.
    pub detail: String,
}

/// Phase 6 §6.9's `doctor` checks, collected as plain data rather than
/// printed directly — `run_doctor` below is now a thin printer over this,
/// mirroring how `list_dashboard_targets`/`run_scan` already split
/// "collect data" from "print it" for the dashboard (Phase 7 §7.2). This is
/// what Phase 7 §7.8.6's GUI Doctor panel binds to instead of duplicating
/// the checks a second time.
pub struct DoctorReport {
    pub tool_checks: Vec<ToolCheck>,
    pub dpapi_ok: bool,
    pub dpapi_detail: String,
    pub known_target_count: Result<usize, String>,
    pub log_tail: Vec<String>,
}

pub fn collect_doctor_report() -> DoctorReport {
    fn check(name: &'static str, result: Result<PathBuf, AutoGseError>) -> ToolCheck {
        match result {
            Ok(p) => ToolCheck { name, ok: true, detail: p.display().to_string() },
            Err(e) => ToolCheck { name, ok: false, detail: e.to_string() },
        }
    }

    let tool_checks = vec![
        check("generate_emu_config", goldberg::tools_root()),
        check("parse_controller_vdf", goldberg::parse_controller_vdf_root()),
        check("lobby_connect", goldberg::lobby_connect_root()),
        check("steamclient_experimental", goldberg::steamclient_experimental_root()),
    ];

    let (dpapi_ok, dpapi_detail) = match credentials::self_test() {
        Ok(()) => (true, "DPAPI credential store reachable".to_string()),
        Err(e) => (false, e.to_string()),
    };

    let known_target_count = index::load_existing_injected().map(|t| t.len()).map_err(|e| e.to_string());
    let log_tail = crate::log::tail(20).unwrap_or_default();

    DoctorReport { tool_checks, dpapi_ok, dpapi_detail, known_target_count, log_tail }
}

pub fn run_doctor() -> Result<(), AutoGseError> {
    let report = collect_doctor_report();
    println!("=== AutoGSE Doctor ===");

    for check in &report.tool_checks {
        if check.ok {
            println!("[OK]   {} tools resolved: {}", check.name, check.detail);
        } else {
            println!("[FAIL] {} tools: {}", check.name, check.detail);
        }
    }

    if report.dpapi_ok {
        println!("[OK]   {}", report.dpapi_detail);
    } else {
        println!("[FAIL] DPAPI credential store: {}", report.dpapi_detail);
    }

    match &report.known_target_count {
        Ok(n) => println!("[OK]   {n} known injected target(s) on this machine"),
        Err(e) => println!("[FAIL] known-target index: {e}"),
    }

    if report.log_tail.is_empty() {
        println!("--- log: no entries yet ---");
    } else {
        println!("--- log tail ({} line(s)) ---", report.log_tail.len());
        for line in &report.log_tail {
            println!("{line}");
        }
    }

    Ok(())
}

pub fn run_check_update() -> Result<(), AutoGseError> {
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

/// Status classification for one `scan --root` target (Phase 6 §6.8), also
/// the data source for Phase 7 §7.2's dashboard badges.
#[derive(Debug, PartialEq, Eq)]
pub enum ScanStatus {
    Vanilla,
    Injected,
    /// Manifest present but stale — either its schema version predates the
    /// running binary's, or a backed-up file's recorded SHA-256 no longer
    /// matches what's on disk (reusing the same hash-check
    /// `backup::restore_backup` already performs on revert, just without
    /// actually restoring).
    NeedsUpdate,
}

pub fn classify_target(tod: &Path) -> Result<ScanStatus, AutoGseError> {
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

/// One row of Phase 7 §7.2's dashboard: a discovered-or-known target plus its
/// classification and (if injected) resolved App ID/title, for the GUI to
/// render a badge and fetch header art from — `app_id`/`game_title` are
/// `None` for a `Vanilla` target (nothing has been injected there yet, so
/// there is nothing recorded to look either up from).
pub struct DashboardRow {
    pub tod: PathBuf,
    pub status: ScanStatus,
    pub app_id: Option<u64>,
    pub game_title: Option<String>,
}

/// Merges an on-disk `root` scan (`discovery::find_all_targets_under`) with
/// every already-known injected target (`index::load_existing_injected`) —
/// the latter surfaces targets outside `root` entirely (Phase 7 §7.2), the
/// same gap `scan --root`/the CLI `list` subcommand each only cover one half
/// of individually. Deduplicates by TOD so a target both under `root` and
/// already indexed isn't listed twice.
pub fn list_dashboard_targets(root: &Path) -> Result<Vec<DashboardRow>, AutoGseError> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();

    for target in discovery::find_all_targets_under(root)? {
        seen.insert(target.tod.clone());
        rows.push(dashboard_row(target.tod)?);
    }

    for tod in index::load_existing_injected()? {
        if seen.insert(tod.clone()) {
            rows.push(dashboard_row(tod)?);
        }
    }

    Ok(rows)
}

fn dashboard_row(tod: PathBuf) -> Result<DashboardRow, AutoGseError> {
    let status = classify_target(&tod)?;
    let manifest = manifest::load(&tod)?;
    Ok(DashboardRow {
        app_id: manifest.as_ref().and_then(|m| m.app_id),
        game_title: manifest.as_ref().and_then(|m| m.game_title.clone()),
        tod,
        status,
    })
}

pub fn run_scan(args: &ScanArgs) -> Result<(), AutoGseError> {
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

pub fn run_list() -> Result<(), AutoGseError> {
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
pub fn run_join(args: &JoinArgs, interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    let resolution = discovery::resolve_target(&args.path, Some(interaction))?;
    let arch = pe::read_bitness(&resolution.dll_path)?;

    println!("[AutoGSE] Launching lobby_connect for {} ({arch})...", resolution.tod.display());
    goldberg::run_lobby_connect(&resolution.tod, arch)
}

pub fn run_add_mod(args: &AddModArgs) -> Result<(), AutoGseError> {
    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    // Never prompts: an already-injected target must already have a
    // standard-named DLL on record, so there is nothing to disambiguate here.
    let resolution = discovery::resolve_target(&args.path, None)?;
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

pub fn run_parse_controller_vdf(args: &ParseControllerVdfArgs) -> Result<(), AutoGseError> {
    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(&args.path, None)?;
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

pub fn run_sync_saves(args: &SyncSavesArgs) -> Result<(), AutoGseError> {
    let d_root = discovery::compute_d_root(&args.path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(&args.path, None)?;
    let Some(manifest) = manifest::load(&resolution.tod)? else {
        return Err(AutoGseError::NotInjected(resolution.tod));
    };
    let app_id = manifest.app_id.ok_or_else(|| AutoGseError::SaveSync("target's manifest has no resolved App ID recorded".to_string()))?;
    let game_title = manifest
        .game_title
        .clone()
        .unwrap_or_else(|| resolution.tod.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());

    let direction = match args.direction {
        SyncDirection::ToGoldberg => save_sync::MigrateDirection::ToGoldberg,
        SyncDirection::ToSteam => save_sync::MigrateDirection::ToSteam,
    };
    let report = save_sync::migrate(&resolution.tod, app_id, &game_title, direction, args.steam_path.as_deref())?;

    let (from, to) = match report.direction {
        save_sync::MigrateDirection::ToGoldberg => (&report.steam_path, &report.goldberg_path),
        save_sync::MigrateDirection::ToSteam => (&report.goldberg_path, &report.steam_path),
    };
    println!("[AutoGSE] Migrated saves: {} -> {}.", from.display(), to.display());
    if let Some(backup) = &report.backed_up_destination {
        println!("[AutoGSE] Existing destination contents backed up to {}.", backup.display());
    }
    Ok(())
}

pub fn run_deploy_real_glyphs(args: &DeployRealGlyphsArgs) -> Result<(), AutoGseError> {
    let resolution = discovery::resolve_target(&args.path, None)?;
    if manifest::load(&resolution.tod)?.is_none() {
        return Err(AutoGseError::NotInjected(resolution.tod));
    }

    if controller_glyphs::deploy_real_glyphs(&resolution.tod)? {
        println!("[AutoGSE] Deployed real Steam controller glyphs to {}.", resolution.tod.display());
    } else {
        println!("[AutoGSE] No real Steam controller glyphs deployed (Steam not found, or no glyph images present).");
    }
    Ok(())
}

pub fn run_backup_achievements(args: &BackupAchievementsArgs) -> Result<(), AutoGseError> {
    let resolution = discovery::resolve_target(&args.path, None)?;
    let Some(manifest) = manifest::load(&resolution.tod)? else {
        return Err(AutoGseError::NotInjected(resolution.tod));
    };
    let app_id = manifest.app_id.ok_or_else(|| AutoGseError::SaveSync("target's manifest has no resolved App ID recorded".to_string()))?;

    let snapshot = backup_manager::backup(&resolution.tod, app_id, args.cloud.as_deref())?;
    println!("[AutoGSE] Backup snapshot {} created at {}.", snapshot.manifest.id, snapshot.path.display());
    if !snapshot.manifest.achievements_backed_up {
        println!("[AutoGSE] No achievement unlock-state file found yet (nothing to back up there).");
    }
    if !snapshot.manifest.save_backed_up {
        println!("[AutoGSE] No Goldberg save directory found yet (nothing to back up there).");
    }
    if let Some(cloud) = &args.cloud {
        println!("[AutoGSE] Also copied to {}.", cloud.display());
    }
    Ok(())
}

pub fn run_list_backups() -> Result<(), AutoGseError> {
    let backups = backup_manager::list_backups()?;
    if backups.is_empty() {
        println!("[AutoGSE] No local backup snapshots recorded on this machine.");
        return Ok(());
    }
    for b in &backups {
        println!("[{}] AppID {} — {} (achievements: {}, save: {})", b.id, b.app_id, b.tod, b.achievements_backed_up, b.save_backed_up);
    }
    println!("[AutoGSE] {} backup snapshot(s) known on this machine.", backups.len());
    Ok(())
}

pub fn run_restore(args: &RestoreArgs) -> Result<(), AutoGseError> {
    let resolution = discovery::resolve_target(&args.path, None)?;
    if manifest::load(&resolution.tod)?.is_none() {
        return Err(AutoGseError::NotInjected(resolution.tod));
    }

    backup_manager::restore(&args.snapshot, &resolution.tod)?;
    println!("[AutoGSE] Restored snapshot {} onto {}.", args.snapshot, resolution.tod.display());
    Ok(())
}

pub fn run_lan(args: &LanArgs) -> Result<(), AutoGseError> {
    let resolution = discovery::resolve_target(&args.path, None)?;
    if manifest::load(&resolution.tod)?.is_none() {
        return Err(AutoGseError::NotInjected(resolution.tod));
    }
    let tod = &resolution.tod;

    match &args.action {
        LanAction::AddPeer { ip_or_domain } => {
            lan::add_peer(tod, ip_or_domain)?;
            println!("[AutoGSE] Added {ip_or_domain} to this target's custom broadcast list.");
        }
        LanAction::RemovePeer { ip_or_domain } => {
            lan::remove_peer(tod, ip_or_domain)?;
            println!("[AutoGSE] Removed {ip_or_domain} from this target's custom broadcast list.");
        }
        LanAction::ListPeers => {
            let peers = lan::list_peers(tod)?;
            if peers.is_empty() {
                println!("[AutoGSE] No custom broadcast peers configured for this target.");
            }
            for peer in &peers {
                println!("{peer}");
            }
        }
        LanAction::SetListenPort { port } => {
            lan::set_listen_port(tod, *port)?;
            println!("[AutoGSE] listen_port set to {port}.");
        }
        LanAction::ApplyPreset { preset, port } => {
            let resolved = match preset {
                CliNetworkPreset::Default => NetworkPreset::Default,
                CliNetworkPreset::CustomPort => {
                    let port = port.ok_or_else(|| AutoGseError::Lan("--port is required for the custom-port preset".to_string()))?;
                    NetworkPreset::CustomPort(port)
                }
            };
            lan::apply_preset(tod, resolved)?;
            println!("[AutoGSE] Applied network preset {preset:?} (listen_port is now {}).", lan::get_listen_port(tod)?);
        }
    }
    Ok(())
}

/// Resolves which Steam access mode `run_inject_single` should use, per
/// roadmap.md Phase 5: login is the default once configured, `--anon` is
/// always honored as an explicit opt-out, and a first-run machine (neither
/// credentials nor an `anon_opt_in` preference on record) gets the
/// disclosure prompt on interactive runs or a silent, non-persisted
/// anonymous fallback plus a toast on non-interactive ones (context-menu
/// clicks, `--silent`) — a blocking prompt there would just hang the click.
fn resolve_auth_mode(args: &TargetArgs, interactive: bool, out: &Output, interaction: &dyn Interaction) -> Result<AuthMode, AutoGseError> {
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

    match interaction.login_disclosure() {
        DisclosureChoice::LogInNow => match interaction.capture_login() {
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

/// Validates `lang` against the target's own `steam_settings/supported_languages.txt`
/// (when present — an emu-generated tree always has one, but this is also
/// called against arbitrary paths from the GUI's config editor) before
/// writing it, rather than letting the emu silently ignore an unsupported
/// value. Shared by `apply_persona` (CLI `--language`/saved default) and
/// Phase 7 §7.4's config editor (manual language dropdown / persona
/// switcher) so both paths enforce the identical rule.
pub fn set_language(tod: &Path, configs_user_ini: &Path, lang: &str) -> Result<(), AutoGseError> {
    let supported_path = tod.join("steam_settings").join("supported_languages.txt");
    if supported_path.is_file() {
        let supported = std::fs::read_to_string(&supported_path)?;
        if !supported.lines().any(|l| l.trim().eq_ignore_ascii_case(lang)) {
            return Err(AutoGseError::UnsupportedLanguage(lang.to_string()));
        }
    }
    ini_patch::set_key(configs_user_ini, "user::general", "language", lang)
}

/// Resolves and applies the persona (language / account name / SteamID64)
/// for this injection, per roadmap Phase 6 §6.1: an explicit CLI flag wins,
/// then a saved `preferences.json` default, then the emu's own generated
/// default is left alone entirely (no key is touched unless something
/// resolved). `--language` is validated against the target's own
/// `supported_languages.txt` when the merged tree includes one, rather than
/// letting the emu silently ignore an unsupported value.
fn apply_persona(
    tod: &Path,
    configs_user_ini: &Path,
    args: &TargetArgs,
    interactive: bool,
    out: &Output,
    interaction: &dyn Interaction,
) -> Result<(), AutoGseError> {
    let prefs = preferences::load()?;

    let language = args.language.clone().or_else(|| prefs.default_language.clone());
    if let Some(lang) = &language {
        set_language(tod, configs_user_ini, lang)?;
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
    if interactive && (language_is_new || account_name_is_new) && interaction.confirm_save_default_persona() {
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

pub fn run_configure_overlay(args: &ConfigureOverlayArgs) -> Result<(), AutoGseError> {
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
pub fn run_inject(args: &TargetArgs, out: &Output, interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    if let Some(root) = &args.root {
        return run_inject_batch(root, args, out, interaction);
    }
    run_inject_single(args.path.as_deref().expect("clap guarantees path or root"), args, None, out, interaction)
}

/// Scans `root` (`discovery::find_all_targets_under`) and injects every
/// discovered target, resolving `AuthMode` **once** up front and threading
/// it through every target instead of re-resolving (and re-prompting for
/// login) per game.
fn run_inject_batch(root: &Path, args: &TargetArgs, out: &Output, interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    let targets = discovery::find_all_targets_under(root)?;
    if targets.is_empty() {
        out.info(format!("No injectable targets found under {}.", root.display()));
        return Ok(());
    }

    let auth_mode = resolve_auth_mode(args, !args.silent, out, interaction)?;

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for target in &targets {
        match run_inject_single(&target.tod, args, Some(auth_mode.clone()), out, interaction) {
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
fn run_inject_single(
    path: &Path,
    args: &TargetArgs,
    preresolved_auth: Option<AuthMode>,
    out: &Output,
    interaction: &dyn Interaction,
) -> Result<(), AutoGseError> {
    let interactive = !args.silent;
    let prompt = interactive.then_some(interaction);

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

    let resolution = discovery::resolve_target(path, prompt)?;

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

    let appid_ctx = AppIdContext { tod: &resolution.tod, exe_hint: path, override_appid: args.appid, interaction: prompt };
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
        None => resolve_auth_mode(args, interactive, out, interaction)?,
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
        apply_persona(&resolution.tod, &configs_user_ini, args, interactive, out, interaction)?;
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

pub fn run_revert(args: &TargetArgs, out: &Output, interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    if let Some(root) = &args.root {
        return run_revert_batch(root, args, out, interaction);
    }
    run_revert_single(args.path.as_deref().expect("clap guarantees path or root"), args, out, interaction)
}

/// Reverts every target `discovery::find_all_targets_under(root)` finds.
/// Unlike inject, there's no shared login session to resolve up front — a
/// vanilla (never-injected) target in the batch is already a harmless no-op
/// via `run_revert_single`'s own "nothing to revert" early return.
fn run_revert_batch(root: &Path, args: &TargetArgs, out: &Output, interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    let targets = discovery::find_all_targets_under(root)?;
    if targets.is_empty() {
        out.info(format!("No targets found under {}.", root.display()));
        return Ok(());
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for target in &targets {
        match run_revert_single(&target.tod, args, out, interaction) {
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

fn run_revert_single(path: &Path, args: &TargetArgs, out: &Output, interaction: &dyn Interaction) -> Result<(), AutoGseError> {
    let interactive = !args.silent;

    let d_root = discovery::compute_d_root(path)?;
    let _lock = AutoGseLock::acquire(&d_root, LOCK_TIMEOUT_MS)?;

    let resolution = discovery::resolve_target(path, interactive.then_some(interaction))?;

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
    use crate::cli::{Cli, Command};
    use clap::Parser;

    #[test]
    fn collect_doctor_report_checks_all_four_vendored_tools() {
        let report = collect_doctor_report();
        let names: Vec<&str> = report.tool_checks.iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["generate_emu_config", "parse_controller_vdf", "lobby_connect", "steamclient_experimental"]);
    }

    #[test]
    fn collect_doctor_report_dpapi_self_test_passes_on_this_machine() {
        // Mirrors `credentials::tests::self_test_round_trips_successfully` —
        // this is the same real DPAPI round-trip, just observed through the
        // structured report instead of a printed line.
        let report = collect_doctor_report();
        assert!(report.dpapi_ok, "{}", report.dpapi_detail);
    }

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
