use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
}

#[derive(clap::Args, Debug, Clone)]
pub struct TargetArgs {
    /// Path to the game executable or its containing folder.
    #[arg(long)]
    pub path: PathBuf,

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
}
