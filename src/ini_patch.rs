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

/// Removes one `key`'s line from `section`, if present. A no-op (not an
/// error) when the section or key doesn't exist — callers that just want
/// "make sure this key is gone" (e.g. Phase 7 §7.4's DLC checkbox toggling)
/// shouldn't have to check existence first.
pub fn remove_key(path: &Path, section: &str, key: &str) -> Result<(), AutoGseError> {
    let content = std::fs::read_to_string(path)?;
    let updated = remove_key_in_str(&content, section, key);
    std::fs::write(path, updated)?;
    Ok(())
}

fn remove_key_in_str(content: &str, section: &str, key: &str) -> String {
    let section_header = format!("[{section}]");
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();

    if let Some(section_start) = lines.iter().position(|l| l.trim() == section_header) {
        let mut i = section_start + 1;
        while i < lines.len() && !lines[i].trim_start().starts_with('[') {
            let trimmed = lines[i].trim_start();
            if !trimmed.starts_with('#') && !trimmed.starts_with(';') {
                if let Some((existing_key, _)) = trimmed.split_once('=') {
                    if existing_key.trim() == key {
                        lines.remove(i);
                        break;
                    }
                }
            }
            i += 1;
        }
    }

    lines.join("\r\n") + "\r\n"
}

/// One `key=value` pair read by [`read_all`], in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniEntry {
    pub key: String,
    pub value: String,
}

/// One `[section]` and its entries, in file order — comments/blank lines are
/// dropped, same "only real keys matter" rule `set_key`/`remove_key` apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniSection {
    pub name: String,
    pub entries: Vec<IniEntry>,
}

/// Phase 7 §7.4's read side: enumerates every section and key in `path`, for
/// a generic tabbed viewer that doesn't want to hardcode every key Phase 6
/// §6.1-§6.4 already introduced as CLI flags. `set_key`/`remove_key` remain
/// the only write path — this never needs to write anything back verbatim,
/// so it doesn't attempt to preserve comments/formatting at all (unlike
/// those two, which edit in place and must not disturb unrelated lines).
pub fn read_all(path: &Path) -> Result<Vec<IniSection>, AutoGseError> {
    let content = std::fs::read_to_string(path)?;
    Ok(read_all_from_str(&content))
}

fn read_all_from_str(content: &str) -> Vec<IniSection> {
    let mut sections = Vec::new();
    let mut current: Option<IniSection> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(IniSection { name: name.to_string(), entries: Vec::new() });
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            if let Some(section) = current.as_mut() {
                section.entries.push(IniEntry { key: key.trim().to_string(), value: value.trim().to_string() });
            }
        }
    }
    if let Some(section) = current.take() {
        sections.push(section);
    }

    sections
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

    #[test]
    fn remove_key_deletes_only_the_matching_line() {
        let (_dir, path) = write_temp("[app::dlcs]\r\nunlock_all=0\r\n304140=Brazilian Paint Jobs Pack\r\n1704460=Volvo Construction Equipment\r\n");
        remove_key(&path, "app::dlcs", "304140").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(!result.contains("304140"));
        assert!(result.contains("1704460=Volvo Construction Equipment"));
        assert!(result.contains("unlock_all=0"));
    }

    #[test]
    fn remove_key_of_missing_key_is_a_noop_not_an_error() {
        let (_dir, path) = write_temp("[app::dlcs]\r\nunlock_all=0\r\n");
        remove_key(&path, "app::dlcs", "999999").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("unlock_all=0"));
    }

    #[test]
    fn remove_key_never_touches_a_commented_example_line() {
        let (_dir, path) = write_temp("[app::dlcs]\r\n#1234=DLCNAME\r\n1234=RealName\r\n");
        remove_key(&path, "app::dlcs", "1234").unwrap();
        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("#1234=DLCNAME"));
        assert!(!result.contains("1234=RealName"));
    }

    #[test]
    fn read_all_parses_sections_and_entries_in_order() {
        let (_dir, path) = write_temp(
            "[user::general]\r\naccount_name=gse_user\r\nlanguage=english\r\n\r\n[app::dlcs]\r\nunlock_all=0\r\n304140=Brazilian Paint Jobs Pack\r\n",
        );
        let sections = read_all(&path).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "user::general");
        assert_eq!(sections[0].entries, vec![
            IniEntry { key: "account_name".to_string(), value: "gse_user".to_string() },
            IniEntry { key: "language".to_string(), value: "english".to_string() },
        ]);
        assert_eq!(sections[1].name, "app::dlcs");
        assert_eq!(sections[1].entries[1], IniEntry { key: "304140".to_string(), value: "Brazilian Paint Jobs Pack".to_string() });
    }

    #[test]
    fn read_all_skips_comments_and_blank_lines() {
        let (_dir, path) = write_temp("[user::general]\r\n# a comment\r\n\r\n#ticket=example\r\naccount_name=gse_user\r\n");
        let sections = read_all(&path).unwrap();
        assert_eq!(sections[0].entries, vec![IniEntry { key: "account_name".to_string(), value: "gse_user".to_string() }]);
    }

    #[test]
    fn read_all_of_empty_file_is_empty() {
        let (_dir, path) = write_temp("");
        assert!(read_all(&path).unwrap().is_empty());
    }
}
