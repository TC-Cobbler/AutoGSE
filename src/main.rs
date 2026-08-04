use std::process::ExitCode;

use clap::Parser;

use autogse::cli::{Cli, Command};
use autogse::error::{self, AutoGseError};
use autogse::interaction::StdioInteraction;
use autogse::output::Output;
use autogse::{elevate, engine, log};

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

/// Thin CLI dispatcher: every real workflow lives in `autogse::engine` (Phase
/// 7 §7.0), shared with the `autogse-gui` binary. This function's only job is
/// mapping a parsed `Command` onto the matching engine call, supplying the
/// CLI's own `StdioInteraction` wherever a prompt might be needed.
fn run(command: Command) -> Result<(), AutoGseError> {
    let interaction = StdioInteraction;
    match command {
        Command::Inject(args) => {
            let out = Output::new_with_json(args.silent, args.json);
            engine::run_inject(&args, &out, &interaction)
        }
        Command::Revert(args) => {
            let out = Output::new_with_json(args.silent, args.json);
            engine::run_revert(&args, &out, &interaction)
        }
        Command::InstallMenu => engine::run_install_menu(),
        Command::UninstallMenu => engine::run_uninstall_menu(),
        Command::Login(args) => engine::run_login(&args, &interaction),
        Command::Logout(args) => engine::run_logout(&args),
        Command::RaLogin => engine::run_ra_login(),
        Command::RaLogout => engine::run_ra_logout(),
        Command::ParseControllerVdf(args) => engine::run_parse_controller_vdf(&args),
        Command::ConfigureOverlay(args) => engine::run_configure_overlay(&args),
        Command::AddMod(args) => engine::run_add_mod(&args),
        Command::Join(args) => engine::run_join(&args, &interaction),
        Command::Scan(args) => engine::run_scan(&args),
        Command::List(args) => engine::run_list(&args),
        Command::Reinject(args) => engine::run_reinject(&args),
        Command::Repair(args) => engine::run_repair(&args),
        Command::Audit(args) => engine::run_audit(&args),
        Command::Doctor(args) => engine::run_doctor(&args),
        Command::CheckUpdate => engine::run_check_update(),
        Command::SyncSaves(args) => engine::run_sync_saves(&args),
        Command::DeployRealGlyphs(args) => engine::run_deploy_real_glyphs(&args),
        Command::BackupAchievements(args) => engine::run_backup_achievements(&args),
        Command::ListBackups => engine::run_list_backups(),
        Command::Restore(args) => engine::run_restore(&args),
        Command::Lan(args) => engine::run_lan(&args),
        Command::Export(args) => engine::run_export(&args),
        Command::Import(args) => engine::run_import(&args),
        Command::ExportAchievements(args) => engine::run_export_achievements(&args),
        Command::Rpcs3Trophies(args) => engine::run_rpcs3_trophies(&args),
    }
}
