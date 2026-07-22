use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::credentials;
use crate::error::AutoGseError;

const LOG_FILENAME: &str = "autogse.log";

/// Simplest capping approach that needs no log-rotation library: if the file
/// exceeds this size at the moment a new line is about to be appended,
/// truncate to its newest half first (Phase 6 §6.9 — this is what several of
/// Phases 3/5's empirical failures were harder to diagnose after the fact
/// without, since console/toast output disappears once the window closes).
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

fn log_path(dir: &Path) -> PathBuf {
    dir.join(LOG_FILENAME)
}

/// Appends one line (a timestamp is not added here — callers already know
/// their own context; keep this primitive dumb and easy to test).
pub fn append(line: &str) -> Result<(), AutoGseError> {
    append_in(&credentials::store_dir()?, line)
}

fn append_in(dir: &Path, line: &str) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dir)?;
    let path = log_path(dir);
    cap_if_needed(&path)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn cap_if_needed(path: &Path) -> Result<(), AutoGseError> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() <= MAX_LOG_BYTES {
        return Ok(());
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let keep_from = lines.len() / 2;
    let trimmed = if lines[keep_from..].is_empty() { String::new() } else { lines[keep_from..].join("\n") + "\n" };
    std::fs::write(path, trimmed)?;
    Ok(())
}

/// The last `n` lines, for `doctor`'s report. Empty if there's no log yet.
pub fn tail(n: usize) -> Result<Vec<String>, AutoGseError> {
    tail_in(&credentials::store_dir()?, n)
}

fn tail_in(dir: &Path, n: usize) -> Result<Vec<String>, AutoGseError> {
    let path = log_path(dir);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_then_tail_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        append_in(dir.path(), "first").unwrap();
        append_in(dir.path(), "second").unwrap();

        assert_eq!(tail_in(dir.path(), 10).unwrap(), vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn tail_returns_empty_when_no_log_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(tail_in(dir.path(), 10).unwrap().is_empty());
    }

    #[test]
    fn tail_returns_only_the_last_n_lines() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            append_in(dir.path(), &format!("line {i}")).unwrap();
        }
        assert_eq!(tail_in(dir.path(), 2).unwrap(), vec!["line 3".to_string(), "line 4".to_string()]);
    }

    #[test]
    fn cap_if_needed_truncates_to_newest_half_when_oversized() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_path(dir.path());
        // Build a file well past MAX_LOG_BYTES out of many short lines.
        let line = "x".repeat(100);
        let mut content = String::new();
        while (content.len() as u64) <= MAX_LOG_BYTES {
            content.push_str(&line);
            content.push('\n');
        }
        std::fs::write(&path, &content).unwrap();
        let lines_before = content.lines().count();

        cap_if_needed(&path).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        let lines_after = after.lines().count();
        assert!(lines_after < lines_before, "oversized log must be truncated");
        assert!((after.len() as u64) < MAX_LOG_BYTES);
    }

    #[test]
    fn cap_if_needed_is_a_noop_under_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_path(dir.path());
        std::fs::write(&path, "small log\n").unwrap();

        cap_if_needed(&path).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "small log\n");
    }
}
