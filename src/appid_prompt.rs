use std::io::{BufRead, Write};
use std::path::Path;

use crate::error::AutoGseError;
use crate::steam_api::ScoredCandidate;

/// Outcome of a boxed-ASCII numbered-list prompt (PRD §5.3.4's mockup style).
#[derive(Debug, PartialEq, Eq)]
pub enum PickResult {
    Selected(usize),
    Manual(String),
    Cancelled,
}

/// Shared "numbered list + optional manual entry" terminal prompt primitive,
/// used both for the App ID disambiguation UI (Step 5) and discovery's
/// non-standard-DLL-name fallback. Reads a single line; on EOF (e.g. a
/// non-interactive/closed stdin) or an unparsable/out-of-range answer it
/// returns `Cancelled` rather than blocking or looping.
pub fn pick_from_list<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    header: &str,
    options: &[String],
    manual_label: Option<&str>,
) -> PickResult {
    const RULE: &str = "===================================================================";
    let _ = writeln!(writer, "{RULE}");
    let _ = writeln!(writer, " {header}");
    let _ = writeln!(writer, "{RULE}");
    for (i, opt) in options.iter().enumerate() {
        let _ = writeln!(writer, " [{}] {}", i + 1, opt);
    }
    let manual_index = options.len() + 1;
    if let Some(label) = manual_label {
        let _ = writeln!(writer, " [{manual_index}] {label}");
    }
    let _ = writeln!(writer, "{RULE}");
    let max_index = if manual_label.is_some() { manual_index } else { options.len() };
    let _ = write!(writer, " Select an option [1-{max_index}]: ");
    let _ = writer.flush();

    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return PickResult::Cancelled,
        Ok(_) => {}
    }

    match line.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= options.len() => PickResult::Selected(n - 1),
        Ok(n) if manual_label.is_some() && n == manual_index => {
            let _ = write!(writer, " Enter value: ");
            let _ = writer.flush();
            let mut manual_line = String::new();
            match reader.read_line(&mut manual_line) {
                Ok(0) | Err(_) => PickResult::Cancelled,
                Ok(_) => {
                    let value = manual_line.trim().to_string();
                    if value.is_empty() {
                        PickResult::Cancelled
                    } else {
                        PickResult::Manual(value)
                    }
                }
            }
        }
        _ => PickResult::Cancelled,
    }
}

/// Step 5 (PRD §5.3.4): renders the boxed disambiguation UI over whatever
/// candidates Step 4 found (possibly none, if the network was unreachable),
/// plus a manual-entry option. This is always the cascade's terminal step —
/// it must return a concrete App ID or a descriptive error, never silently
/// fall through.
pub fn prompt_app_id_disambiguation<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    target_dir: &Path,
    candidates: &[ScoredCandidate],
) -> Result<(u64, Option<String>), AutoGseError> {
    let _ = writeln!(writer, " Target Directory: {}", target_dir.display());
    if candidates.is_empty() {
        let _ = writeln!(writer, " Could not determine a Steam App ID automatically (no network match found).");
    } else {
        let _ = writeln!(writer, " Could not auto-verify Steam App ID with high confidence.");
        let _ = writeln!(writer);
        let _ = writeln!(writer, " Top Candidate Matches:");
    }

    let options: Vec<String> = candidates
        .iter()
        .map(|c| format!("{} (AppID: {}) - Confidence: {:.0}%", c.name, c.appid, c.score * 100.0))
        .collect();

    match pick_from_list(
        reader,
        writer,
        "AutoGSE - Steam App ID Disambiguation",
        &options,
        Some("Enter Custom Steam App ID manually"),
    ) {
        PickResult::Selected(i) => candidates
            .get(i)
            .map(|c| (c.appid, Some(c.name.clone())))
            .ok_or_else(|| AutoGseError::AppIdResolutionFailed("selection out of range".to_string())),
        PickResult::Manual(value) => value
            .trim()
            .parse::<u64>()
            .map(|id| (id, None))
            .map_err(|_| AutoGseError::AppIdResolutionFailed(format!("'{value}' is not a valid numeric Steam App ID"))),
        PickResult::Cancelled => Err(AutoGseError::AppIdResolutionFailed(
            "no Steam App ID selected (prompt cancelled or non-interactive)".to_string(),
        )),
    }
}

pub fn prompt_app_id_disambiguation_stdio(
    target_dir: &Path,
    candidates: &[ScoredCandidate],
) -> Result<(u64, Option<String>), AutoGseError> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout();
    prompt_app_id_disambiguation(&mut stdin, &mut stdout, target_dir, candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn scored_candidates() -> Vec<ScoredCandidate> {
        vec![
            ScoredCandidate { appid: 1091500, name: "Cyberpunk 2077".to_string(), score: 0.95 },
            ScoredCandidate { appid: 999999, name: "Cyberpunk Knockoff".to_string(), score: 0.60 },
        ]
    }

    #[test]
    fn selects_top_candidate() {
        let mut input = Cursor::new(b"1\n".to_vec());
        let mut output = Vec::new();
        let result = prompt_app_id_disambiguation(&mut input, &mut output, &PathBuf::from("C:\\Games\\Foo"), &scored_candidates());
        assert_eq!(result.unwrap(), (1091500, Some("Cyberpunk 2077".to_string())));
    }

    #[test]
    fn manual_entry_returns_parsed_appid() {
        let mut input = Cursor::new(b"3\n1245620\n".to_vec());
        let mut output = Vec::new();
        let result = prompt_app_id_disambiguation(&mut input, &mut output, &PathBuf::from("C:\\Games\\Foo"), &scored_candidates());
        assert_eq!(result.unwrap(), (1245620, None));
    }

    #[test]
    fn non_numeric_manual_entry_is_an_error() {
        let mut input = Cursor::new(b"3\nnot-a-number\n".to_vec());
        let mut output = Vec::new();
        let result = prompt_app_id_disambiguation(&mut input, &mut output, &PathBuf::from("C:\\Games\\Foo"), &scored_candidates());
        assert!(matches!(result, Err(AutoGseError::AppIdResolutionFailed(_))));
    }

    #[test]
    fn eof_with_no_candidates_is_an_error() {
        let mut input = Cursor::new(Vec::new());
        let mut output = Vec::new();
        let result = prompt_app_id_disambiguation(&mut input, &mut output, &PathBuf::from("C:\\Games\\Foo"), &[]);
        assert!(matches!(result, Err(AutoGseError::AppIdResolutionFailed(_))));
    }

    #[test]
    fn empty_candidates_still_offers_manual_entry() {
        let mut input = Cursor::new(b"1\n1091500\n".to_vec());
        let mut output = Vec::new();
        let result = prompt_app_id_disambiguation(&mut input, &mut output, &PathBuf::from("C:\\Games\\Foo"), &[]);
        assert_eq!(result.unwrap(), (1091500, None));
    }

    #[test]
    fn renders_target_directory_in_output() {
        let mut input = Cursor::new(b"1\n".to_vec());
        let mut output = Vec::new();
        let _ = prompt_app_id_disambiguation(&mut input, &mut output, &PathBuf::from("C:\\Games\\Foo"), &scored_candidates());
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("C:\\Games\\Foo"));
        assert!(rendered.contains("Confidence: 95%"));
    }

    fn options() -> Vec<String> {
        vec!["Candidate One (AppID: 111)".to_string(), "Candidate Two (AppID: 222)".to_string()]
    }

    #[test]
    fn selects_numbered_option() {
        let mut input = Cursor::new(b"2\n".to_vec());
        let mut output = Vec::new();
        let result = pick_from_list(&mut input, &mut output, "Test", &options(), Some("Enter manually"));
        assert_eq!(result, PickResult::Selected(1));
    }

    #[test]
    fn manual_entry_branch() {
        let mut input = Cursor::new(b"3\n1245620\n".to_vec());
        let mut output = Vec::new();
        let result = pick_from_list(&mut input, &mut output, "Test", &options(), Some("Enter manually"));
        assert_eq!(result, PickResult::Manual("1245620".to_string()));
    }

    #[test]
    fn eof_is_cancelled() {
        let mut input = Cursor::new(Vec::new());
        let mut output = Vec::new();
        let result = pick_from_list(&mut input, &mut output, "Test", &options(), Some("Enter manually"));
        assert_eq!(result, PickResult::Cancelled);
    }

    #[test]
    fn out_of_range_is_cancelled() {
        let mut input = Cursor::new(b"99\n".to_vec());
        let mut output = Vec::new();
        let result = pick_from_list(&mut input, &mut output, "Test", &options(), None);
        assert_eq!(result, PickResult::Cancelled);
    }

    #[test]
    fn empty_manual_entry_is_cancelled() {
        let mut input = Cursor::new(b"3\n\n".to_vec());
        let mut output = Vec::new();
        let result = pick_from_list(&mut input, &mut output, "Test", &options(), Some("Enter manually"));
        assert_eq!(result, PickResult::Cancelled);
    }

    #[test]
    fn garbage_input_is_cancelled() {
        let mut input = Cursor::new(b"not-a-number\n".to_vec());
        let mut output = Vec::new();
        let result = pick_from_list(&mut input, &mut output, "Test", &options(), None);
        assert_eq!(result, PickResult::Cancelled);
    }
}
