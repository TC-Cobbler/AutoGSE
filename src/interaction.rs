use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::anticheat::AntiCheatFinding;
use crate::appid_prompt::{self, PickResult};
use crate::credentials::Credentials;
use crate::error::AutoGseError;
use crate::login_prompt::{self, DisclosureChoice};
use crate::steam_api::ScoredCandidate;

/// Phase 7 §7.0's replacement for the old `interactive: bool` +
/// hardcoded-`_stdio()` convention: every prompt the engine can surface is
/// one method here, so a caller (CLI today, a future GUI) supplies its own
/// implementation instead of the engine assuming a real console exists.
/// Threaded through as `Option<&dyn Interaction>`/`&dyn Interaction` — `None`
/// (where accepted) reproduces today's `interactive: false` behavior exactly.
pub trait Interaction {
    /// PRD §5.3.4's App ID disambiguation step. Returns the chosen App ID and
    /// its display name (`None` for a raw manually-entered ID).
    fn disambiguate_app_id(
        &self,
        target_dir: &Path,
        candidates: &[ScoredCandidate],
    ) -> Result<(u64, Option<String>), AutoGseError>;

    /// PRD §8's non-standard/renamed-DLL fallback: offered `near_matches`
    /// (any `.dll` containing `steam_api` that isn't an exact name match),
    /// plus an implicit "enter a path manually" option. `None` = cancelled.
    fn pick_dll(&self, near_matches: &[PathBuf], d_root: &Path) -> Option<PathBuf>;

    /// Phase 5 §5.2's first-run disclosure (login now / anon once / anon
    /// forever / cancelled).
    fn login_disclosure(&self) -> DisclosureChoice;

    /// Phase 5 §5.3's credential capture, target of `login_disclosure`'s
    /// "log in now" choice (and the standalone `login` subcommand).
    fn capture_login(&self) -> Result<Credentials, AutoGseError>;

    /// Phase 6 §6.1's "save this as your default persona?" confirmation.
    fn confirm_save_default_persona(&self) -> bool;

    /// Phase 10 §10.1's pre-injection anti-cheat/anti-tamper scan: `findings`
    /// is always non-empty when this is called (the caller only prompts on a
    /// real hit). `true` = proceed with the DLL swap anyway.
    fn confirm_anticheat_findings(&self, findings: &[AntiCheatFinding]) -> bool;
}

/// The CLI's `Interaction` implementation — behaviorally identical to the
/// pre-Phase-7 hardcoded stdio calls it replaces; every method here just
/// delegates to the same already-tested functions those call sites used
/// directly before this abstraction existed.
pub struct StdioInteraction;

impl Interaction for StdioInteraction {
    fn disambiguate_app_id(
        &self,
        target_dir: &Path,
        candidates: &[ScoredCandidate],
    ) -> Result<(u64, Option<String>), AutoGseError> {
        appid_prompt::prompt_app_id_disambiguation_stdio(target_dir, candidates)
    }

    fn pick_dll(&self, near_matches: &[PathBuf], _d_root: &Path) -> Option<PathBuf> {
        let options: Vec<String> = near_matches.iter().map(|p| p.display().to_string()).collect();
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout();

        match appid_prompt::pick_from_list(
            &mut stdin,
            &mut stdout,
            "AutoGSE - Non-Standard Steam DLL Detected",
            &options,
            Some("Enter DLL path manually"),
        ) {
            PickResult::Selected(i) => near_matches.get(i).cloned(),
            PickResult::Manual(value) => {
                let manual_path = PathBuf::from(value);
                if manual_path.is_file() { Some(manual_path) } else { None }
            }
            PickResult::Cancelled => None,
        }
    }

    fn login_disclosure(&self) -> DisclosureChoice {
        login_prompt::prompt_disclosure_stdio()
    }

    fn capture_login(&self) -> Result<Credentials, AutoGseError> {
        login_prompt::capture_login_stdio()
    }

    fn confirm_save_default_persona(&self) -> bool {
        print!("[AutoGSE] Save this as your default persona for future injections? [y/N]: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }

    fn confirm_anticheat_findings(&self, findings: &[AntiCheatFinding]) -> bool {
        println!("[AutoGSE] warning: possible anti-cheat/anti-tamper protection detected:");
        for finding in findings {
            println!("  - {}: {}", finding.system, finding.detail);
        }
        println!(
            "[AutoGSE] Swapping steam_api(64).dll may break this protection's own integrity checks. \
             Consider `--mode steamclient` instead, which leaves it untouched."
        );
        print!("[AutoGSE] Proceed with injection anyway? [y/N]: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }
}
