use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, thiserror::Error)]
pub enum AutoGseError {
    #[error("target path does not exist: {0}")]
    TargetNotFound(PathBuf),

    #[error("no steam_api.dll or steam_api64.dll found under {0}")]
    DllNotFound(PathBuf),

    #[error("{0} is not a valid PE (DOS/NT header) file")]
    InvalidPeHeader(PathBuf),

    #[error("{0}")]
    ProcessRunning(String),

    #[error("another AutoGSE operation is already running against {0}")]
    AlreadyLocked(PathBuf),

    #[error("backup hash mismatch for {path}: expected {expected}, found {actual}; refusing to revert")]
    HashMismatch { path: PathBuf, expected: String, actual: String },

    #[error("registry operation failed: {0}")]
    Registry(String),

    #[error("elevation relaunch failed: {0}")]
    Elevation(String),

    #[error("could not determine a Steam App ID: {0}")]
    AppIdResolutionFailed(String),

    #[error("{tool} failed: {message}")]
    ExternalToolFailed { tool: String, message: String },

    #[error("{0} timed out")]
    ExternalToolTimeout(String),

    #[error("vendored GSE tools not found at {0}")]
    VendoredToolsNotFound(PathBuf),

    #[error("credential storage error: {0}")]
    Credentials(String),

    #[error("Steam login failed: {0}")]
    LoginFailed(String),

    #[error("Achievement Watcher integration error: {0}")]
    AchievementWatcher(String),

    #[error("'{0}' is not listed in this target's supported_languages.txt")]
    UnsupportedLanguage(String),

    #[error("{0} is not an injected AutoGSE target (no .gse_manifest.json) — run `inject` first")]
    NotInjected(PathBuf),

    #[error("'{0}' is not a valid overlay position (expected one of: top_left, top_center, top_right, bot_left, bot_center, bot_right)")]
    InvalidOverlayPosition(String),

    #[error("'{0}' is not a recognized --compat-flag (expected one of: achievements_bypass, disable_steamoverlaygameid_env_var, enable_steam_preowned_ids, new_app_ticket)")]
    InvalidCompatFlag(String),

    #[error("could not find a game executable under {0} to point --mode steamclient's loader at")]
    NoGameExeFound(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl AutoGseError {
    /// Process exit code, stable across releases so scripts can branch on it.
    pub fn exit_code(&self) -> u8 {
        match self {
            AutoGseError::TargetNotFound(_) => 2,
            AutoGseError::DllNotFound(_) => 3,
            AutoGseError::InvalidPeHeader(_) => 14,
            AutoGseError::ProcessRunning(_) => 4,
            AutoGseError::AlreadyLocked(_) => 5,
            AutoGseError::HashMismatch { .. } => 9,
            AutoGseError::Registry(_) => 10,
            AutoGseError::Elevation(_) => 11,
            AutoGseError::AppIdResolutionFailed(_) => 15,
            AutoGseError::Io(_) => 12,
            AutoGseError::Json(_) => 13,
            AutoGseError::ExternalToolFailed { .. } => 16,
            AutoGseError::ExternalToolTimeout(_) => 17,
            AutoGseError::VendoredToolsNotFound(_) => 18,
            AutoGseError::Credentials(_) => 19,
            AutoGseError::LoginFailed(_) => 20,
            AutoGseError::AchievementWatcher(_) => 21,
            AutoGseError::UnsupportedLanguage(_) => 22,
            AutoGseError::NotInjected(_) => 23,
            AutoGseError::InvalidOverlayPosition(_) => 24,
            AutoGseError::InvalidCompatFlag(_) => 25,
            AutoGseError::NoGameExeFound(_) => 26,
        }
    }
}

pub fn report_and_exit(err: anyhow::Error) -> ExitCode {
    let code = err
        .downcast_ref::<AutoGseError>()
        .map(AutoGseError::exit_code)
        .unwrap_or(1);
    eprintln!("[AutoGSE] error: {err:#}");
    // Best-effort — the persistent log (§6.9) is what makes a failure from a
    // context-menu click (no visible console) diagnosable after the window
    // closes, since the toast below is transient too.
    let _ = crate::log::append(&format!("run: ERROR {err:#}"));
    // Context-menu-triggered runs have no visible console, so a toast is the
    // only way a failure ever reaches the user.
    crate::notify::show("AutoGSE: Error", &format!("{err:#}"));
    ExitCode::from(code)
}
