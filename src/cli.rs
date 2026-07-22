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
    List,

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
}

#[derive(clap::Args, Debug, Clone)]
pub struct JoinArgs {
    /// Path to the game executable or its containing folder.
    #[arg(long)]
    pub path: PathBuf,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ScanArgs {
    /// Games-library folder whose immediate subfolders are scanned, each as
    /// its own independent target (e.g. `SteamLibrary\steamapps\common\`).
    #[arg(long)]
    pub root: PathBuf,
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
