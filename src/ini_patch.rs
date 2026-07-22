use std::path::Path;

use crate::error::AutoGseError;

/// Minimal read-modify-write editor for the `[section]` / `key=value` INI
/// format every vendored `configs.*.ini` file uses (confirmed by direct
/// inspection — CRLF line endings, `#`-prefixed comments, no quoting). A full
/// INI crate isn't worth the extra dependency weight for this: the format is
/// flat and the only operation ever needed is "set this one key under this
/// one section," never full structural parsing.
pub fn set_key(path: &Path, section: &str, key: &str, value: &str) -> Result<(), AutoGseError> {
    let content = std::fs::read_to_string(path)?;
    let updated = set_key_in_str(&content, section, key, value);
    std::fs::write(path, updated)?;
    Ok(())
}

/// Commented-out example lines (e.g. `#ticket=...`) must never be mistaken
/// for the real key — only a line that isn't `#`/`;`-prefixed counts.
fn set_key_in_str(content: &str, section: &str, key: &str, value: &str) -> String {
    let section_header = format!("[{section}]");
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();

    let Some(section_start) = lines.iter().position(|l| l.trim() == section_header) else {
        if !lines.is_empty() && !lines.last().unwrap().trim().is_empty() {
            lines.push(String::new());
        }
        lines.push(section_header);
        lines.push(format!("{key}={value}"));
        return lines.join("\r\n") + "\r\n";
    };

    let mut i = section_start + 1;
    while i < lines.len() && !lines[i].trim_start().starts_with('[') {
        let trimmed = lines[i].trim_start();
        if !trimmed.starts_with('#') && !trimmed.starts_with(';') {
            if let Some((existing_key, _)) = trimmed.split_once('=') {
                if existing_key.trim() == key {
                    lines[i] = format!("{key}={value}");
                    return lines.join("\r\n") + "\r\n";
                }
            }
        }
        i += 1;
    }

    lines.insert(section_start + 1, format!("{key}={value}"));
    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_temp(content: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("configs.user.ini");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn replaces_existing_key_value() {
        let (_dir, path) = write_temp("[user::general]\r\naccount_name=gse_user\r\n");
        set_key(&path, "user::general", "account_name", "jayeff89").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("account_name=jayeff89"));
        assert!(!result.contains("gse_user"));
    }

    #[test]
    fn does_not_confuse_commented_example_line_with_real_key() {
        let (_dir, path) = write_temp("[user::general]\r\n#ticket=examplebase64\r\nlanguage=english\r\n");
        set_key(&path, "user::general", "ticket", "realvalue").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        // The commented example must be left untouched, and the real key
        // appended fresh rather than uncommenting the example line.
        assert!(result.contains("#ticket=examplebase64"));
        assert!(result.contains("ticket=realvalue"));
    }

    #[test]
    fn appends_key_to_existing_section_when_missing() {
        let (_dir, path) = write_temp("[user::general]\r\naccount_name=gse_user\r\n\r\n[user::saves]\r\nsaves_folder_name=GSE Saves\r\n");
        set_key(&path, "user::general", "language", "german").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("language=german"));
        // Must land inside [user::general], before [user::saves] starts.
        let general_pos = result.find("[user::general]").unwrap();
        let saves_pos = result.find("[user::saves]").unwrap();
        let lang_pos = result.find("language=german").unwrap();
        assert!(general_pos < lang_pos && lang_pos < saves_pos);
    }

    #[test]
    fn creates_missing_section_and_key() {
        let (_dir, path) = write_temp("[other::section]\r\nfoo=bar\r\n");
        set_key(&path, "user::general", "account_name", "jayeff89").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("[user::general]"));
        assert!(result.contains("account_name=jayeff89"));
        // Original section/content untouched.
        assert!(result.contains("[other::section]"));
        assert!(result.contains("foo=bar"));
    }

    #[test]
    fn preserves_unrelated_lines_verbatim() {
        let (_dir, path) = write_temp("# a comment\r\n\r\n[user::general]\r\naccount_name=gse_user\r\nip_country=US\r\n");
        set_key(&path, "user::general", "account_name", "jayeff89").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("# a comment"));
        assert!(result.contains("ip_country=US"));
    }
}
