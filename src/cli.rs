use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "autogse", version, about = "Automated Goldberg Achievement & Emulator Integrator")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Internal marker set on elevation relaunch to prevent relaunch loops.
    #[arg(long, hide = true, global = true)]
    pub elevated: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Inject the achievement emulator into a game folder or executable.
    Inject(TargetArgs),

    /// Revert a previously injected folder back to its vanilla state.
    Revert(TargetArgs),

    /// Register the Windows Explorer context-menu entries.
    #[command(hide = true, name = "install-menu")]
    InstallMenu,

    /// Remove the Windows Explorer context-menu entries.
    #[command(hide = true, name = "uninstall-menu")]
    UninstallMenu,

    /// Store Steam login credentials so future injections include
    /// achievement data. Without this, AutoGSE runs anonymously and skips
    /// achievement names/descriptions/icons.
    Login,

    /// Remove stored Steam login credentials (reverts to anonymous mode).
    Logout,

    /// Store RetroAchievements.org login credentials (username + Web API
    /// key) for the achievement viewer's RetroAchievements panel (§7.7). A
    /// separate account/secret from Steam login above — not required for
    /// Goldberg/Steam injection at all.
    #[command(name = "ra-login")]
    RaLogin,

    /// Remove stored RetroAchievements.org login credentials.
    #[command(name = "ra-logout")]
    RaLogout,

    /// Manually generate controller action-set files from a hand-supplied
    /// Steam `.vdf` (e.g. downloaded from SteamDB/Workshop), for games
    /// where `--controller`'s automatic download doesn't cover what's
    /// needed. A separate workflow from `inject --controller`, not a
    /// dependency of it.
    #[command(hide = true, name = "parse-controller-vdf")]
    ParseControllerVdf(ParseControllerVdfArgs),

    /// Save overlay notification tuning (position/duration) as a persisted
    /// preference profile, applied on every future `inject --overlay` run.
    /// Only the flags actually passed are updated; omitted ones keep
    /// whatever was previously saved.
    #[command(name = "configure-overlay")]
    ConfigureOverlay(ConfigureOverlayArgs),

    /// Scaffold one Steam Workshop mod entry into an already-injected
    /// target's `steam_settings/mods.json`.
    #[command(hide = true, name = "add-mod")]
    AddMod(AddModArgs),

    /// Launch the vendored `lobby_connect` tool against a game folder, for
    /// rich-presence-style lobby joins. This hands off to the tool's own
    /// interactive menu (it has no CLI flags of its own — confirmed via its
    /// `--help`), it does not automate lobby selection.
    Join(JoinArgs),

    /// Recursively find every injectable game under a games-library root and
    /// report status (vanilla / injected / needs update) — one folder at a
    /// time, unlike `inject`/`revert --root`.
    Scan(ScanArgs),

    /// Enumerate every folder AutoGSE has touched on this machine (a local
    /// index keyed off known `.gse_manifest.json` locations), so you can
    /// find all injected games without remembering where they are.
    List(ListArgs),

    /// Restages the Goldberg emulator DLL after a Steam update silently
    /// overwrote it with a vanilla copy (see `scan`/`list`'s "steam update
    /// reverted" status). Preserves the existing steam_settings/ and any
    /// custom configuration already on disk — does not regenerate anything.
    Reinject(ReinjectArgs),

    /// Diagnoses (and, where safely possible, fixes) a single corrupted or
    /// interrupted injection: a `.org` backup whose hash no longer matches
    /// `.gse_manifest.json`, a manifest schema older than this binary
    /// supports, or a DLL that was swapped mid-inject before a manifest was
    /// ever written at all.
    Repair(RepairArgs),

    /// Recursively scans a games-library folder (same immediate-subfolder
    /// convention as `scan --root`) and reports every target with an
    /// integrity problem `repair` could diagnose — read-only, never fixes
    /// anything itself.
    Audit(AuditArgs),

    /// Dump environment/tooling diagnostics (vendored tools resolution,
    /// DPAPI store reachability, recent log tail, known-target count) for
    /// troubleshooting — a failure otherwise is only ever visible in one
    /// console/toast and then gone.
    Doctor,

    /// Check GitHub releases for a newer AutoGSE version. Opt-in only:
    /// never runs automatically, never auto-downloads anything — just
    /// prints a message.
    #[command(name = "check-update")]
    CheckUpdate,

    /// Migrate save data between a game's real Steam-side save location and
    /// its Goldberg one (`<save_root>\<AppID>`), backing up whatever's
    /// currently at the destination first. The target must already be
    /// injected (its App ID comes from `.gse_manifest.json`).
    #[command(name = "sync-saves")]
    SyncSaves(SyncSavesArgs),

    /// Copies the real Steam controller glyph images from this machine's
    /// Steam install into an already-injected target, replacing the free
    /// example glyphs `generate_emu_config` deploys unconditionally.
    #[command(hide = true, name = "deploy-real-glyphs")]
    DeployRealGlyphs(DeployRealGlyphsArgs),

    /// Snapshots an already-injected target's achievement/save progress
    /// into a local timestamped backup, optionally also copying it to a
    /// local folder (e.g. an already-installed OneDrive/Google Drive/Dropbox
    /// sync folder — no cloud API integration, just an ordinary folder copy).
    #[command(name = "backup-achievements")]
    BackupAchievements(BackupAchievementsArgs),

    /// List every local backup snapshot recorded on this machine.
    #[command(name = "list-backups")]
    ListBackups,

    /// Restores one backup snapshot's achievement/save data onto a target.
    Restore(RestoreArgs),

    /// Manage this target's real network settings: its custom broadcast
    /// peer list (`steam_settings/custom_broadcasts.txt`) and its
    /// `listen_port` (`configs.main.ini`'s `[main::connectivity]` section).
    /// There is no "room code" concept in real GSE — joining a peer outside
    /// the local UDP broadcast domain (e.g. across a router or VPN) means
    /// adding their IP/domain here so the emulator sends broadcast traffic
    /// there directly; `join` (above) remains the actual lobby-discovery UI.
    Lan(LanArgs),

    /// Bundles an already-injected target's `.gse_manifest.json` and every
    /// file it lists (`steam_settings/`, `steam_appid.txt`, ...) into a
    /// portable zip package. Save-side achievement unlock progress is not
    /// included — see `backup-achievements` for that.
    Export(ExportArgs),

    /// Deploys a package built by `export` onto another (already-vanilla)
    /// copy of the same game, entirely offline: no network calls, no Steam
    /// API lookups. Still performs a real local DLL backup+swap for
    /// `regular`-mode packages (that step never needed the network to begin
    /// with) — only the config-generation step `export` already did once is
    /// skipped.
    Import(ImportArgs),

    /// Exports library-wide achievement completion statistics, timestamps,
    /// and unlock progress across every known-injected target on this
    /// machine — for external auditing or spreadsheets, not for backup/
    /// restore (see `backup-achievements`/`restore` for that).
    #[command(name = "export-achievements")]
    ExportAchievements(ExportAchievementsArgs),

    /// Parses one RPCS3 trophy set (`TROPCONF.SFM` + `TROPUSR.DAT`) and
    /// prints the trophy list with live unlock state. Not yet wired into
    /// `export-achievements` or the GUI achievement viewer — a standalone,
    /// tested parser first.
    #[command(hide = true, name = "rpcs3-trophies")]
    Rpcs3Trophies(Rpcs3TrophiesArgs),
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ExportAchievementsArgs {
    #[arg(long, value_enum)]
    pub format: ExportFormat,

    /// Write to this file instead of stdout — same flag name as `export`'s
    /// own output-package flag.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(clap::Args, Debug, Clone)]
pub struct Rpcs3TrophiesArgs {
    /// Folder containing one game's TROPCONF.SFM/TROPUSR.DAT directly (the
    /// real RPCS3 layout: `<dev_hdd0>/home/<user>/trophy/<trp_name>/`) —
    /// not RPCS3's install root or its whole trophy/ folder.
    #[arg(long)]
    pub path: PathBuf,

    /// Emit a JSON array instead of human-readable lines — same convention
    /// as `scan --json`.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct BackupAchievementsArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,

    /// Also copy the snapshot to this folder (any local folder, including
    /// an already-installed cloud-sync folder).
    #[arg(long)]
    pub cloud: Option<PathBuf>,
}

#[derive(clap::Args, Debug, Clone)]
pub struct RestoreArgs {
    /// The already-injected game folder to restore onto.
    #[arg(long)]
    pub path: PathBuf,

    /// The snapshot ID to restore, as shown by `list-backups`.
    #[arg(long)]
    pub snapshot: String,
}

#[derive(clap::Args, Debug, Clone)]
pub struct DeployRealGlyphsArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ReinjectArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`)
    /// whose steam_api(64).dll a Steam update reverted to vanilla.
    #[arg(long)]
    pub path: PathBuf,
}

#[derive(clap::Args, Debug, Clone)]
pub struct RepairArgs {
    /// The game folder to diagnose (and, where safely possible, fix).
    #[arg(long)]
    pub path: PathBuf,
}

#[derive(clap::Args, Debug, Clone)]
pub struct AuditArgs {
    /// Games-library folder whose immediate subfolders are audited, each as
    /// its own independent target — same convention as `scan --root`.
    #[arg(long)]
    pub root: PathBuf,

    /// Emit a JSON array instead of human-readable lines — see `scan
    /// --json`'s doc comment for the shared rationale.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct SyncSavesArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,

    #[arg(long, value_enum)]
    pub direction: SyncDirection,

    /// Explicit Steam-side save folder/file, overriding the automatic
    /// Ludusavi-manifest/common-directory resolution.
    #[arg(long = "steam-path")]
    pub steam_path: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    ToGoldberg,
    ToSteam,
}

#[derive(clap::Args, Debug, Clone)]
pub struct JoinArgs {
    /// Path to the game executable or its containing folder.
    #[arg(long)]
    pub path: PathBuf,
}

#[derive(clap::Args, Debug, Clone)]
pub struct LanArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,

    #[command(subcommand)]
    pub action: LanAction,
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum LanAction {
    /// Add a peer's IP or domain to this target's custom broadcast list.
    AddPeer { ip_or_domain: String },

    /// Remove a peer's IP or domain from the custom broadcast list.
    RemovePeer { ip_or_domain: String },

    /// List every peer currently in the custom broadcast list.
    ListPeers,

    /// Set the UDP/TCP port the emulator listens on — every peer must agree
    /// on the same port.
    SetListenPort { port: u16 },

    /// Apply a named network preset.
    ApplyPreset {
        #[arg(value_enum)]
        preset: CliNetworkPreset,

        /// Required when `preset` is `custom-port`.
        #[arg(long)]
        port: Option<u16>,
    },
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliNetworkPreset {
    Default,
    CustomPort,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ExportArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,

    /// Output package path (any filename/extension — it's a plain zip).
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ImportArgs {
    /// Package built by `export`.
    #[arg(long)]
    pub package: PathBuf,

    /// A vanilla (not yet AutoGSE-injected) copy of the same game.
    #[arg(long)]
    pub path: PathBuf,

    /// Overwrite an already-injected target instead of refusing.
    #[arg(long)]
    pub force: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ScanArgs {
    /// Games-library folder whose immediate subfolders are scanned, each as
    /// its own independent target (e.g. `SteamLibrary\steamapps\common\`).
    #[arg(long)]
    pub root: PathBuf,

    /// Emit a JSON array instead of human-readable lines — the stable,
    /// scriptable contract external tooling (Phase 11 §11.4's Playnite
    /// plugin) integrates against instead of parsing console text.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ListArgs {
    /// Emit a JSON array instead of human-readable lines — see `scan
    /// --json`'s doc comment for the shared rationale.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct AddModArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,

    /// Numeric mod/Workshop file ID (the key under `mods.json`).
    #[arg(long)]
    pub id: u64,

    #[arg(long)]
    pub title: String,

    #[arg(long)]
    pub description: Option<String>,

    /// Primary mod file, copied into `steam_settings/mods/<id>/`.
    #[arg(long)]
    pub file: PathBuf,

    /// Optional preview image, copied into `steam_settings/mods_img/<id>/`.
    #[arg(long)]
    pub preview: Option<PathBuf>,
}

#[derive(clap::Args, Debug, Clone, Default)]
pub struct ConfigureOverlayArgs {
    /// Position of achievement-unlock notifications.
    #[arg(long)]
    pub pos_achievement: Option<String>,

    /// Position of friend-invitation notifications.
    #[arg(long)]
    pub pos_invitation: Option<String>,

    /// Position of chat-message notifications.
    #[arg(long)]
    pub pos_chat_msg: Option<String>,

    /// Seconds an achievement-progress notification stays visible.
    #[arg(long)]
    pub duration_progress: Option<f64>,

    /// Seconds an achievement-unlock notification stays visible.
    #[arg(long)]
    pub duration_achievement: Option<f64>,

    /// Seconds a friend-invitation notification stays visible.
    #[arg(long)]
    pub duration_invitation: Option<f64>,

    /// Seconds a chat-message notification stays visible.
    #[arg(long)]
    pub duration_chat: Option<f64>,

    /// Seconds of slide-in/out animation (0 disables it).
    #[arg(long)]
    pub notification_animation: Option<f64>,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ParseControllerVdfArgs {
    /// The already-injected game folder (must contain `.gse_manifest.json`).
    #[arg(long)]
    pub path: PathBuf,

    /// One or more `.vdf` files to parse (repeatable).
    #[arg(long = "vdf", required = true)]
    pub vdf: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_controller_vdf_accepts_repeated_vdf_flag() {
        let cli = Cli::parse_from([
            "autogse",
            "parse-controller-vdf",
            "--path",
            "C:\\Games\\Foo",
            "--vdf",
            "a.vdf",
            "--vdf",
            "b.vdf",
        ]);
        let Command::ParseControllerVdf(args) = cli.command else { panic!("expected ParseControllerVdf") };
        assert_eq!(args.vdf, vec![PathBuf::from("a.vdf"), PathBuf::from("b.vdf")]);
    }

    #[test]
    fn parse_controller_vdf_requires_at_least_one_vdf() {
        let result = Cli::try_parse_from(["autogse", "parse-controller-vdf", "--path", "C:\\Games\\Foo"]);
        assert!(result.is_err());
    }
}

#[derive(clap::Args, Debug, Clone)]
pub struct TargetArgs {
    /// Path to the game executable or its containing folder. Exactly one of
    /// `--path`/`--root` is required.
    #[arg(long, conflicts_with = "root", required_unless_present = "root")]
    pub path: Option<PathBuf>,

    /// Games-library folder for a batch run: every immediate subfolder is
    /// treated as its own independent target (see `scan`'s same
    /// convention). Reuses a single resolved login session across all
    /// targets instead of prompting per game.
    #[arg(long, conflicts_with = "path", required_unless_present = "path")]
    pub root: Option<PathBuf>,

    /// Force a specific Steam App ID instead of auto-detecting it.
    #[arg(long)]
    pub appid: Option<u64>,

    /// Suppress console output unless an error occurs.
    #[arg(long)]
    pub silent: bool,

    /// Force anonymous Steam access for this run, even if login credentials
    /// are stored (skips achievement data; see `autogse login`).
    #[arg(long)]
    pub anon: bool,

    /// Override the emulator-reported language (e.g. `english`, `german`).
    /// Validated against the target's own `supported_languages.txt` when
    /// present. Falls back to the saved default persona (see `autogse
    /// login`'s sibling preference, set via this same flag) when omitted.
    #[arg(long)]
    pub language: Option<String>,

    /// Override the Steam persona name written to `configs.user.ini`'s
    /// `account_name`. Falls back to the saved default persona when omitted.
    #[arg(long = "account-name")]
    pub account_name: Option<String>,

    /// Override the SteamID64 written to `configs.user.ini`'s
    /// `account_steamid`. The emu ignores an invalid value and generates its
    /// own, so this is not further validated here.
    #[arg(long)]
    pub steamid: Option<u64>,

    /// Also download & generate Steam Input controller configuration files
    /// (off by default: opts out of `-skip_con`).
    #[arg(long)]
    pub controller: bool,

    /// Also download & generate inventory data (`items.json`/
    /// `default_items.json`) for games using `ISteamInventory` (off by
    /// default: opts out of `-skip_inv`).
    #[arg(long)]
    pub inventory: bool,

    /// Enable the emu's experimental in-game overlay
    /// (`enable_experimental_overlay=1`). The vendored tool's own caveat —
    /// "might cause crashes or other problems, USE AT YOUR OWN RISK" — is
    /// surfaced as a warning every time this is passed, since `--silent`
    /// runs can't block on a confirmation prompt.
    #[arg(long)]
    pub overlay: bool,

    /// Fully local, no-broadcast install: sets `configs.main.ini`'s
    /// `[main::connectivity]` → `offline=1`, `disable_networking=1`,
    /// `disable_lobby_creation=1`.
    #[arg(long)]
    pub offline: bool,

    /// Pretend the app is running on a Steam Deck
    /// (`[main::general]` → `steam_deck=1`).
    #[arg(long = "steam-deck")]
    pub steam_deck: bool,

    /// Enable a documented `configs.main.ini` compatibility workaround by
    /// its real key name (repeatable). Valid names: `achievements_bypass`,
    /// `disable_steamoverlaygameid_env_var`, `enable_steam_preowned_ids`
    /// (all `[main::misc]`), `new_app_ticket` (`[main::general]`).
    #[arg(long = "compat-flag")]
    pub compat_flag: Vec<String>,

    /// Report all DLCs as unlocked (`configs.app.ini`'s `[app::dlcs]` →
    /// `unlock_all=1`), for games that gate content behind owned-DLC checks.
    #[arg(long = "unlock-all-dlc")]
    pub unlock_all_dlc: bool,

    /// Injection mode. `regular` (default) swaps `steam_api(64).dll` for
    /// Goldberg's own, same as every phase through 5. `steamclient` instead
    /// stages the vendored `steamclient_experimental/` loader fileset and
    /// leaves the game's real `steam_api(64).dll` untouched — for games
    /// that verify that DLL on disk/in memory (anti-tamper checks) and
    /// would reject a swapped one.
    #[arg(long, value_enum, default_value_t = InjectMode::Regular)]
    pub mode: InjectMode,

    /// Skip the pre-injection anti-cheat/anti-tamper scan (Easy Anti-Cheat,
    /// BattlEye, VMProtect). Only relevant to `--mode regular` (`steamclient`
    /// mode never swaps the DLL these protections might be watching).
    #[arg(long = "skip-ac-scan")]
    pub skip_ac_scan: bool,

    /// Strip Valve's SteamStub DRM wrapper from the game's main executable
    /// (via the vendored Steamless) before injecting — some SteamStub-
    /// wrapped binaries won't run correctly once steam_api(64).dll is
    /// swapped underneath them. Off by default: this modifies the game's
    /// actual executable, a bigger risk than the DLL swap alone. A no-op
    /// (not an error) when the target isn't SteamStub-protected.
    #[arg(long = "unpack-steamstub")]
    pub unpack_steamstub: bool,

    /// Preview what `inject`/`revert` would do without changing anything in
    /// the target directory: resolved TOD, DLL arch, App ID, achievement-
    /// data availability, and (for `inject`) the real file list a full run
    /// would produce, generated into a scratch temp dir and reported but
    /// never merged in. Still makes the same network calls a real run would
    /// (App ID lookup, `generate_emu_config.exe`) — this is an accurate
    /// preview, not an offline one.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InjectMode {
    #[default]
    Regular,
    Steamclient,
}

impl std::fmt::Display for InjectMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_possible_value().expect("no skipped variants").get_name())
    }
}
