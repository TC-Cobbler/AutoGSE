use std::io::{BufRead, Write};

use windows::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE,
};

use crate::credentials::Credentials;
use crate::error::AutoGseError;

const RULE: &str = "===================================================================";

#[derive(Debug, PartialEq, Eq)]
pub enum DisclosureChoice {
    LogInNow,
    AnonOnce,
    AnonForever,
    /// EOF or an unparsable answer — callers must treat this the same as
    /// `AnonOnce` (a safe, non-committal fallback), not as an error, since a
    /// closed/non-interactive stdin can hit this even when the caller
    /// believed itself to be `interactive` (see `run_inject`'s existing
    /// `!args.silent` convention).
    Cancelled,
}

/// Roadmap §5.2's literal first-run disclosure: shown once per machine, the
/// first time `inject`/`revert` runs with neither `credentials.dat` nor an
/// `anon_opt_in` preference on record. Confirmed live (this phase) that
/// anonymous Steam login cannot fetch achievement data at all — not just
/// `-acw`'s multi-language schema — so this must name that gap plainly
/// rather than let the user discover a silently incomplete `steam_settings/`.
pub fn prompt_disclosure<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> DisclosureChoice {
    let _ = writeln!(writer, "{RULE}");
    let _ = writeln!(writer, " AutoGSE - Steam Login");
    let _ = writeln!(writer, "{RULE}");
    let _ = writeln!(writer, " No Steam login is configured yet.");
    let _ = writeln!(writer);
    let _ = writeln!(writer, " Without logging in, AutoGSE can still inject the emulator and");
    let _ = writeln!(writer, " generate everything except achievement data:");
    let _ = writeln!(writer);
    let _ = writeln!(writer, "   Works anonymously      : app name, DLCs, depots, branches, configs");
    let _ = writeln!(writer, "   Needs a Steam login    : achievement names, descriptions, icons");
    let _ = writeln!(writer);
    let _ = writeln!(writer, " Credentials are encrypted with Windows DPAPI and stored only on");
    let _ = writeln!(writer, " this PC (%LOCALAPPDATA%\\AutoGSE\\credentials.dat). AutoGSE never");
    let _ = writeln!(writer, " sends them anywhere except to Steam itself.");
    let _ = writeln!(writer, "{RULE}");
    let _ = writeln!(writer, " [1] Log in now");
    let _ = writeln!(writer, " [2] Continue without logging in (ask again next time)");
    let _ = writeln!(writer, " [3] Continue without logging in (don't ask again)");
    let _ = writeln!(writer, "{RULE}");
    let _ = write!(writer, " Select an option [1-3] (default 1, Enter to accept): ");
    let _ = writer.flush();

    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return DisclosureChoice::Cancelled,
        Ok(_) => {}
    }

    match line.trim() {
        "" | "1" => DisclosureChoice::LogInNow,
        "2" => DisclosureChoice::AnonOnce,
        "3" => DisclosureChoice::AnonForever,
        _ => DisclosureChoice::Cancelled,
    }
}

pub fn prompt_disclosure_stdio() -> DisclosureChoice {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout();
    prompt_disclosure(&mut stdin, &mut stdout)
}

fn print_login_header<W: Write>(writer: &mut W) {
    let _ = writeln!(writer, "{RULE}");
    let _ = writeln!(writer, " AutoGSE - Steam Login");
    let _ = writeln!(writer, "{RULE}");
    let _ = writeln!(writer, " Enter your Steam account credentials. They're encrypted with");
    let _ = writeln!(writer, " Windows DPAPI and stored only on this PC - never transmitted");
    let _ = writeln!(writer, " anywhere except to Steam itself.");
    let _ = writeln!(writer);
    let _ = writeln!(writer, " If Steam Guard prompts for a code afterward, enter it when asked.");
    let _ = writeln!(writer, "{RULE}");
}

/// Toggles `ENABLE_ECHO_INPUT` off on the real console's input handle for
/// the duration of one `read_line`, so the typed password isn't echoed.
/// Deliberately not generic over `BufRead`/testable via `Cursor`: console
/// mode manipulation only means anything against a real console handle,
/// same OS-interaction boundary as `notify.rs`/`shortcut.rs`. Falls back to
/// a plain (visible) read if no console is attached (e.g. output piped)
/// rather than failing the whole login.
fn read_password_stdio() -> Result<String, AutoGseError> {
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) }
        .map_err(|e| AutoGseError::LoginFailed(format!("could not access console input: {e}")))?;

    let mut original_mode = Default::default();
    let has_console = unsafe { GetConsoleMode(handle, &mut original_mode) }.is_ok();
    if has_console {
        let masked_mode = windows::Win32::System::Console::CONSOLE_MODE(original_mode.0 & !ENABLE_ECHO_INPUT.0);
        let _ = unsafe { SetConsoleMode(handle, masked_mode) };
    }

    let mut password = String::new();
    let read_result = std::io::stdin().read_line(&mut password);

    if has_console {
        let _ = unsafe { SetConsoleMode(handle, original_mode) };
    }

    read_result.map_err(AutoGseError::Io)?;
    Ok(password.trim_end_matches(['\r', '\n']).to_string())
}

/// The `login` subcommand's capture flow, and the target of the disclosure
/// prompt's "Log in now" choice.
pub fn capture_login_stdio() -> Result<Credentials, AutoGseError> {
    let mut stdout = std::io::stdout();
    print_login_header(&mut stdout);

    let _ = write!(stdout, " Steam username: ");
    let _ = stdout.flush();
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();
    if username.is_empty() {
        return Err(AutoGseError::LoginFailed("no username entered".to_string()));
    }

    let _ = write!(stdout, " Steam password: ");
    let _ = stdout.flush();
    let password = read_password_stdio()?;
    let _ = writeln!(stdout);
    if password.is_empty() {
        return Err(AutoGseError::LoginFailed("no password entered".to_string()));
    }

    Ok(Credentials { username, password })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bare_enter_defaults_to_log_in_now() {
        let mut input = Cursor::new(b"\n".to_vec());
        let mut output = Vec::new();
        assert_eq!(prompt_disclosure(&mut input, &mut output), DisclosureChoice::LogInNow);
    }

    #[test]
    fn explicit_1_is_log_in_now() {
        let mut input = Cursor::new(b"1\n".to_vec());
        let mut output = Vec::new();
        assert_eq!(prompt_disclosure(&mut input, &mut output), DisclosureChoice::LogInNow);
    }

    #[test]
    fn choice_2_is_anon_once() {
        let mut input = Cursor::new(b"2\n".to_vec());
        let mut output = Vec::new();
        assert_eq!(prompt_disclosure(&mut input, &mut output), DisclosureChoice::AnonOnce);
    }

    #[test]
    fn choice_3_is_anon_forever() {
        let mut input = Cursor::new(b"3\n".to_vec());
        let mut output = Vec::new();
        assert_eq!(prompt_disclosure(&mut input, &mut output), DisclosureChoice::AnonForever);
    }

    #[test]
    fn eof_is_cancelled() {
        let mut input = Cursor::new(Vec::new());
        let mut output = Vec::new();
        assert_eq!(prompt_disclosure(&mut input, &mut output), DisclosureChoice::Cancelled);
    }

    #[test]
    fn garbage_input_is_cancelled() {
        let mut input = Cursor::new(b"nope\n".to_vec());
        let mut output = Vec::new();
        assert_eq!(prompt_disclosure(&mut input, &mut output), DisclosureChoice::Cancelled);
    }

    #[test]
    fn renders_the_achievement_data_disclosure() {
        let mut input = Cursor::new(b"2\n".to_vec());
        let mut output = Vec::new();
        let _ = prompt_disclosure(&mut input, &mut output);
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("achievement data"));
        assert!(rendered.contains("DPAPI"));
    }
}
